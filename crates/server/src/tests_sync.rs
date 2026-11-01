//! M4.2 live sync, in process: snapshot, command journal, SSE event stream, resync, replay.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use trackcraft_api::{Hosted, Hub, NullSink};
use trackcraft_engine::Session;
use trackcraft_engine::sync::Journal;
use trackcraft_engine::sync::proto::{self, Entry, SseEvent, SseParser};

use super::*;

const TOKEN: &str = "tct_sync_secret";

fn setup_hosted(session: Session) -> (Server, Router) {
    let hub = Arc::new(Hub::new());
    hub.add(Hosted::new("lab", session, None));
    let rec =
        TokenRecord { id: "t1".into(), user: "riley".into(), email: None, label: String::new(), sha256: config::hash(TOKEN), created: String::new() };
    let server = Server::new(hub, Arc::new(TokenIdentity::new(vec![rec])), Arc::new(NullSink), Options { log_requests: false, ..Options::default() });
    let router = server.router();
    (server, router)
}

fn setup() -> (Server, Router) {
    setup_hosted(Session::sample())
}

fn req(method: &str, path: &str, body: Option<Value>, auth: bool) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(path).header("host", "track.test").header("content-type", "application/json");
    if auth {
        b = b.header("authorization", format!("Bearer {TOKEN}"));
    }
    b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap()
}

