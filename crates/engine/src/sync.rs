//! Live sync (M4.2): the server's command journal and deterministic replay.
//!
//! The server is authoritative. With a journal enabled ([`Session::enable_journal`]) every
//! local command that changes the workspace is appended as a [`proto::Entry`] — command,
//! params, actor, the pinned clock `at`, and the id counter before it. A client replays entries
//! with [`Session::apply_remote`]: the command runs as `actor` with `now()` pinned to `at`, and
//! because ids come from the workspace's own counter, the same workspace plus the same entry
//! gives the same workspace. The counter and the revision sequence are checked on every replay,
//! so a client that has drifted finds out on the next entry and resyncs from a snapshot.
//!
//! Undo/redo are local-only ([`crate::LOCAL_ONLY`]): a server never journals them and a replay
//! refuses them. Per-user undo through the server (inverse operations) is later work.

use std::collections::{BTreeMap, VecDeque};

use serde_json::Value;
use trackcraft_model::{Timestamp, Workspace};
pub use trackcraft_sync_proto as proto;

use crate::authz::{Delivery, Scope};
use crate::{Mode, Session};

use proto::{Entry, Origin};

/// Default journal bounds: entries kept, and their approximate total size.
pub const JOURNAL_MAX_ENTRIES: usize = 10_000;
pub const JOURNAL_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Clients whose last applied `seq` is remembered (duplicate suppression).
pub const MAX_TRACKED_CLIENTS: usize = 10_000;

/// The workspace id counter (the next id is one more). Part of every entry as a cheap
/// determinism check.
pub fn ids_next(ws: &Workspace) -> u64 {
    serde_json::to_value(&ws.ids).ok().and_then(|v| v.get("next").and_then(Value::as_u64)).unwrap_or(0)
}

/// The placeholder a sync stream sends instead of an entry the user may not see (M4.3,
/// `docs/permissions.md`): it carries only `rev`, `at`, `idsBefore` and `params.idsAfter`.
/// [`Session::apply_remote`] advances the revision and the id counter and changes nothing else.
pub const SKIP_COMMAND: &str = "sync.skip";

/// What the journal keeps beside each entry for per-user delivery (never sent).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// What the command touched, captured before it ran.
    pub scope: Scope,
    /// The id counter after it (a skip placeholder's `idsAfter`).
    pub ids_after: u64,
    /// The inbox's notification counter after it (a skip placeholder's `inboxAfter`): a hidden
    /// change can notify someone, and filtered copies must number later notifications alike.
    pub inbox_after: u64,
}

/// The `sync.skip` placeholder for `e`.
pub fn skip_entry(e: &Entry, meta: &Meta) -> Entry {
    Entry {
        rev: e.rev,
        command: SKIP_COMMAND.to_string(),
        params: serde_json::json!({"idsAfter": meta.ids_after, "inboxAfter": meta.inbox_after}),
        actor: String::new(),
        at: e.at,
        ids_before: e.ids_before,
        origin: None,
    }
}

/// A bounded ring of applied entries.
#[derive(Debug, Clone)]
pub struct Journal {
    entries: VecDeque<(Entry, usize, Meta)>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    /// `since` values below this can't be served (entries after it were dropped from the ring).
    base: u64,
    clients: BTreeMap<String, u64>,
}

impl Journal {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Journal { entries: VecDeque::new(), bytes: 0, max_entries: max_entries.max(1), max_bytes, base: 0, clients: BTreeMap::new() }
    }

    fn push(&mut self, e: Entry, meta: Meta) {
        let size = serde_json::to_vec(&e.params).map(|v| v.len()).unwrap_or(0).saturating_add(e.command.len()).saturating_add(128);
        if let Some(o) = &e.origin {
            if !self.clients.contains_key(&o.client)
                && self.clients.len() >= MAX_TRACKED_CLIENTS
                && let Some(k) = self.clients.keys().next().cloned()
            {
                self.clients.remove(&k);
            }
            self.clients.insert(o.client.clone(), o.seq);
        }
        self.entries.push_back((e, size, meta));
        self.bytes = self.bytes.saturating_add(size);
        while self.entries.len() > self.max_entries || (self.bytes > self.max_bytes && self.entries.len() > 1) {
            match self.entries.pop_front() {
                Some((old, s, _)) => {
                    self.base = old.rev;
                    self.bytes = self.bytes.saturating_sub(s);
                }
                None => break,
            }
        }
    }

    /// The oldest `since` that can be streamed from.
    pub fn oldest(&self) -> u64 {
        self.base
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn last(&self) -> Option<&Entry> {
        self.entries.back().map(|(e, _, _)| e)
    }

    /// The highest `seq` applied for `client`.
    pub fn applied_seq(&self, client: &str) -> Option<u64> {
        self.clients.get(client).copied()
    }
}

