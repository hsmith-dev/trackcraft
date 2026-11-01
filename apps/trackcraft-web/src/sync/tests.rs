//! The browser sync state machine against a fake transport: an in-memory server session with a
//! journal (the real engine), and clients whose actions are carried out synchronously.

use serde_json::{Value, json};
use trackcraft_engine::Session;
use trackcraft_engine::sync::Journal;

use super::*;

struct FakeServer {
    s: Session,
    epoch: String,
    up: bool,
    /// Answers 401 to everything.
    deny: bool,
}

impl FakeServer {
    fn new() -> Self {
        let mut s = Session::sample();
        s.enable_journal(Journal::new(10_000, 64 << 20));
        FakeServer { s, epoch: "e1".into(), up: true, deny: false }
    }

    fn down(&self) -> Option<Failure> {
        if !self.up {
            Some(Failure::new(0, "NetworkError when attempting to fetch resource."))
        } else if self.deny {
            Some(Failure::new(401, "unauthorized"))
        } else {
            None
        }
    }

    fn snapshot(&self, client: &str, user: &str) -> Snapshot {
        let text = trackcraft_store::to_string(&self.s.ws).unwrap();
        Snapshot {
            protocol: proto::PROTOCOL,
            epoch: self.epoch.clone(),
            rev: self.s.sync_rev(),
            user: user.into(),
            applied_seq: self.s.journal().and_then(|j| j.applied_seq(client)),
            workspace: serde_json::from_str(&text).unwrap(),
        }
    }

    /// What `POST /api/command` does (`crates/api/src/native.rs`).
    fn post(&mut self, body: &str, user: &str) -> Result<CommandResponse, Failure> {
        let req: CommandRequest = serde_json::from_str(body).unwrap();
        if let Some(o) = &req.origin
            && self.s.journal().and_then(|j| j.applied_seq(&o.client)).is_some_and(|m| o.seq <= m)
        {
            return Ok(CommandResponse { command: req.command, result: Value::Null, rev: self.s.sync_rev(), entry: None, duplicate: true });
        }
        let before = self.s.sync_rev();
        let prev = std::mem::replace(&mut self.s.user, user.into());
        self.s.set_origin(req.origin.clone());
        let r = self.s.run(&req.command, &req.params);
        self.s.set_origin(None);
        self.s.user = prev;
        let result = r.map_err(|e| Failure::new(400, e))?;
        let rev = self.s.sync_rev();
        let entry = if rev != before { self.s.journal().and_then(|j| j.last()).cloned() } else { None };
        Ok(CommandResponse { command: req.command, result, rev, entry, duplicate: false })
    }

    fn entries_after(&self, rev: u64) -> Vec<Entry> {
        self.s.journal_since(rev, usize::MAX).unwrap_or_default()
    }

    /// Someone else (a REST caller) changes the workspace.
    fn other(&mut self, cmd: &str, params: Value) {
        let prev = std::mem::replace(&mut self.s.user, "sam".into());
        self.s.run(cmd, &params).unwrap();
        self.s.user = prev;
    }
}

struct Client {
    sync: Sync,
    view: Session,
    user: String,
    inbox: Vec<Net>,
    /// The open stream's last delivered revision.
    stream: Option<u64>,
    /// Drop the next POST answer after the server applied it (a lost response).
    lose_next_answer: bool,
    now: f64,
    actions: Vec<Action>,
    notices: Vec<Notice>,
}

impl Client {
    fn connect(server: &FakeServer, client: &str, user: &str) -> Client {
        let (sync, view) = Sync::new(client, &server.snapshot(client, user)).unwrap();
        Client { sync, view, user: user.into(), inbox: vec![], stream: None, lose_next_answer: false, now: 0.0, actions: vec![], notices: vec![] }
    }

    fn run(&mut self, cmd: &str, params: Value) {
        self.view.run(cmd, &params).unwrap();
    }

