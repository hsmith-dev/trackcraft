//! The client against a real in-process server on 127.0.0.1: convergence, offline queue,
//! conflicts (server order wins), refused commands, server restarts.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use trackcraft_api::{Hosted, Hub, NullSink};
use trackcraft_engine::Session;
use trackcraft_server::{Options, Server, TokenIdentity, TokenRecord, config};

use super::*;

const RILEY: &str = "tct_riley_sync";
const SAM: &str = "tct_sam_sync";

fn rec(user: &str, token: &str) -> TokenRecord {
    TokenRecord { id: user.into(), user: user.into(), email: None, label: String::new(), sha256: config::hash(token), created: String::new() }
}

fn server_over(hub: Arc<Hub>) -> Server {
    let ident = TokenIdentity::new(vec![rec("riley", RILEY), rec("sam", SAM)]);
    Server::new(hub, Arc::new(ident), Arc::new(NullSink), Options { log_requests: false, ..Options::default() })
}

fn new_server() -> Server {
    let hub = Arc::new(Hub::new());
    hub.add(Hosted::new("team", Session::sample(), None));
    server_over(hub)
}

/// A server listening on `addr` (port 0 = any) until dropped.
struct Running {
    rt: Option<tokio::runtime::Runtime>,
    addr: SocketAddr,
}

