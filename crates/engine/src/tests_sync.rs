//! M4.2: journal, deterministic replay, remote mode and the after-command observer.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::sync::{Journal, ReplayError, SinceError, ids_next, proto::Entry};
use crate::{Executed, LOCAL_ONLY, Session};

static TICK: AtomicI64 = AtomicI64::new(0);

/// A clock that moves on every call, so a replay that didn't pin `at` would differ.
fn moving_now() -> trackcraft_model::Timestamp {
    let n = TICK.fetch_add(1, Ordering::SeqCst);
    chrono::DateTime::from_timestamp(1_793_448_000 + n, 1_000).unwrap_or_default()
}

fn base() -> trackcraft_model::Workspace {
    crate::sample::workspace(chrono::DateTime::from_timestamp(1_793_448_000, 0).unwrap_or_default())
}

fn server() -> Session {
    let mut s = Session::new(base(), "riley").with_clock(moving_now);
    s.enable_journal(Journal::new(10_000, 64 << 20));
    s
}

/// xorshift: small, seeded, reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
    fn n(&mut self, max: u64) -> u64 {
        self.next() % max
    }
}

/// A random command: mostly plausible, sometimes junk (failures must not reach the journal).
fn random_command(r: &mut Rng, s: &Session) -> (String, Value) {
    let keys: Vec<String> = s.ws.issues.keys().map(|k| k.to_string()).collect();
    let key = if keys.is_empty() { "LUME-1".to_string() } else { r.pick(&keys).clone() };
    let pages: Vec<u64> = s.ws.pages.keys().map(|k| k.0).collect();
    let page = if pages.is_empty() { 1 } else { *r.pick(&pages) };
    let n = r.n(1000);
    match r.n(14) {
        0 => ("issue.create".into(), json!({"project": r.pick(&["LUME", "OPS"]), "summary": format!("Task {n}"), "labels": ["sync"]})),
        1 => ("issue.edit".into(), json!({"key": key, "summary": format!("Renamed {n}"), "storyPoints": n % 8})),
        2 => ("issue.transition".into(), json!({"key": key, "to": r.pick(&["To Do", "In Progress", "Done", "Nope"])})),
        3 => ("issue.comment".into(), json!({"key": key, "body": format!("Comment **{n}**")})),
        4 => ("issue.assign".into(), json!({"key": key, "assignee": r.pick(&["riley", "sam", "jo", "ghost"])})),
        5 => ("issue.link".into(), json!({"key": key, "type": "relates to", "target": r.pick(&keys)})),
        6 => ("issue.delete".into(), json!({"key": key})),
        7 => ("page.create".into(), json!({"space": r.pick(&["ENG", "TEAM"]), "title": format!("Page {n}"), "body": "Hello"})),
        8 => ("page.edit".into(), json!({"id": page, "body": format!("Body {n}")})),
        9 => ("page.comment".into(), json!({"id": page, "body": "Looks good"})),
        10 => ("sprint.create".into(), json!({"project": "LUME", "goal": format!("Goal {n}")})),
        11 => ("user.add".into(), json!({"id": format!("u{n}"), "name": format!("User {n}")})),
        12 => ("issue.rank".into(), json!({"keys": [key], "before": r.pick(&keys)})),
        _ => ("issue.edit".into(), json!({"key": 5, "summary": {}})),
    }
}

#[test]
fn replay_is_deterministic_over_random_sequences() {
    for seed in 1..=12u64 {
        let mut a = server();
        // B starts from the same snapshot but has its own (moving) clock: replays must not use it.
        let mut b = Session::new(base(), "sam").with_clock(moving_now);
        b.set_remote(true);
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        for _ in 0..120 {
            a.user = r.pick(&["riley", "sam", "jo"]).to_string();
            let (id, p) = random_command(&mut r, &a);
            let _ = a.run(&id, &p);
        }
        let entries = a.journal_since(0, usize::MAX).unwrap();
        assert_eq!(entries.len() as u64, a.sync_rev(), "seed {seed}: one entry per change");
        assert!(entries.len() > 20, "seed {seed}: too few successful commands ({})", entries.len());
        for e in &entries {
            b.apply_remote(e).unwrap_or_else(|err| panic!("seed {seed}: rev {}: {err}", e.rev));
        }
        assert_eq!(b.sync_rev(), a.sync_rev());
        assert!(a.ws == b.ws, "seed {seed}: workspaces differ after replay");
        assert_eq!(b.user, "sam", "replay restores the acting user");
    }
}