    /// One step: carry out the actions synchronously against `server`.
    fn turn(&mut self, server: &mut FakeServer) {
        // Live entries on the open stream.
        if let Some(seen) = self.stream {
            if server.up {
                for e in server.entries_after(seen) {
                    self.stream = Some(e.rev);
                    self.inbox.push(Net::Stream { event: proto::EVENT_ENTRY.into(), data: serde_json::to_string(&e).unwrap() });
                }
            } else {
                self.stream = None;
                self.inbox.push(Net::StreamError);
            }
        }
        let nets = std::mem::take(&mut self.inbox);
        let step = self.sync.step(&mut self.view, self.now, nets);
        self.notices.extend(step.notices);
        for a in step.actions {
            self.actions.push(a.clone());
            match a {
                Action::FetchSnapshot => {
                    let r = match server.down() {
                        Some(f) => Err(f),
                        None => Ok(Box::new(server.snapshot(self.sync.client(), &self.user))),
                    };
                    self.inbox.push(Net::Snapshot(r));
                }
                Action::FetchTicket => self.inbox.push(Net::Ticket(server.down().map_or(Ok("tcs_x".into()), Err))),
                Action::OpenStream { query, .. } => {
                    let since: u64 = query.split('&').find_map(|kv| kv.strip_prefix("since=")).unwrap().parse().unwrap();
                    let epoch = query.split('&').find_map(|kv| kv.strip_prefix("epoch=")).unwrap();
                    if server.down().is_some() {
                        self.inbox.push(Net::StreamError);
                        continue;
                    }
                    let hello = json!({"protocol": 1, "epoch": server.epoch, "rev": server.s.sync_rev(), "oldest": 0});
                    self.inbox.push(Net::Stream { event: proto::EVENT_HELLO.into(), data: hello.to_string() });
                    if epoch != server.epoch || since > server.s.sync_rev() {
                        self.inbox.push(Net::Stream { event: proto::EVENT_RESYNC.into(), data: "{}".into() });
                        self.inbox.push(Net::StreamError);
                    } else {
                        self.stream = Some(since);
                    }
                }
                Action::CloseStream => self.stream = None,
                Action::Post { seq, body } => {
                    let result = match server.down() {
                        Some(f) => Err(f),
                        None => server.post(&body, &self.user),
                    };
                    if self.lose_next_answer && result.is_ok() {
                        self.lose_next_answer = false;
                        self.inbox.push(Net::Posted { seq, result: Err(Failure::new(0, "connection reset")) });
                    } else {
                        self.inbox.push(Net::Posted { seq, result });
                    }
                }
            }
        }
    }

    fn summary(&self, key: &str) -> String {
        self.view.ws.issues.iter().find(|(k, _)| k.to_string() == key).map(|(_, i)| i.summary.clone()).unwrap_or_default()
    }

    fn has_notice(&self, f: impl Fn(&Notice) -> bool) -> bool {
        self.notices.iter().any(f)
    }
}

/// Steps every client (time moving 1 s per round) until all agree with the server.
fn settle(server: &mut FakeServer, clients: &mut [&mut Client]) {
    for _ in 0..200 {
        for c in clients.iter_mut() {
            c.now += 1.0;
            c.turn(server);
        }
        let done = clients.iter().all(|c| {
            c.sync.pending() == 0
                && c.inbox.is_empty()
                && c.view.ws == server.s.ws
                && c.sync.confirmed().ws == server.s.ws
                && c.sync.state() == &ConnState::Online
        });
        if done {
            return;
        }
    }
    panic!("did not converge: {:?}", clients.iter().map(|c| (c.sync.status_text(), c.sync.pending())).collect::<Vec<_>>());
}

#[test]
fn two_clients_converge_and_server_order_wins() {
    let mut server = FakeServer::new();
    let mut a = Client::connect(&server, "a", "riley");
    let mut b = Client::connect(&server, "b", "sam");
    assert!(a.view.is_remote());
    settle(&mut server, &mut [&mut a, &mut b]);
    assert!(a.actions.contains(&Action::FetchTicket));
    assert!(a.notices.is_empty(), "connecting is not news: {:?}", a.notices);

    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Riley's name"}));
    a.run("issue.create", json!({"project": "LUME", "summary": "New from Riley"}));
    b.run("issue.edit", json!({"key": "LUME-1", "summary": "Sam's name"}));
    b.run("issue.comment", json!({"key": "LUME-2", "body": "Seen"}));
    // Optimistic: each sees its own change at once.
    assert_eq!(a.summary("LUME-1"), "Riley's name");
    assert_eq!(b.summary("LUME-1"), "Sam's name");
    settle(&mut server, &mut [&mut a, &mut b]);
    // A posted first, so B's rename is later in server order and wins everywhere.
    assert_eq!(a.summary("LUME-1"), "Sam's name");
    assert_eq!(b.summary("LUME-1"), "Sam's name");
    assert_eq!(a.view.ws, b.view.ws);

    // Someone else's change while we have nothing pending: replayed onto the view.
    server.other("issue.edit", json!({"key": "LUME-2", "summary": "From REST"}));
    settle(&mut server, &mut [&mut a, &mut b]);
    assert_eq!(a.summary("LUME-2"), "From REST");
}

