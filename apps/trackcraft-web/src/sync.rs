//! Live sync for the browser (M4.6): the transport-free state machine. `docs/sync.md`, "Browser
//! transport".
//!
//! This is a port of `trackcraft-sync-client`'s `Remote` (the desktop client): the same
//! *confirmed copy + pending queue + replay* model over the same wire types
//! (`trackcraft-sync-proto`) and the same engine hooks (`Session::apply_remote`,
//! `Session::set_after_command`, `Session::replay`). What differs is the shape: the desktop
//! client owns two blocking threads, while a browser has one thread and asynchronous `fetch` /
//! `EventSource`. So here the network is driven *from outside*: [`Sync::step`] takes what the
//! browser reported ([`Net`]) and the current time, and answers with [`Action`]s for the browser
//! side (`sync_web.rs`) to carry out — fetch a snapshot, mint a stream ticket, open or close the
//! event stream, POST the next command. That keeps every decision (ordering, retries, backoff,
//! resync, rebase) in plain Rust, unit-tested natively with a fake transport.
//!
//! TODO(sync-core): this duplicates the rebase logic of `crates/sync-client/src/lib.rs` (owned
//! by another work stream this round). Extract a transport-agnostic core (this file's
//! `Net`/`Action` shape fits both) into `trackcraft-sync-client` and make the desktop threads a
//! driver of it, as this file's browser driver is.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use trackcraft_engine::{Executed, Session};
use trackcraft_sync_proto::{self as proto, CommandRequest, CommandResponse, Entry, Origin, Snapshot};

/// Largest snapshot accepted (bytes of JSON).
pub const MAX_SNAPSHOT: usize = 256 * 1024 * 1024;
/// Out-of-order entries held while waiting for the one before them.
const MAX_BUFFERED: usize = 4096;
/// Commands queued (not yet confirmed) before new local changes are refused.
pub const MAX_PENDING: usize = 10_000;
/// First retry delay and the longest one (seconds).
const MIN_BACKOFF: f64 = 0.25;
pub const MAX_BACKOFF: f64 = 10.0;

/// Connection state, for the title and the server panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    /// Event stream up and the last command (if any) delivered.
    Online,
    /// The server can't be reached; changes are queued.
    Offline { reason: String },
    /// Reloading the workspace from a snapshot.
    Resyncing,
    /// The server rejected the token; changes are queued.
    Unauthorized { reason: String },
}

/// Something worth a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    State(ConnState),
    /// The server refused queued commands; they were dropped. One line per command.
    Dropped(Vec<String>),
    /// The local copy diverged and is being reloaded.
    Resync(String),
}

impl Notice {
    pub fn message(&self, server: &str) -> String {
        match self {
            Notice::State(ConnState::Online) => format!("Connected to {server}"),
            Notice::State(ConnState::Offline { reason }) => format!("Offline ({reason}): changes are queued and sent on reconnect"),
            Notice::State(ConnState::Resyncing) => "Reloading the workspace from the server".into(),
            Notice::State(ConnState::Unauthorized { reason }) => format!("The server refused the token ({reason}); changes are queued"),
            Notice::Dropped(lines) => format!("The server refused {} change(s), which were dropped: {}", lines.len(), lines.join("; ")),
            Notice::Resync(why) => format!("Out of sync with the server ({why}); reloading. Your view is kept."),
        }
    }

    /// Whether the toast is an error.
    pub fn is_error(&self) -> bool {
        matches!(self, Notice::Dropped(_) | Notice::Resync(_) | Notice::State(ConnState::Unauthorized { .. }))
    }
}

/// A failed request: `status` 0 = no HTTP answer (network error, offline, unreadable body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub status: u16,
    pub message: String,
}

impl Failure {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Failure { status, message: message.into() }
    }

    /// Worth retrying: no answer, a timeout, rate limiting or a server error.
    fn transient(&self) -> bool {
        self.status == 0 || self.status == 408 || self.status == 429 || self.status >= 500
    }

    fn text(&self) -> String {
        let m: String = self.message.chars().take(120).collect();
        if self.status == 0 { m } else { format!("HTTP {}: {m}", self.status) }
    }
}