#[test]
fn entries_survive_the_wire() {
    let mut a = server();
    a.run("issue.create", &json!({"project": "LUME", "summary": "Wire"})).unwrap();
    a.run("page.create", &json!({"space": "ENG", "title": "Wire page"})).unwrap();
    let mut b = Session::new(base(), "riley");
    for e in a.journal_since(0, 100).unwrap() {
        let wire: Entry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        b.apply_remote(&wire).unwrap();
    }
    assert!(a.ws == b.ws);
}

#[test]
fn replay_detects_divergence() {
    let mut a = server();
    a.run("issue.create", &json!({"project": "LUME", "summary": "One"})).unwrap();
    a.run("issue.create", &json!({"project": "LUME", "summary": "Two"})).unwrap();
    let es = a.journal_since(0, 10).unwrap();
    let mut b = Session::new(base(), "riley");
    assert_eq!(b.apply_remote(&es[1]), Err(ReplayError::Gap { expected: 1, got: 2 }));
    // A local change B made on its own moves its id counter.
    b.run("issue.comment", &json!({"key": "LUME-1", "body": "local"})).unwrap();
    assert!(matches!(b.apply_remote(&es[0]), Err(ReplayError::Diverged { .. })));
    let mut c = Session::new(base(), "riley");
    let before = c.ws.clone();
    let mut bad = es[0].clone();
    bad.params = json!({"project": "NOPE", "summary": "x"});
    assert!(matches!(c.apply_remote(&bad), Err(ReplayError::Failed(_))));
    assert!(c.ws == before && c.sync_rev() == 0, "a failed replay changes nothing");
    for id in LOCAL_ONLY {
        let e = Entry { command: id.to_string(), ..es[0].clone() };
        assert!(c.apply_remote(&e).is_err(), "{id} must not replay");
    }
}

#[test]
fn remote_mode_has_no_undo_and_refuses_local_only_commands() {
    let mut s = Session::new(base(), "riley");
    s.run("issue.edit", &json!({"key": "LUME-1", "summary": "local"})).unwrap();
    assert!(s.can_undo());
    s.set_remote(true);
    assert!(!s.can_undo());
    s.run("issue.edit", &json!({"key": "LUME-1", "summary": "remote"})).unwrap();
    assert!(!s.can_undo(), "remote mode records no undo step");
    for id in LOCAL_ONLY {
        let e = s.run(id, &json!({"path": "/tmp/x.track", "id": "sam", "kind": "jiraCsv"})).unwrap_err();
        assert!(e.contains("not available while connected") || e.contains("unavailable"), "{id}: {e}");
    }
    assert_eq!(s.user, "riley");
}