async fn call(r: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = r.clone().oneshot(req(method, path, body, true)).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn command(r: &Router, cmd: &str, params: Value, origin: Option<(&str, u64)>) -> (StatusCode, Value) {
    let mut body = json!({"command": cmd, "params": params});
    if let Some((c, s)) = origin {
        body["origin"] = json!({"client": c, "seq": s});
    }
    call(r, "POST", "/api/command", Some(body)).await
}

/// An open SSE response, read event by event.
struct Events {
    body: axum::body::BodyDataStream,
    buf: String,
    parser: SseParser,
    ready: std::collections::VecDeque<SseEvent>,
}

impl Events {
    async fn open(r: &Router, query: &str) -> (StatusCode, Option<Events>) {
        let resp = r.clone().oneshot(req("GET", &format!("/api/sync/events{query}"), None, true)).await.unwrap();
        let status = resp.status();
        if status != StatusCode::OK {
            return (status, None);
        }
        assert_eq!(resp.headers()["content-type"], "text/event-stream");
        (status, Some(Events { body: resp.into_body().into_data_stream(), buf: String::new(), parser: SseParser::new(), ready: Default::default() }))
    }

    /// The next event, or `None` if the stream ended or stayed quiet for `wait`.
    async fn next(&mut self, wait: Duration) -> Option<SseEvent> {
        loop {
            if let Some(e) = self.ready.pop_front() {
                return Some(e);
            }
            let chunk = tokio::time::timeout(wait, self.body.next()).await.ok()??.ok()?;
            self.buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(i) = self.buf.find('\n') {
                let line: String = self.buf.drain(..=i).collect();
                if let Some(e) = self.parser.feed_line(line.trim_end_matches('\n')) {
                    self.ready.push_back(e);
                }
            }
        }
    }

    async fn entry(&mut self) -> Entry {
        let e = self.next(Duration::from_secs(5)).await.expect("an entry event");
        assert_eq!(e.event, proto::EVENT_ENTRY, "{e:?}");
        let entry: Entry = serde_json::from_str(&e.data).unwrap();
        assert_eq!(e.id, Some(entry.rev.to_string()));
        entry
    }
}

const QUIET: Duration = Duration::from_millis(150);

#[tokio::test]
async fn snapshot_commands_and_journal() {
    let (server, r) = setup();
    let (s, snap) = call(&r, "GET", "/api/sync/snapshot?client=c1", None).await;
    assert_eq!(s, StatusCode::OK);
    let snap: proto::Snapshot = serde_json::from_value(snap).unwrap();
    assert_eq!((snap.rev, snap.user.as_str(), snap.applied_seq, snap.protocol), (0, "riley", None, proto::PROTOCOL));
    let ws0 = trackcraft_store::from_str(&snap.workspace.to_string()).unwrap();

    // A sync client's command: rev 1, its entry, its origin.
    let (s, v) = command(&r, "issue.edit", json!({"key": "LUME-1", "summary": "Synced"}), Some(("c1", 1))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let resp: proto::CommandResponse = serde_json::from_value(v).unwrap();
    let e1 = resp.entry.clone().unwrap();
    assert_eq!((resp.rev, resp.duplicate, e1.rev, e1.actor.as_str()), (1, false, 1, "riley"));
    assert_eq!(e1.origin, Some(proto::Origin { client: "c1".into(), seq: 1 }));
    // Resent after a lost response: not applied twice.
    let (_, v) = command(&r, "issue.edit", json!({"key": "LUME-1", "summary": "Synced"}), Some(("c1", 1))).await;
    assert_eq!((v["duplicate"].as_bool(), v["rev"].as_u64(), v["entry"].is_null()), (Some(true), Some(1), true));
    // Queries produce no entry; the next seq does.
    let (_, v) = command(&r, "issue.get", json!({"key": "LUME-1"}), None).await;
    assert_eq!((v["rev"].as_u64(), v["entry"].is_null()), (Some(1), true));
    let (_, v) = command(&r, "issue.edit", json!({"key": "LUME-1", "summary": "Synced again"}), Some(("c1", 2))).await;
    assert_eq!(v["rev"].as_u64(), Some(2));
    // The Jira surface is journaled too (the journal is written by Session::run).
    let (s, _) = call(&r, "POST", "/rest/api/3/issue/LUME-2/comment", Some(json!({"body": "via jira"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    // Blocked commands, including the path-reading import.
    for id in ["workspace.import", "edit.undo", "file.open"] {
        let (s, _) = command(&r, id, json!({"kind": "jiraCsv", "path": "/etc/passwd"}), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{id}");
    }
    let (_, snap2) = call(&r, "GET", "/api/sync/snapshot?client=c1", None).await;
    assert_eq!((snap2["rev"].as_u64(), snap2["appliedSeq"].as_u64()), (Some(3), Some(2)));

    // Replaying the journal on the first snapshot reproduces the server's workspace exactly.
    let (_, Some(mut ev)) = Events::open(&r, &format!("?since=0&epoch={}", snap.epoch)).await else { panic!("no stream") };
    let hello = ev.next(Duration::from_secs(5)).await.unwrap();
    assert_eq!(hello.event, proto::EVENT_HELLO);
    let hello: proto::Hello = serde_json::from_str(&hello.data).unwrap();
    assert_eq!((hello.rev, hello.oldest, hello.epoch.as_str()), (3, 0, snap.epoch.as_str()));
    let mut client = Session::new(ws0, "riley");
    for _ in 0..3 {
        client.apply_remote(&ev.entry().await).unwrap();
    }
    let h = server.hub.get("lab").unwrap();
    assert!(h.lock().ws == client.ws, "replayed copy equals the server's");
}

#[tokio::test]
async fn events_stream_live_and_resume() {
    let (_server, r) = setup();
    let (_, Some(mut ev)) = Events::open(&r, "").await else { panic!("no stream") };
    assert_eq!(ev.next(Duration::from_secs(5)).await.unwrap().event, proto::EVENT_HELLO);
    assert!(ev.next(QUIET).await.is_none(), "nothing to stream yet");
    for i in 0..3 {
        command(&r, "issue.comment", json!({"key": "LUME-1", "body": format!("c{i}")}), None).await;
    }
    let revs: Vec<u64> = [ev.entry().await, ev.entry().await, ev.entry().await].iter().map(|e| e.rev).collect();
    assert_eq!(revs, vec![1, 2, 3]);
    // A second subscriber resuming from 2 gets only 3, then live entries.
    let (_, Some(mut ev2)) = Events::open(&r, "?since=2").await else { panic!("no stream") };
    ev2.next(Duration::from_secs(5)).await.unwrap();
    assert_eq!(ev2.entry().await.rev, 3);
    command(&r, "issue.comment", json!({"key": "LUME-1", "body": "live"}), None).await;
    assert_eq!(ev2.entry().await.rev, 4);
    assert_eq!(ev.entry().await.rev, 4);
}

#[tokio::test]
async fn resync_when_since_cannot_be_served() {
    let mut s = Session::sample();
    s.enable_journal(Journal::new(2, 1 << 20));
    let (_server, r) = setup_hosted(s);
    for i in 0..4 {
        command(&r, "issue.comment", json!({"key": "LUME-1", "body": format!("c{i}")}), None).await;
    }
    for (q, why) in [("?since=1", "older"), ("?since=9", "ahead"), ("?since=4&epoch=other", "restart")] {
        let (_, Some(mut ev)) = Events::open(&r, q).await else { panic!("no stream") };
        assert_eq!(ev.next(Duration::from_secs(5)).await.unwrap().event, proto::EVENT_HELLO);
        let e = ev.next(Duration::from_secs(5)).await.unwrap();
        assert_eq!(e.event, proto::EVENT_RESYNC, "{q}");
        let rs: proto::Resync = serde_json::from_str(&e.data).unwrap();
        assert!(rs.reason.contains(why), "{q}: {}", rs.reason);
        assert_eq!(rs.rev, 4);
        assert!(ev.next(Duration::from_secs(2)).await.is_none(), "{q}: the stream ends after resync");
    }
    let (_, Some(mut ok)) = Events::open(&r, "?since=2").await else { panic!("no stream") };
    ok.next(Duration::from_secs(5)).await.unwrap();
    assert_eq!(ok.entry().await.rev, 3);
}

#[tokio::test]
async fn hostile_sync_requests() {
    let (server, r) = setup();
    for q in ["?since=abc", "?since=-1", "?client=bad%20id", "?since=99999999999999999999999"] {
        let path = if q.contains("client") { format!("/api/sync/snapshot{q}") } else { format!("/api/sync/events{q}") };
        let (s, _) = call(&r, "GET", &path, None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{q}");
    }
    for origin in [json!("x"), json!({"client": "", "seq": 1}), json!({"client": "a b", "seq": 1}), json!({"client": "c", "seq": -1})] {
        let (s, _) = call(&r, "POST", "/api/command", Some(json!({"command": "issue.get", "params": {"key": "LUME-1"}, "origin": origin}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    for path in ["/api/sync/snapshot", "/api/sync/events"] {
        let resp = r.clone().oneshot(req("GET", path, None, false)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
    // A multi-command request that fails half-way leaves no journal entries behind.
    let rev = server.hub.get("lab").unwrap().lock().sync_rev();
    let (s, _) =
        call(&r, "PUT", "/rest/api/3/issue/LUME-1", Some(json!({"fields": {"summary": "half"}, "update": {"comment": [{"add": {"body": 5}}]}})))
            .await;
    assert!(s.is_client_error(), "{s}");
    let h = server.hub.get("lab").unwrap();
    let after = h.lock();
    assert_eq!(after.sync_rev(), rev);
    assert!(after.journal_since(0, 100).unwrap().is_empty());
}