/// Why entries can't be streamed from a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinceError {
    /// The journal no longer holds entries that far back (or this session keeps none).
    TooOld { oldest: u64 },
    /// `since` is past the current revision (another epoch, or a hostile request).
    Future { rev: u64 },
}

/// Why a replay failed. Any of these means the client has diverged: resync from a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// The entry isn't the next revision.
    Gap { expected: u64, got: u64 },
    /// The id counter differs from the server's before this command.
    Diverged { ours: u64, theirs: u64 },
    /// The command failed or changed nothing here, while it changed the server's copy.
    Failed(String),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Gap { expected, got } => write!(f, "expected revision {expected}, got {got}"),
            ReplayError::Diverged { ours, theirs } => write!(f, "workspace diverged (id counter {ours}, server {theirs})"),
            ReplayError::Failed(m) => write!(f, "replay failed: {m}"),
        }
    }
}
impl std::error::Error for ReplayError {}

/// Sync state of a session: its synced revision, the journal (servers) and the origin stamped
/// on entries recorded during a request.
#[derive(Debug, Default)]
pub(crate) struct SyncState {
    rev: u64,
    journal: Option<Journal>,
    origin: Option<Origin>,
}

impl SyncState {
    /// A local change: with a journal, it becomes the next revision.
    pub(crate) fn journaling(&self) -> bool {
        self.journal.is_some()
    }

    pub(crate) fn record(&mut self, id: &str, params: &Value, actor: &str, at: Timestamp, ids_before: u64, meta: Meta) {
        let Some(j) = self.journal.as_mut() else { return };
        self.rev = self.rev.saturating_add(1);
        j.push(
            Entry {
                rev: self.rev,
                command: id.to_string(),
                params: params.clone(),
                actor: actor.to_string(),
                at,
                ids_before,
                origin: self.origin.clone(),
            },
            meta,
        );
    }
}

impl Session {
    /// Starts journaling (servers): from now on every local change is the next sync revision.
    pub fn enable_journal(&mut self, journal: Journal) {
        let mut j = journal;
        j.base = self.sync.rev;
        self.sync.journal = Some(j);
    }

    pub fn journal(&self) -> Option<&Journal> {
        self.sync.journal.as_ref()
    }

    /// The synced revision: the journal head on a server, the last applied entry on a client.
    pub fn sync_rev(&self) -> u64 {
        self.sync.rev
    }

    /// The origin stamped on entries recorded until it is cleared (a server request from a
    /// sync client).
    pub fn set_origin(&mut self, origin: Option<Origin>) {
        self.sync.origin = origin;
    }

    /// Entries after `since`, at most `limit`.
    pub fn journal_since(&self, since: u64, limit: usize) -> Result<Vec<Entry>, SinceError> {
        Ok(self.journal_since_meta(since, limit)?.into_iter().map(|(e, _)| e).collect())
    }

    /// Entries after `since` with what each touched, at most `limit` (per-user streams).
    pub fn journal_since_meta(&self, since: u64, limit: usize) -> Result<Vec<(Entry, Meta)>, SinceError> {
        let j = self.sync.journal.as_ref().ok_or(SinceError::TooOld { oldest: self.sync.rev })?;
        if since > self.sync.rev {
            return Err(SinceError::Future { rev: self.sync.rev });
        }
        if since < j.base {
            return Err(SinceError::TooOld { oldest: j.base });
        }
        Ok(j.entries.iter().filter(|(e, _, _)| e.rev > since).take(limit).map(|(e, _, m)| (e.clone(), m.clone())).collect())
    }