#[test]
fn after_command_observer_sees_local_changes_only() {
    let seen: Arc<Mutex<Vec<Executed>>> = Arc::default();
    let mut s = Session::new(base(), "riley").with_clock(moving_now);
    let sink = Arc::clone(&seen);
    s.set_after_command(Some(Box::new(move |e| sink.lock().unwrap().push(e.clone()))));
    let ids = ids_next(&s.ws);
    s.run("issue.comment", &json!({"key": "LUME-1", "body": "hi"})).unwrap();
    s.run("issue.get", &json!({"key": "LUME-1"})).unwrap();
    let _ = s.run("issue.edit", &json!({"key": "LUME-999", "summary": "x"}));
    let same = s.run("issue.get", &json!({"key": "LUME-1"})).unwrap()["fields"]["summary"].clone();
    let _ = s.run("issue.edit", &json!({"key": "LUME-1", "summary": same}));
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1, "queries, failures and no-ops are not reported: {got:?}");
    assert_eq!((got[0].command.as_str(), got[0].actor.as_str(), got[0].ids_before), ("issue.comment", "riley", ids));
    // The comment's timestamp is the pinned `at`.
    assert!(s.ws.issues.values().flat_map(|i| &i.comments).any(|c| c.created == got[0].at && c.updated == got[0].at));
    // Replays never call the observer.
    let e = Entry {
        rev: 1,
        command: "issue.comment".into(),
        params: json!({"key": "LUME-1", "body": "remote"}),
        actor: "sam".into(),
        at: got[0].at,
        ids_before: ids_next(&s.ws),
        origin: None,
    };
    s.apply_remote(&e).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn journal_bounds_since_and_rollback() {
    let mut s = Session::new(base(), "riley");
    s.enable_journal(Journal::new(3, 64 << 20));
    for i in 0..5 {
        s.run("issue.comment", &json!({"key": "LUME-1", "body": format!("c{i}")})).unwrap();
    }
    assert_eq!(s.sync_rev(), 5);
    assert_eq!(s.journal().unwrap().len(), 3);
    assert_eq!(s.journal_since(1, 10), Err(SinceError::TooOld { oldest: 2 }));
    assert_eq!(s.journal_since(2, 10).unwrap().iter().map(|e| e.rev).collect::<Vec<_>>(), vec![3, 4, 5]);
    assert_eq!(s.journal_since(4, 1).unwrap().len(), 1);
    assert_eq!(s.journal_since(5, 10).unwrap(), vec![]);
    assert_eq!(s.journal_since(9, 10), Err(SinceError::Future { rev: 5 }));
    let (ws, mark) = (s.ws.clone(), s.sync_rev());
    s.run("issue.comment", &json!({"key": "LUME-1", "body": "rolled back"})).unwrap();
    s.rollback_to(ws.clone(), mark);
    assert!(s.ws == ws);
    assert_eq!((s.sync_rev(), s.journal().unwrap().last().map(|e| e.rev)), (5, Some(5)));
    s.run("issue.comment", &json!({"key": "LUME-1", "body": "next"})).unwrap();
    assert_eq!(s.sync_rev(), 6);
    // A byte budget evicts too, but always keeps the newest entry.
    let mut t = Session::new(base(), "riley");
    t.enable_journal(Journal::new(100, 10));
    t.run("issue.comment", &json!({"key": "LUME-1", "body": "x".repeat(1000)})).unwrap();
    t.run("issue.comment", &json!({"key": "LUME-1", "body": "y"})).unwrap();
    assert_eq!(t.journal().unwrap().len(), 1);
    // Without a journal nothing is recorded and the sync revision stays put.
    let mut u = Session::new(base(), "riley");
    u.run("issue.comment", &json!({"key": "LUME-1", "body": "z"})).unwrap();
    assert_eq!(u.sync_rev(), 0);
    assert!(u.journal_since(0, 1).is_err());
}

#[test]
fn origins_are_stamped_and_tracked() {
    use crate::sync::proto::Origin;
    let mut s = server();
    s.set_origin(Some(Origin { client: "c1".into(), seq: 4 }));
    s.run("issue.comment", &json!({"key": "LUME-1", "body": "from c1"})).unwrap();
    s.set_origin(None);
    s.run("issue.comment", &json!({"key": "LUME-1", "body": "no origin"})).unwrap();
    let es = s.journal_since(0, 10).unwrap();
    assert_eq!(es[0].origin.as_ref().map(|o| o.seq), Some(4));
    assert_eq!(es[1].origin, None);
    assert_eq!(s.journal().unwrap().applied_seq("c1"), Some(4));
    assert_eq!(s.journal().unwrap().applied_seq("c2"), None);
}