/// What the browser reports.
#[derive(Debug, Clone)]
pub enum Net {
    /// Answer to [`Action::FetchTicket`].
    Ticket(Result<String, Failure>),
    /// Answer to [`Action::FetchSnapshot`].
    Snapshot(Result<Box<Snapshot>, Failure>),
    /// An event from the stream [`Action::OpenStream`] opened (`hello`, `entry`, `resync`).
    Stream { event: String, data: String },
    /// That stream failed or ended (`EventSource` `error`).
    StreamError,
    /// Answer to [`Action::Post`].
    Posted { seq: u64, result: Result<CommandResponse, Failure> },
    /// `navigator.onLine` changed (`online` / `offline` events).
    Online(bool),
}

/// What the browser should do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `GET api/sync/snapshot?client=ID` → [`Net::Snapshot`].
    FetchSnapshot,
    /// `POST api/sync/ticket` → [`Net::Ticket`].
    FetchTicket,
    /// Close any open stream, then open `api/sync/events?{query}&ticket={ticket}`.
    OpenStream { query: String, ticket: String },
    /// Close the open stream (its later events are ignored).
    CloseStream,
    /// `POST api/command` with `body` (JSON) → [`Net::Posted`] with `seq`.
    Post { seq: u64, body: String },
}

/// What [`Sync::step`] decided.
#[derive(Debug, Default)]
pub struct Step {
    pub actions: Vec<Action>,
    pub notices: Vec<Notice>,
    /// Call `step` again within this many seconds (a retry timer), even with nothing new.
    pub wake_in: Option<f64>,
}

/// A local command sent but not yet confirmed.
#[derive(Debug, Clone)]
struct Pending {
    seq: u64,
    command: String,
    params: Value,
}

impl Pending {
    fn label(&self) -> String {
        let target = ["key", "id", "project", "space"].iter().find_map(|k| match self.params.get(k) {
            Some(Value::String(s)) => Some(s.chars().take(40).collect::<String>()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        });
        match target {
            Some(t) => format!("{} {t}", self.command),
            None => self.command.clone(),
        }
    }
}

/// Where the event stream is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Waiting until `at` (seconds), then fetching a snapshot (`snapshot`) or a ticket.
    Idle {
        at: f64,
        snapshot: bool,
    },
    Snapshot,
    Ticket,
    Open,
}

/// One [`Sync::step`] in progress.
struct Turn {
    now: f64,
    out: Step,
    /// `view` must be rebuilt from the confirmed copy + pending.
    rebuild: bool,
    /// Refused commands, for one toast.
    dropped: Vec<String>,
}

/// The sync side of a server workspace.
pub struct Sync {
    client: String,
    epoch: String,
    confirmed: Session,
    pending: VecDeque<Pending>,
    /// Pending commands not yet delivered, in order (the front is in flight when `inflight`).
    unsent: VecDeque<Pending>,
    inflight: Option<u64>,
    post_at: f64,
    post_backoff: f64,
    next_seq: u64,
    buffered: BTreeMap<u64, Entry>,
    awaiting_snapshot: bool,
    phase: Phase,
    stream_backoff: f64,
    outbox: Arc<Mutex<Vec<Executed>>>,
    events: Result<(), String>,
    commands: Result<(), String>,
    unauthorized: Option<String>,
    state: ConnState,
}

/// The workspace inside a snapshot.
pub fn workspace_of(snap: &Snapshot) -> Result<trackcraft_engine::model::Workspace, String> {
    trackcraft_store::from_str(&snap.workspace.to_string()).map_err(|e| format!("the server's workspace did not load: {e}"))
}

/// Checks a snapshot body from the server.
pub fn parse_snapshot(body: &str) -> Result<Snapshot, String> {
    if body.len() > MAX_SNAPSHOT {
        return Err(format!("the workspace is larger than {} MB", MAX_SNAPSHOT >> 20));
    }
    let snap: Snapshot = serde_json::from_str(body).map_err(|e| format!("bad snapshot: {e}"))?;
    if snap.protocol != proto::PROTOCOL {
        return Err(format!("the server speaks sync protocol {}, this page {}", snap.protocol, proto::PROTOCOL));
    }
    Ok(snap)
}