    /// What `user` is told about an entry with `scope` ([`crate::authz::delivery`]).
    pub fn delivery(&self, user: &str, scope: &Scope) -> Delivery {
        crate::authz::delivery(&self.ws, user, scope)
    }

    /// Puts the workspace back and forgets journal entries after `rev` (a multi-command request
    /// that failed half-way). Their revisions are reused by the next change.
    pub fn rollback_to(&mut self, ws: Workspace, rev: u64) {
        self.ws = ws;
        if rev < self.sync.rev {
            if let Some(j) = self.sync.journal.as_mut() {
                while j.entries.back().is_some_and(|(e, _, _)| e.rev > rev) {
                    if let Some((_, s, _)) = j.entries.pop_back() {
                        j.bytes = j.bytes.saturating_sub(s);
                    }
                }
                j.base = j.base.min(rev);
            }
            self.sync.rev = rev;
        }
        self.bump();
    }

    /// Replaces the workspace with a server snapshot at `rev` (clients: first load and resync).
    /// Undo history is dropped; the session is not dirty (the server owns persistence).
    pub fn load_synced(&mut self, ws: Workspace, rev: u64) {
        self.ws = ws;
        self.sync.rev = rev;
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.bump();
    }

    /// Replaces the workspace keeping the sync revision (a client rebuilding its optimistic view).
    pub fn set_workspace(&mut self, ws: Workspace) {
        if ws != self.ws {
            self.ws = ws;
            self.bump();
        }
        self.undo.clear();
        self.redo.clear();
    }

    /// Runs `command` as `actor` with the clock pinned to `at`, without undo, journal or
    /// observer. Returns the result and whether the workspace changed. [`crate::LOCAL_ONLY`]
    /// commands are refused.
    pub fn replay(&mut self, command: &str, params: &Value, actor: &str, at: Timestamp) -> Result<(Value, bool), String> {
        if command == SKIP_COMMAND {
            return self.skip(params);
        }
        let prev_user = std::mem::replace(&mut self.user, actor.to_string());
        let prev_pin = self.pinned.replace(at);
        let r = self.exec(command, params, Mode::Replay);
        self.pinned = prev_pin;
        self.user = prev_user;
        r
    }

    /// A `sync.skip` placeholder: something this user can't see changed; only the id counter
    /// moves (to `params.idsAfter`, never backwards).
    fn skip(&mut self, params: &Value) -> Result<(Value, bool), String> {
        let after = params.get("idsAfter").and_then(Value::as_u64).ok_or("sync.skip needs \"idsAfter\"")?;
        if after < ids_next(&self.ws) {
            return Err("sync.skip would move the id counter backwards".into());
        }
        let ids = serde_json::from_value(serde_json::json!({"next": after})).map_err(|e| e.to_string())?;
        self.ws.ids = ids;
        // Optional (placeholders from before M4.5 lack it); never moves backwards.
        if let Some(n) = params.get("inboxAfter").and_then(Value::as_u64) {
            if n < self.ws.inbox.next_id {
                return Err("sync.skip would move the notification counter backwards".into());
            }
            self.ws.inbox.next_id = n;
        }
        self.bump();
        Ok((Value::Null, true))
    }

    /// Applies the server's next journal entry. On success the session is at `entry.rev`; any
    /// error means this copy has diverged (the workspace is unchanged) and the caller resyncs.
    pub fn apply_remote(&mut self, entry: &Entry) -> Result<Value, ReplayError> {
        let expected = self.sync.rev.saturating_add(1);
        if entry.rev != expected {
            return Err(ReplayError::Gap { expected, got: entry.rev });
        }
        let ours = ids_next(&self.ws);
        if ours != entry.ids_before {
            return Err(ReplayError::Diverged { ours, theirs: entry.ids_before });
        }
        let (v, changed) = self.replay(&entry.command, &entry.params, &entry.actor, entry.at).map_err(ReplayError::Failed)?;
        if !changed {
            return Err(ReplayError::Failed(format!("{} changed nothing here", entry.command.chars().take(80).collect::<String>())));
        }
        self.sync.rev = entry.rev;
        Ok(v)
    }
}
