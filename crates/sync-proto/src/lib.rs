//! TrackCraft live-sync protocol (L1): the wire types the server, the engine's replay and the
//! desktop client share, plus a Server-Sent Events line parser. Serde only — no I/O, no runtime.
//!
//! Nothing here knows about issues or pages: an [`Entry`] is "command `command` with `params`
//! ran as `actor` at `at`", so the same types can move into a shared `craft-collab` crate for
//! Chatcraft (see `docs/sync.md`). The one app-defined field is [`Entry::ids_before`], a
//! determinism check the app chooses (TrackCraft: the workspace id counter before the command).
//!
//! Endpoints (relative to a workspace base, e.g. `/w/team`):
//! - `GET  /api/sync/snapshot[?client=ID]` → [`Snapshot`];
//! - `GET  /api/sync/events?since=REV&epoch=E[&client=ID]` → SSE: [`EVENT_HELLO`] ([`Hello`]),
//!   then [`EVENT_ENTRY`] ([`Entry`], `id:` = rev) per change, or [`EVENT_RESYNC`] ([`Resync`]);
//! - `POST /api/command` with [`CommandRequest`] → [`CommandResponse`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol version, reported in [`Hello`] and [`Snapshot`].
pub const PROTOCOL: u32 = 1;

/// SSE event names.
pub const EVENT_HELLO: &str = "hello";
pub const EVENT_ENTRY: &str = "entry";
pub const EVENT_RESYNC: &str = "resync";

/// Largest SSE event (`data:` bytes) a client accepts; a bigger one is a protocol error.
pub const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;

/// Longest client id.
pub const MAX_CLIENT_ID: usize = 64;

/// Who sent a command: a client instance (random id per process) and its own sequence number,
/// which rises by one per command. The server keeps the highest applied `seq` per client, so a
/// command resent after a lost response is not applied twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub client: String,
    pub seq: u64,
}

impl Origin {
    /// Client ids are 1–64 of `A-Z a-z 0-9 - _` (they go into query strings and logs).
    pub fn valid_client(id: &str) -> bool {
        !id.is_empty() && id.len() <= MAX_CLIENT_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
}

/// One applied mutating command in a workspace's journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// The workspace revision this command produced (the first change after start is 1).
    pub rev: u64,
    pub command: String,
    #[serde(default)]
    pub params: Value,
    /// The user it ran as.
    pub actor: String,
    /// The clock every `now()` inside the command saw.
    pub at: DateTime<Utc>,
    /// App-defined determinism check taken before the command (TrackCraft: the id counter).
    #[serde(default)]
    pub ids_before: u64,
    /// Set when a sync client sent the command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// First event of every stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    /// Changes whenever the server's journal starts over (a restart): revisions from another
    /// epoch mean nothing, so the client resyncs.
    pub epoch: String,
    /// The current revision.
    pub rev: u64,
    /// The oldest revision the journal can still stream from (`since >= oldest`).
    pub oldest: u64,
}

/// Sent instead of entries when `since` can't be served (too old, wrong epoch, in the future);
/// the stream ends after it and the client fetches a [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resync {
    pub epoch: String,
    pub rev: u64,
    pub reason: String,
}

/// The whole workspace at a revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub protocol: u32,
    pub epoch: String,
    pub rev: u64,
    /// The authenticated user (commands from this client run as them).
    pub user: String,
    /// With `?client=ID`: the highest `seq` of that client the snapshot already contains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_seq: Option<u64>,
    /// The workspace in the app's file format (TrackCraft: the `.track` JSON envelope).
    pub workspace: Value,
}

/// `POST /api/command` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    pub command: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// `POST /api/command` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResponse {
    pub command: String,
    #[serde(default)]
    pub result: Value,
    /// The workspace revision after the command.
    #[serde(default)]
    pub rev: u64,
    /// The journal entry it produced (`None` for queries and commands that changed nothing).
    #[serde(default)]
    pub entry: Option<Entry>,
    /// The origin's `seq` had already been applied; nothing ran.
    #[serde(default)]
    pub duplicate: bool,
}

/// One dispatched SSE event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` (empty = the default `message`).
    pub event: String,
    pub data: String,
    pub id: Option<String>,
    /// The data passed [`MAX_EVENT_BYTES`] and was cut: treat as a protocol error.
    pub truncated: bool,
}

/// Incremental SSE parser (the WHATWG event-stream rules that matter here): feed it one line at
/// a time without the line ending; a blank line dispatches the event collected so far. Comment
/// lines (`:` keep-alives) and unknown fields are ignored.
#[derive(Debug, Default)]
pub struct SseParser {
    cur: SseEvent,
    has_data: bool,
}

impl SseParser {
    pub fn new() -> Self {
        SseParser::default()
    }

    pub fn feed_line(&mut self, line: &str) -> Option<SseEvent> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if !self.has_data && self.cur.event.is_empty() {
                self.cur = SseEvent::default();
                return None;
            }
            self.has_data = false;
            return Some(std::mem::take(&mut self.cur));
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.cur.event = value.chars().take(64).collect(),
            "id" => self.cur.id = Some(value.chars().take(64).collect()),
            "data" => {
                let extra = value.len().saturating_add(usize::from(self.has_data));
                if self.cur.truncated || self.cur.data.len().saturating_add(extra) > MAX_EVENT_BYTES {
                    self.cur.truncated = true;
                } else {
                    if self.has_data {
                        self.cur.data.push('\n');
                    }
                    self.cur.data.push_str(value);
                }
                self.has_data = true;
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parsing() {
        let mut p = SseParser::new();
        let mut out = vec![];
        for l in [": keep-alive", "", "event: entry", "id: 3", "data: {\"a\":", "data:1}", "", "data", "", "retry: 5", "bogus", ""] {
            if let Some(e) = p.feed_line(l) {
                out.push(e);
            }
        }
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].event.as_str(), out[0].id.as_deref(), out[0].data.as_str()), ("entry", Some("3"), "{\"a\":\n1}"));
        assert_eq!(out[1].data, "");
        let big = "x".repeat(MAX_EVENT_BYTES);
        assert!(p.feed_line(&format!("data: {big}")).is_none());
        assert!(p.feed_line("data: y").is_none());
        let e = p.feed_line("").unwrap();
        assert!(e.truncated);
    }

    #[test]
    fn entries_round_trip() {
        let e = Entry {
            rev: 7,
            command: "issue.edit".into(),
            params: serde_json::json!({"key": "A-1"}),
            actor: "riley".into(),
            at: DateTime::from_timestamp(1_793_448_000, 123_456_789).unwrap(),
            ids_before: 41,
            origin: Some(Origin { client: "c1".into(), seq: 2 }),
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("idsBefore"));
        assert_eq!(serde_json::from_str::<Entry>(&s).unwrap(), e);
        assert!(Origin::valid_client("abc-DEF_09"));
        for bad in ["", "a b", "é", &"a".repeat(65)] {
            assert!(!Origin::valid_client(bad));
        }
    }
}