/// Percent-encodes a query or path component.
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The relative URL prefix of a workspace (`""` = the server's default, else `w/NAME/`).
pub fn workspace_base(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() { String::new() } else { format!("w/{}/", pct(name)) }
}

/// A token as typed: trimmed; refused when empty, too long or holding spaces/control characters
/// (it goes into a header).
pub fn clean_token(t: &str) -> Result<String, String> {
    let t = t.trim();
    if t.is_empty() {
        return Err("enter a token (trackcraft-server token create --user NAME)".into());
    }
    if t.len() > 1024 || t.chars().any(|c| c.is_whitespace() || c.is_control() || !c.is_ascii()) {
        return Err("the token contains spaces or characters a token can't have".into());
    }
    Ok(t.to_string())
}

fn backoff(d: f64) -> f64 {
    (d * 2.0).min(MAX_BACKOFF)
}

impl Sync {
    /// Starts from a first snapshot: the sync handle and the session the UI edits (remote mode,
    /// acting as the snapshot's user). The first [`Sync::step`] asks for a stream ticket.
    pub fn new(client: &str, snap: &Snapshot) -> Result<(Sync, Session), String> {
        if !Origin::valid_client(client) {
            return Err("bad client id".into());
        }
        let ws = workspace_of(snap)?;
        let mut confirmed = Session::new(ws.clone(), &snap.user);
        confirmed.load_synced(ws.clone(), snap.rev);
        confirmed.set_remote(true);
        let mut view = Session::new(ws.clone(), &snap.user);
        view.load_synced(ws, snap.rev);
        view.set_remote(true);
        let outbox: Arc<Mutex<Vec<Executed>>> = Arc::default();
        let sink = Arc::clone(&outbox);
        view.set_after_command(Some(Box::new(move |e| sink.lock().unwrap_or_else(|p| p.into_inner()).push(e.clone()))));
        let sync = Sync {
            client: client.to_string(),
            epoch: snap.epoch.clone(),
            confirmed,
            pending: VecDeque::new(),
            unsent: VecDeque::new(),
            inflight: None,
            post_at: 0.0,
            post_backoff: MIN_BACKOFF,
            next_seq: 1,
            buffered: BTreeMap::new(),
            awaiting_snapshot: false,
            phase: Phase::Idle { at: 0.0, snapshot: false },
            stream_backoff: MIN_BACKOFF,
            outbox,
            events: Err("connecting".into()),
            commands: Ok(()),
            unauthorized: None,
            state: ConnState::Offline { reason: "connecting".into() },
        };
        Ok((sync, view))
    }

    pub fn client(&self) -> &str {
        &self.client
    }

    #[cfg(test)]
    pub fn state(&self) -> &ConnState {
        &self.state
    }

    /// Local changes not yet confirmed by the server.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub fn rev(&self) -> u64 {
        self.confirmed.sync_rev()
    }

    #[cfg(test)]
    pub fn confirmed(&self) -> &Session {
        &self.confirmed
    }

    /// `Online`, `Offline · 3 unsent`, …
    pub fn status_text(&self) -> String {
        let base = match &self.state {
            ConnState::Online => "Online",
            ConnState::Offline { .. } => "Offline",
            ConnState::Resyncing => "Resyncing",
            ConnState::Unauthorized { .. } => "Token refused",
        };
        if self.pending.is_empty() { base.to_string() } else { format!("{base} · {} unsent", self.pending.len()) }
    }

    /// One turn: queue local changes, take in what the browser reported, decide what to do.
    pub fn step(&mut self, view: &mut Session, now: f64, nets: Vec<Net>) -> Step {
        let mut t = Turn { now, out: Step::default(), rebuild: false, dropped: Vec::new() };
        self.take_local(&mut t);
        for n in nets {
            match n {
                Net::Ticket(r) => self.on_ticket(&mut t, r),
                Net::Snapshot(r) => self.on_snapshot(view, &mut t, r),
                Net::Stream { event, data } => self.on_stream(view, &mut t, &event, &data),
                Net::StreamError => self.on_stream_error(&mut t),
                Net::Posted { seq, result } => self.on_posted(view, &mut t, seq, result),
                Net::Online(on) => self.on_online(&mut t, on),
            }
        }
        // A resync the stream or a replay asked for: drop the stream, fetch a snapshot.
        if self.awaiting_snapshot && !matches!(self.phase, Phase::Snapshot | Phase::Idle { snapshot: true, .. }) {
            if self.phase == Phase::Open {
                t.out.actions.push(Action::CloseStream);
            }
            self.phase = Phase::Snapshot;
            t.out.actions.push(Action::FetchSnapshot);
        }
        self.timers(&mut t);
        if t.rebuild {
            self.rebuild(view);
        }
        if !t.dropped.is_empty() {
            t.out.notices.push(Notice::Dropped(std::mem::take(&mut t.dropped)));
        }
        self.update_state(&mut t.out.notices);
        t.out
    }

    /// Local changes, in the order they ran, join the queue.
    fn take_local(&mut self, t: &mut Turn) {
        let local: Vec<Executed> = std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|p| p.into_inner()));
        for e in local {
            if self.pending.len() >= MAX_PENDING {
                t.dropped.push(format!("{} (too many unsent changes)", e.command));
                t.rebuild = true;
                continue;
            }
            let p = Pending { seq: self.next_seq, command: e.command, params: e.params };
            self.next_seq = self.next_seq.saturating_add(1);
            self.unsent.push_back(p.clone());
            self.pending.push_back(p);
        }
    }

    fn on_ticket(&mut self, t: &mut Turn, r: Result<String, Failure>) {
        if self.phase != Phase::Ticket {
            return;
        }
        match r {
            Ok(ticket) => {
                let query = format!("since={}&epoch={}&client={}", self.confirmed.sync_rev(), pct(&self.epoch), pct(&self.client));
                t.out.actions.push(Action::OpenStream { query, ticket });
                self.phase = Phase::Open;
            }
            Err(f) => self.stream_failed(t.now, f, false),
        }
    }

    fn on_snapshot(&mut self, view: &mut Session, t: &mut Turn, r: Result<Box<Snapshot>, Failure>) {
        if self.phase != Phase::Snapshot {
            return;
        }
        let snap = match r {
            Ok(s) => s,
            Err(f) => return self.stream_failed(t.now, f, true),
        };
        let ws = match workspace_of(&snap) {
            Ok(ws) => ws,
            Err(e) => {
                t.out.notices.push(Notice::Resync(e.clone()));
                return self.stream_failed(t.now, Failure::new(0, e), true);
            }
        };
        self.confirmed.load_synced(ws, snap.rev);
        self.epoch = snap.epoch.clone();
        self.awaiting_snapshot = false;
        if let Some(applied) = snap.applied_seq {
            self.pending.retain(|p| p.seq > applied);
            let inflight = self.inflight;
            self.unsent.retain(|p| p.seq > applied || Some(p.seq) == inflight);
        }
        let rev = snap.rev;
        self.buffered.retain(|r, _| *r > rev);
        t.rebuild = true;
        self.advance(view, &mut t.out.notices);
        self.phase = Phase::Ticket;
        t.out.actions.push(Action::FetchTicket);
    }

    fn on_stream(&mut self, view: &mut Session, t: &mut Turn, event: &str, data: &str) {
        if self.phase != Phase::Open {
            return;
        }
        match event {
            proto::EVENT_HELLO => {
                self.events = Ok(());
                self.unauthorized = None;
                self.stream_backoff = MIN_BACKOFF;
            }
            proto::EVENT_ENTRY => {
                let entry = (data.len() <= proto::MAX_EVENT_BYTES).then(|| serde_json::from_str::<Entry>(data).ok()).flatten();
                let Some(e) = entry else {
                    return self.request_resync(&mut t.out.notices, "the server sent an unreadable entry".into());
                };
                if self.awaiting_snapshot {
                    return;
                }
                let rev = e.rev;
                self.buffer(e);
                t.rebuild |= self.advance(view, &mut t.out.notices);
                // The stream is in order: if this entry still can't apply, revisions are missing.
                if !self.awaiting_snapshot && rev > self.confirmed.sync_rev() {
                    self.request_resync(&mut t.out.notices, format!("missing revisions before {rev}"));
                }
            }
            proto::EVENT_RESYNC => self.awaiting_snapshot = true,
            _ => {}
        }
    }

    fn on_stream_error(&mut self, t: &mut Turn) {
        if self.phase == Phase::Open {
            t.out.actions.push(Action::CloseStream);
            self.events = Err("the event stream was interrupted".into());
            self.phase = Phase::Idle { at: t.now + self.stream_backoff, snapshot: false };
            self.stream_backoff = backoff(self.stream_backoff);
        }
    }

    fn on_posted(&mut self, view: &mut Session, t: &mut Turn, seq: u64, result: Result<CommandResponse, Failure>) {
        if self.inflight != Some(seq) {
            return;
        }
        self.inflight = None;
        match result {
            Ok(resp) => {
                self.commands = Ok(());
                self.unauthorized = None;
                self.post_backoff = MIN_BACKOFF;
                self.unsent.retain(|p| p.seq != seq);
                if resp.duplicate {
                    return;
                }
                match resp.entry {
                    Some(e) => {
                        self.buffer(e);
                        if !self.awaiting_snapshot {
                            t.rebuild |= self.advance(view, &mut t.out.notices);
                        }
                    }
                    // Ran on the server but changed nothing there.
                    None => t.rebuild |= self.forget(seq).is_some(),
                }
            }
            Err(f) if f.status == 401 || f.transient() => {
                if f.status == 401 {
                    self.unauthorized = Some(f.text());
                } else {
                    self.commands = Err(f.text());
                }
                self.post_at = t.now + self.post_backoff;
                self.post_backoff = backoff(self.post_backoff);
            }
            Err(f) => {
                self.commands = Ok(());
                self.unsent.retain(|p| p.seq != seq);
                if let Some(p) = self.forget(seq) {
                    t.dropped.push(format!("{}: {}", p.label(), f.text()));
                    t.rebuild = true;
                }
            }
        }
    }

    fn on_online(&mut self, t: &mut Turn, online: bool) {
        if online {
            if let Phase::Idle { snapshot, .. } = self.phase {
                self.phase = Phase::Idle { at: t.now, snapshot };
            }
            self.stream_backoff = MIN_BACKOFF;
            self.post_backoff = MIN_BACKOFF;
            self.post_at = t.now;
            return;
        }
        self.events = Err("the browser is offline".into());
        if self.phase == Phase::Open {
            t.out.actions.push(Action::CloseStream);
        }
        // `navigator.onLine` can be wrong: keep trying, slowly.
        if matches!(self.phase, Phase::Open | Phase::Idle { .. }) {
            self.phase = Phase::Idle { at: t.now + MAX_BACKOFF, snapshot: self.awaiting_snapshot };
        }
    }

    /// Due retries: reconnect the stream, POST the next command; else when to look again.
    fn timers(&mut self, t: &mut Turn) {
        let now = t.now;
        let mut wake: Option<f64> = None;
        let mut wake_at = |at: f64| {
            let d = (at - now).max(0.0);
            wake = Some(wake.map_or(d, |w: f64| w.min(d)));
        };
        if let Phase::Idle { at, snapshot } = self.phase {
            if now >= at {
                self.phase = if snapshot { Phase::Snapshot } else { Phase::Ticket };
                t.out.actions.push(if snapshot { Action::FetchSnapshot } else { Action::FetchTicket });
            } else {
                wake_at(at);
            }
        }
        if self.inflight.is_none()
            && let Some(p) = self.unsent.front()
        {
            if now < self.post_at {
                wake_at(self.post_at);
            } else {
                let body = CommandRequest {
                    command: p.command.clone(),
                    params: p.params.clone(),
                    origin: Some(Origin { client: self.client.clone(), seq: p.seq }),
                };
                let seq = p.seq;
                match serde_json::to_string(&body) {
                    Ok(body) => {
                        self.inflight = Some(seq);
                        t.out.actions.push(Action::Post { seq, body });
                    }
                    Err(e) => {
                        self.unsent.pop_front();
                        if let Some(p) = self.forget(seq) {
                            t.dropped.push(format!("{}: {e}", p.label()));
                            t.rebuild = true;
                        }
                    }
                }
            }
        }
        t.out.wake_in = wake;
    }

    fn update_state(&mut self, notices: &mut Vec<Notice>) {
        let state = match (&self.unauthorized, &self.events, &self.commands) {
            (Some(m), _, _) => ConnState::Unauthorized { reason: m.clone() },
            _ if self.awaiting_snapshot => ConnState::Resyncing,
            (None, Err(e), _) | (None, Ok(()), Err(e)) => ConnState::Offline { reason: e.chars().take(120).collect() },
            (None, Ok(()), Ok(())) => ConnState::Online,
        };
        if state != self.state {
            // "connecting" → online is not news; offline → offline (another reason) neither.
            let first = matches!(&self.state, ConnState::Offline { reason } if reason == "connecting");
            if !(first && state == ConnState::Online)
                && !(matches!(state, ConnState::Offline { .. }) && matches!(self.state, ConnState::Offline { .. }))
            {
                notices.push(Notice::State(state.clone()));
            }
            self.state = state;
        }
    }

    /// A ticket or snapshot request failed: back off, then try again.
    fn stream_failed(&mut self, now: f64, f: Failure, snapshot: bool) {
        if f.status == 401 {
            self.unauthorized = Some(f.text());
        } else {
            self.events = Err(f.text());
        }
        self.phase = Phase::Idle { at: now + self.stream_backoff, snapshot };
        self.stream_backoff = backoff(self.stream_backoff);
    }

    fn buffer(&mut self, e: Entry) {
        if e.rev > self.confirmed.sync_rev() && self.buffered.len() < MAX_BUFFERED {
            self.buffered.insert(e.rev, e);
        }
    }

    fn forget(&mut self, seq: u64) -> Option<Pending> {
        let i = self.pending.iter().position(|p| p.seq == seq)?;
        self.pending.remove(i)
    }

    fn request_resync(&mut self, notices: &mut Vec<Notice>, why: String) {
        if !self.awaiting_snapshot {
            notices.push(Notice::Resync(why));
        }
        self.buffered.clear();
        self.awaiting_snapshot = true;
    }

    /// Applies buffered entries in revision order. Returns whether `view` must be rebuilt.
    fn advance(&mut self, view: &mut Session, notices: &mut Vec<Notice>) -> bool {
        let mut rebuild = false;
        let next = |s: &Session| s.sync_rev().saturating_add(1);
        while let Some(e) = self.buffered.remove(&next(&self.confirmed)) {
            if let Err(err) = self.confirmed.apply_remote(&e) {
                self.request_resync(notices, err.to_string());
                return false;
            }
            let ours = e.origin.as_ref().filter(|o| o.client == self.client).map(|o| o.seq);
            match ours {
                Some(seq) => {
                    let was_first = self.pending.front().is_some_and(|p| p.seq == seq);
                    self.forget(seq);
                    // Our own echo: `view` already shows it unless the order changed. Once the
                    // queue drains, rebuild so `view` is exactly the server's (its timestamps).
                    if !was_first || self.pending.is_empty() {
                        rebuild = true;
                    }
                }
                None if self.pending.is_empty() && !rebuild => {
                    // Nobody's local change in the way: replay straight onto the view.
                    if !matches!(view.replay(&e.command, &e.params, &e.actor, e.at), Ok((_, true))) {
                        rebuild = true;
                    }
                }
                None => rebuild = true,
            }
        }
        let rev = self.confirmed.sync_rev();
        self.buffered.retain(|r, _| *r > rev);
        rebuild
    }

    /// `view` = confirmed + pending (each pending command re-run on top, as its user).
    fn rebuild(&mut self, view: &mut Session) {
        view.set_workspace(self.confirmed.ws.clone());
        let (user, now) = (view.user.clone(), view.now());
        for p in &self.pending {
            // A pending command that no longer applies stays queued: the server decides.
            let _ = view.replay(&p.command, &p.params, &user, now);
        }
    }
}

#[cfg(test)]
mod tests;