impl Running {
    fn start(server: &Server, addr: &str) -> Running {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let listener = rt.block_on(async {
            for _ in 0..100 {
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => return l,
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                }
            }
            panic!("could not bind {addr}");
        });
        let addr = listener.local_addr().unwrap();
        let router = server.router();
        rt.spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Running { rt: Some(rt), addr }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Kills the server and every open connection.
    fn stop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_timeout(Duration::from_millis(100));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

fn cfg(url: &str, token: &str) -> Config {
    let mut c = Config::new(url, Some("team"), token);
    c.timeout = Duration::from_secs(3);
    c.max_backoff = Duration::from_millis(200);
    c
}

struct Client {
    remote: Remote,
    view: Session,
    notices: Vec<Notice>,
}

impl Client {
    fn connect(url: &str, token: &str) -> Client {
        let (remote, view) = Remote::connect(cfg(url, token), None).unwrap();
        Client { remote, view, notices: vec![] }
    }
    fn pump(&mut self) {
        let n = self.remote.pump(&mut self.view);
        self.notices.extend(n);
    }
    fn run(&mut self, id: &str, p: serde_json::Value) {
        self.view.run(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    fn summary(&self, key: &str) -> String {
        self.view.ws.issues.iter().find(|(k, _)| k.to_string() == key).map(|(_, i)| i.summary.clone()).unwrap_or_default()
    }
}

fn server_ws(s: &Server) -> trackcraft_engine::model::Workspace {
    s.hub.get("team").unwrap().lock().ws.clone()
}

/// Pumps every client until `done` holds (or fails after 10 s).
fn settle(clients: &mut [&mut Client], what: &str, mut done: impl FnMut(&[&mut Client]) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        for c in clients.iter_mut() {
            c.pump();
        }
        if done(clients) {
            return;
        }
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn converged(server: &Server) -> impl FnMut(&[&mut Client]) -> bool + '_ {
    move |cs| {
        let ws = server_ws(server);
        cs.iter().all(|c| c.remote.pending() == 0 && c.view.ws == ws && c.remote.confirmed().ws == ws)
    }
}

#[test]
fn two_clients_converge() {
    let server = new_server();
    let run = Running::start(&server, "127.0.0.1:0");
    let mut a = Client::connect(&run.url(), RILEY);
    let mut b = Client::connect(&run.url(), SAM);
    assert_eq!((a.view.user.as_str(), b.view.user.as_str()), ("riley", "sam"));
    assert!(a.view.is_remote() && !a.view.can_undo());
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Edited by Riley"}));
    a.run("issue.create", json!({"project": "LUME", "summary": "New from Riley"}));
    a.run("page.create", json!({"space": "ENG", "title": "Sync notes"}));
    settle(&mut [&mut a, &mut b], "B sees A's edits", converged(&server));
    assert_eq!(b.summary("LUME-1"), "Edited by Riley");
    assert!(b.view.ws.issues.values().any(|i| i.summary == "New from Riley" && i.reporter == "riley"));
    assert_eq!(a.remote.rev(), 3);
    assert_eq!(a.remote.state(), &ConnState::Online);
    // B answers; both converge again.
    b.run("issue.comment", json!({"key": "LUME-1", "body": "Seen"}));
    settle(&mut [&mut a, &mut b], "A sees B's comment", converged(&server));
    assert!(a.notices.iter().all(|n| !matches!(n, Notice::Dropped(_) | Notice::Resync(_))), "{:?}", a.notices);
    // Undo and host-file commands are refused in remote mode, and nothing is sent for them.
    assert!(a.view.run("edit.undo", &json!({})).is_err());
    assert!(a.view.run("file.save", &json!({"path": "/tmp/x.track"})).is_err());
    a.pump();
    assert_eq!(a.remote.pending(), 0);
}

#[test]
fn conflicting_renames_end_in_server_order() {
    let server = new_server();
    let run = Running::start(&server, "127.0.0.1:0");
    let mut a = Client::connect(&run.url(), RILEY);
    let mut b = Client::connect(&run.url(), SAM);
    // Both rename LUME-1 before seeing the other's change.
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Riley's name"}));
    b.run("issue.edit", json!({"key": "LUME-1", "summary": "Sam's name"}));
    // A's reaches the server first, then B's.
    settle(&mut [&mut a], "A's rename applied", |cs| cs[0].remote.pending() == 0);
    assert_eq!(b.summary("LUME-1"), "Sam's name", "B still shows its optimistic edit");
    settle(&mut [&mut a, &mut b], "both converge", converged(&server));
    for c in [&a, &b] {
        assert_eq!(c.summary("LUME-1"), "Sam's name", "the later command in server order wins");
    }
    // The history keeps both changes, in server order.
    let hist = a.view.run("issue.history", &json!({"key": "LUME-1"})).unwrap();
    let text = hist.to_string();
    assert!(text.contains("Riley's name") && text.contains("Sam's name"), "{text}");
}

#[test]
fn offline_changes_queue_and_replay_on_reconnect() {
    let server = new_server();
    let mut run = Running::start(&server, "127.0.0.1:0");
    let addr = run.addr;
    let mut a = Client::connect(&run.url(), RILEY);
    settle(&mut [&mut a], "online", |cs| cs[0].remote.state() == &ConnState::Online);
    run.stop();
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Offline 1"}));
    a.run("issue.comment", json!({"key": "LUME-2", "body": "Offline 2"}));
    settle(&mut [&mut a], "offline noticed", |cs| matches!(cs[0].remote.state(), ConnState::Offline { .. }));
    assert_eq!(a.remote.pending(), 2);
    assert!(a.remote.status_text().contains("2 unsent"));
    assert_eq!(a.summary("LUME-1"), "Offline 1", "the local view keeps offline edits");
    assert!(a.notices.iter().any(|n| matches!(n, Notice::State(ConnState::Offline { .. }))));
    // Same server comes back on the same port: the queue is sent in order.
    let run2 = Running::start(&server, &addr.to_string());
    settle(&mut [&mut a], "queue replayed", converged(&server));
    settle(&mut [&mut a], "event stream back", |cs| cs[0].remote.state() == &ConnState::Online);
    assert!(a.notices.iter().any(|n| n == &Notice::State(ConnState::Online)), "reconnecting is announced");
    let ws = server_ws(&server);
    let lume1 = ws.issues.iter().find(|(k, _)| k.to_string() == "LUME-1").unwrap().1;
    assert_eq!(lume1.summary, "Offline 1");
    // A newcomer sees them too.
    let mut b = Client::connect(&run2.url(), SAM);
    settle(&mut [&mut a, &mut b], "newcomer converged", converged(&server));
    assert_eq!(b.summary("LUME-1"), "Offline 1");
}

#[test]
fn refused_commands_are_dropped_and_reported() {
    let server = new_server();
    let run = Running::start(&server, "127.0.0.1:0");
    let mut a = Client::connect(&run.url(), RILEY);
    let mut b = Client::connect(&run.url(), SAM);
    b.run("issue.delete", json!({"key": "LUME-3"}));
    settle(&mut [&mut b], "delete applied", |cs| cs[0].remote.pending() == 0);
    // A hasn't pumped yet: it still shows LUME-3 and comments on it.
    a.run("issue.comment", json!({"key": "LUME-3", "body": "Too late"}));
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Still fine"}));
    settle(&mut [&mut a, &mut b], "converged after the refusal", converged(&server));
    let dropped: Vec<&Notice> = a.notices.iter().filter(|n| matches!(n, Notice::Dropped(_))).collect();
    assert_eq!(dropped.len(), 1, "{:?}", a.notices);
    let msg = dropped[0].message(a.remote.server());
    assert!(msg.contains("issue.comment LUME-3"), "{msg}");
    assert_eq!(b.summary("LUME-1"), "Still fine", "later commands still go through");
    assert!(a.view.ws.issues.keys().all(|k| k.to_string() != "LUME-3"));
}

#[test]
fn a_restarted_server_triggers_a_resync() {
    let server = new_server();
    let mut run = Running::start(&server, "127.0.0.1:0");
    let addr = run.addr;
    let mut a = Client::connect(&run.url(), RILEY);
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Before restart"}));
    settle(&mut [&mut a], "applied", converged(&server));
    run.stop();
    // "Restart": a new process loads the saved workspace (new epoch, journal from 0).
    let saved = server_ws(&server);
    let hub = Arc::new(Hub::new());
    hub.add(Hosted::new("team", Session::new(saved, "riley"), None));
    let restarted = server_over(hub);
    a.run("issue.edit", json!({"key": "LUME-2", "summary": "Queued across the restart"}));
    let _run2 = Running::start(&restarted, &addr.to_string());
    settle(&mut [&mut a], "resynced and converged", converged(&restarted));
    assert_eq!(a.summary("LUME-1"), "Before restart");
    assert_eq!(a.summary("LUME-2"), "Queued across the restart");
    assert_eq!(a.remote.rev(), 1, "revisions restart with the new epoch");
}

#[test]
fn diverged_client_resyncs_and_keeps_going() {
    let server = new_server();
    let run = Running::start(&server, "127.0.0.1:0");
    let mut a = Client::connect(&run.url(), RILEY);
    let mut b = Client::connect(&run.url(), SAM);
    // Corrupt A's confirmed copy behind its back (as a bug would): the next entry won't replay.
    a.remote.confirmed.run("issue.comment", &json!({"key": "LUME-1", "body": "phantom"})).unwrap();
    b.run("issue.create", json!({"project": "OPS", "summary": "After divergence"}));
    settle(&mut [&mut a, &mut b], "A resynced", converged(&server));
    assert!(a.notices.iter().any(|n| matches!(n, Notice::Resync(_))), "{:?}", a.notices);
    assert!(a.view.ws.issues.values().any(|i| i.summary == "After divergence"));
}

#[test]
fn connect_errors() {
    let server = new_server();
    let run = Running::start(&server, "127.0.0.1:0");
    for (url, token, want) in [
        (run.url(), "tct_wrong", "refused the token"),
        (run.url(), "bad token", "token"),
        ("https://example.com".to_string(), RILEY, "https"),
        ("http://127.0.0.1:1".to_string(), RILEY, "127.0.0.1"),
    ] {
        let err = Remote::connect(cfg(&url, token), None).err().unwrap();
        assert!(err.contains(want), "{url} {token}: {err}");
    }
    let mut c = cfg(&run.url(), RILEY);
    c.workspace = Some("nope".into());
    assert!(Remote::connect(c, None).err().unwrap().contains("404"));
}