#[test]
fn offline_queue_replays_in_order_on_reconnect() {
    let mut server = FakeServer::new();
    let mut a = Client::connect(&server, "a", "riley");
    settle(&mut server, &mut [&mut a]);
    server.up = false;
    a.inbox.push(Net::Online(false));
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Offline 1"}));
    a.run("issue.comment", json!({"key": "LUME-2", "body": "Offline 2"}));
    a.run("issue.edit", json!({"key": "LUME-1", "summary": "Offline 3"}));
    for _ in 0..30 {
        a.now += 1.0;
        a.turn(&mut server);
    }
    assert!(matches!(a.sync.state(), ConnState::Offline { .. }));
    assert_eq!(a.sync.status_text(), "Offline · 3 unsent");
    assert!(a.has_notice(|n| matches!(n, Notice::State(ConnState::Offline { .. }))));
    // Edits stay visible meanwhile.
    assert_eq!(a.summary("LUME-1"), "Offline 3");
    // Someone else changed things while we were away.
    server.other("issue.edit", json!({"key": "LUME-3", "summary": "While you were out"}));
    // Retries back off (at most every MAX_BACKOFF seconds): not one POST per step.
    let posts = a.actions.iter().filter(|x| matches!(x, Action::Post { .. })).count();
    assert!(posts <= 12, "{posts} posts in 30 s");

    server.up = true;
    a.inbox.push(Net::Online(true));
    settle(&mut server, &mut [&mut a]);
    assert_eq!(a.summary("LUME-1"), "Offline 3");
    assert_eq!(a.summary("LUME-3"), "While you were out");
    let seqs: Vec<u64> = server.entries_after(0).iter().filter_map(|e| e.origin.as_ref().map(|o| o.seq)).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
    assert!(a.has_notice(|n| n == &Notice::State(ConnState::Online)));
}

#[test]
fn a_lost_answer_is_resent_and_deduplicated() {
    let mut server = FakeServer::new();
    let mut a = Client::connect(&server, "a", "riley");
    settle(&mut server, &mut [&mut a]);
    a.lose_next_answer = true;
    a.run("issue.comment", json!({"key": "LUME-1", "body": "Once"}));
    settle(&mut server, &mut [&mut a]);
    let comments = server.entries_after(0).iter().filter(|e| e.command == "issue.comment").count();
    assert_eq!(comments, 1);
}

#[test]
fn refused_commands_are_dropped_and_reported() {
    let mut server = FakeServer::new();
    let mut a = Client::connect(&server, "a", "riley");
    settle(&mut server, &mut [&mut a]);
    // Deleted on the server before our comment arrives.
    server.other("issue.delete", json!({"key": "LUME-3"}));
    a.run("issue.comment", json!({"key": "LUME-3", "body": "Too late"}));
    settle(&mut server, &mut [&mut a]);
    assert!(a.has_notice(|n| matches!(n, Notice::Dropped(l) if l.len() == 1 && l[0].starts_with("issue.comment LUME-3"))));
}

#[test]
fn unauthorized_keeps_the_queue() {
    let mut server = FakeServer::new();
    let mut a = Client::connect(&server, "a", "riley");
    settle(&mut server, &mut [&mut a]);
    server.deny = true;
    a.run("issue.comment", json!({"key": "LUME-1", "body": "Queued"}));
    for _ in 0..5 {
        a.now += 1.0;
        a.turn(&mut server);
    }
    assert!(matches!(a.sync.state(), ConnState::Unauthorized { .. }));
    assert_eq!(a.sync.pending(), 1);
    assert!(a.has_notice(|n| n.is_error() && matches!(n, Notice::State(ConnState::Unauthorized { .. }))));
    server.deny = false;
    settle(&mut server, &mut [&mut a]);
}

#[test]
fn resync_decisions() {
    // A gap in the stream: resync from a snapshot, keeping pending commands not yet applied.
    let mut server = FakeServer::new();
    let mut a = Client::connect(&server, "a", "riley");
    settle(&mut server, &mut [&mut a]);
    server.other("issue.edit", json!({"key": "LUME-1", "summary": "r1"}));
    server.other("issue.edit", json!({"key": "LUME-1", "summary": "r2"}));
    let skipped = server.entries_after(1);
    a.stream = None;
    a.actions.clear();
    a.inbox.push(Net::Stream { event: proto::EVENT_ENTRY.into(), data: serde_json::to_string(&skipped[0]).unwrap() });
    a.turn(&mut server);
    assert!(a.has_notice(|n| matches!(n, Notice::Resync(w) if w.contains("missing revisions before 2"))));
    assert_eq!(a.actions, vec![Action::CloseStream, Action::FetchSnapshot]);
    assert_eq!(a.sync.state(), &ConnState::Resyncing);
    settle(&mut server, &mut [&mut a]);
    assert_eq!(a.summary("LUME-1"), "r2");

    // An unreadable entry also resyncs.
    a.notices.clear();
    a.inbox.push(Net::Stream { event: proto::EVENT_ENTRY.into(), data: "{nope".into() });
    a.turn(&mut server);
    assert!(a.has_notice(|n| matches!(n, Notice::Resync(_))));
    settle(&mut server, &mut [&mut a]);

    // A server restart (new epoch): the stream answers `resync`; a pending command the snapshot
    // already contains (`appliedSeq`) is not re-run or resent.
    server.epoch = "e2".into();
    // The stream dies silently with the old server; the answer to our POST is lost.
    a.stream = None;
    a.lose_next_answer = true;
    a.run("issue.comment", json!({"key": "LUME-2", "body": "applied before the restart"}));
    a.turn(&mut server);
    a.turn(&mut server);
    assert_eq!(a.sync.pending(), 1);
    a.inbox.push(Net::StreamError);
    a.actions.clear();
    settle(&mut server, &mut [&mut a]);
    assert!(a.actions.contains(&Action::FetchSnapshot));
    let comments = server.entries_after(0).iter().filter(|e| e.command == "issue.comment").count();
    assert_eq!(comments, 1);

    // A replay that diverges (the server's id counter disagrees): resync, not a crash.
    let mut e = server.entries_after(0).last().cloned().unwrap();
    e.rev = a.sync.rev() + 1;
    e.ids_before = 999_999;
    a.notices.clear();
    a.inbox.push(Net::Stream { event: proto::EVENT_ENTRY.into(), data: serde_json::to_string(&e).unwrap() });
    a.turn(&mut server);
    assert!(a.has_notice(|n| matches!(n, Notice::Resync(_))));
    settle(&mut server, &mut [&mut a]);
}

#[test]
fn stale_answers_and_junk_are_ignored() {
    let mut server = FakeServer::new();
    let mut a = Client::connect(&server, "a", "riley");
    settle(&mut server, &mut [&mut a]);
    // Answers nobody asked for, junk events.
    a.inbox.extend([
        Net::Ticket(Ok("tcs_stale".into())),
        Net::Snapshot(Err(Failure::new(500, "x"))),
        Net::Posted { seq: 77, result: Err(Failure::new(400, "x")) },
        Net::Stream { event: "bogus".into(), data: "\u{0}".into() },
        Net::Stream { event: proto::EVENT_HELLO.into(), data: "not json".into() },
    ]);
    a.actions.clear();
    a.turn(&mut server);
    assert!(a.actions.is_empty(), "{:?}", a.actions);
    settle(&mut server, &mut [&mut a]);
    // A snapshot that doesn't load is retried, not applied.
    let mut snap = server.snapshot("a", "riley");
    snap.workspace = json!({"format": "nope"});
    a.inbox.push(Net::Stream { event: proto::EVENT_RESYNC.into(), data: "{}".into() });
    a.turn(&mut server);
    a.inbox.clear();
    a.inbox.push(Net::Snapshot(Ok(Box::new(snap))));
    a.turn(&mut server);
    a.inbox.clear();
    settle(&mut server, &mut [&mut a]);
}

#[test]
fn helpers() {
    assert_eq!(workspace_base(""), "");
    assert_eq!(workspace_base(" lab "), "w/lab/");
    assert_eq!(workspace_base("a b/c"), "w/a%20b%2Fc/");
    assert_eq!(clean_token("  tct_abc \n").unwrap(), "tct_abc");
    for bad in ["", "   ", "a b", "tct_\u{7}", "é", &"x".repeat(2000)] {
        assert!(clean_token(bad).is_err(), "{bad:?}");
    }
    let server = FakeServer::new();
    let text = serde_json::to_string(&server.snapshot("a", "riley")).unwrap();
    assert!(parse_snapshot(&text).is_ok());
    assert!(parse_snapshot("{}").is_err());
    assert!(parse_snapshot(&text.replacen("\"protocol\":1", "\"protocol\":9", 1)).is_err());
    assert!(Sync::new("bad id", &server.snapshot("a", "riley")).is_err());
    assert!(Notice::Dropped(vec!["x".into()]).message("s").contains("refused 1"));
}
