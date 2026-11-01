//! M4.5: inbox and watch commands, hostile params, and deterministic replay of notifications.

use serde_json::{Value, json};

use crate::sync::Journal;
use crate::{Session, catalog, command};

fn fixed_now() -> trackcraft_model::Timestamp {
    chrono::DateTime::from_timestamp(1_793_448_000, 0).unwrap_or_default()
}

fn base() -> trackcraft_model::Workspace {
    crate::sample::workspace(fixed_now())
}

fn session(user: &str) -> Session {
    Session::new(base(), user).with_clock(fixed_now)
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.run(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn unread(s: &mut Session) -> u64 {
    run(s, "inbox.list", json!({}))["unread"].as_u64().unwrap()
}

#[test]
fn catalog_floor_with_notifications() {
    for id in ["inbox.list", "inbox.markRead", "issue.watch", "issue.unwatch", "page.watch", "page.unwatch", "watchers.list"] {
        assert!(command(id).is_some(), "{id} missing");
    }
    for row in ["Watchers", "Notifications (in-app, email)", "@mentions"] {
        assert!(catalog::FEATURES.iter().any(|f| f.name == row && !f.command.is_empty()), "{row} not mapped");
    }
    let (done, total) = catalog::coverage();
    assert!(done >= 66, "catalog coverage fell to {done}/{total}");
}

#[test]
fn assign_comment_list_and_mark_read() {
    let mut s = session("riley");
    let k = run(&mut s, "issue.create", json!({"project": "LUME", "summary": "Inbox test"}))["key"].as_str().unwrap().to_string();
    s.user = "sam".into();
    let before = unread(&mut s);
    // riley assigns sam: sam hears about it, riley (the actor) doesn't.
    s.user = "riley".into();
    let riley_before = unread(&mut s);
    run(&mut s, "issue.assign", json!({"key": k, "assignee": "sam"}));
    assert_eq!(unread(&mut s), riley_before);
    s.user = "sam".into();
    assert_eq!(unread(&mut s), before + 1);
    let list = run(&mut s, "inbox.list", json!({"unreadOnly": true, "limit": 1}));
    let n = &list["notifications"][0];
    assert_eq!((n["kind"].as_str(), n["actor"].as_str(), n["target"]["key"].as_str()), (Some("assigned"), Some("riley"), Some(k.as_str())));
    assert_eq!(list["notifications"].as_array().unwrap().len(), 1);
    // sam comments mentioning jo: jo is mentioned, riley (watching as reporter) hears of the comment.
    run(&mut s, "issue.comment", json!({"key": k, "body": "@jo can you check?"}));
    s.user = "jo".into();
    let n = run(&mut s, "inbox.list", json!({}))["notifications"][0].clone();
    assert_eq!((n["kind"].as_str(), n["excerpt"].as_str()), (Some("mentioned"), Some("@jo can you check?")));
    s.user = "riley".into();
    assert_eq!(run(&mut s, "inbox.list", json!({}))["notifications"][0]["kind"], "commented");
    // Marking read: by id, then all.
    s.user = "sam".into();
    let id = n_id(&mut s);
    let r = run(&mut s, "inbox.markRead", json!({"ids": [id]}));
    assert_eq!(r["marked"], 1);
    run(&mut s, "inbox.markRead", json!({"all": true}));
    assert_eq!(unread(&mut s), 0);
    assert_eq!(run(&mut s, "inbox.list", json!({"unreadOnly": true}))["notifications"], json!([]));
    // A second mark-all changes nothing (so no undo step either).
    let depth = s.undo_depth();
    run(&mut s, "inbox.markRead", json!({"all": true}));
    assert_eq!(s.undo_depth(), depth);
}

fn n_id(s: &mut Session) -> u64 {
    run(s, "inbox.list", json!({}))["notifications"][0]["id"].as_u64().unwrap()
}

#[test]
fn watch_unwatch_and_watchers() {
    let mut s = session("jo");
    let w = run(&mut s, "watchers.list", json!({"key": "LUME-1"}));
    assert_eq!(w["watching"], false);
    let r = run(&mut s, "issue.watch", json!({"key": "lume-1"}));
    assert_eq!((r["changed"].as_bool(), r["watching"].as_bool()), (Some(true), Some(true)));
    assert_eq!(run(&mut s, "issue.watch", json!({"key": "LUME-1"}))["changed"], false);
    assert!(run(&mut s, "watchers.list", json!({"key": "LUME-1"}))["watchers"].as_array().unwrap().contains(&json!("jo")));
    // Watching means hearing of status changes made by others.
    s.user = "sam".into();
    let to = if run(&mut s, "issue.get", json!({"key": "LUME-1"}))["status"]["name"] == "In Review" { "To Do" } else { "In Review" };
    run(&mut s, "issue.transition", json!({"key": "LUME-1", "to": to}));
    s.user = "jo".into();
    let n = run(&mut s, "inbox.list", json!({}))["notifications"][0].clone();
    assert_eq!((n["kind"].as_str(), n["excerpt"].as_str()), (Some("statusChanged"), Some(to)));
    assert_eq!(run(&mut s, "issue.unwatch", json!({"key": "LUME-1"}))["watching"], false);
    // Pages.
    let page = *s.ws.pages.keys().next().unwrap();
    run(&mut s, "page.watch", json!({"id": page.0}));
    assert_eq!(run(&mut s, "watchers.list", json!({"id": page.0.to_string()}))["watching"], true);
    run(&mut s, "page.unwatch", json!({"id": page.0}));
    assert_eq!(run(&mut s, "watchers.list", json!({"id": page.0}))["watching"], false);
}

#[test]
fn hostile_params() {
    let junk = [
        Value::Null,
        json!([]),
        json!("x"),
        json!({"ids": "all"}),
        json!({"ids": [-1, "x", null]}),
        json!({"ids": vec![1; 10_000]}),
        json!({"ids": [u64::MAX], "all": true}),
        json!({"all": "yes"}),
        json!({"all": false}),
        json!({"unreadOnly": 1, "limit": -5}),
        json!({"unreadOnly": true, "limit": u64::MAX}),
        json!({"key": "LUME-0"}),
        json!({"key": "X".repeat(100_000)}),
        json!({"key": 7}),
        json!({"key": "LUME-1", "id": 1}),
        json!({"id": u64::MAX}),
        json!({"id": "nope"}),
        json!({"id": {}}),
    ];
    for id in ["inbox.list", "inbox.markRead", "issue.watch", "issue.unwatch", "page.watch", "page.unwatch", "watchers.list"] {
        for p in &junk {
            let mut s = session("riley");
            let before = s.ws.clone();
            if s.run(id, p).is_err() {
                assert!(s.ws == before, "{id} {p}: a failed command changed the workspace");
            }
        }
    }
    let mut s = session("riley");
    assert!(s.run("inbox.list", &json!({"limit": u64::MAX})).is_ok());
    assert!(s.run("inbox.markRead", &json!({})).is_err());
    assert!(s.run("watchers.list", &json!({})).is_err());
}

/// Two sessions replaying the same journal end with identical inboxes (and workspaces).
#[test]
fn replay_produces_identical_inboxes() {
    let mut a = Session::new(base(), "riley").with_clock(fixed_now);
    a.enable_journal(Journal::new(10_000, 64 << 20));
    let home = a.ws.spaces.values().next().unwrap().homepage.0;
    let script: &[(&str, &str, Value)] = &[
        ("riley", "issue.create", json!({"project": "LUME", "summary": "Sync me", "assignee": "sam", "description": "cc @jo"})),
        ("sam", "issue.comment", json!({"key": "LUME-8", "body": "on it @riley"})),
        ("jo", "issue.watch", json!({"key": "LUME-8"})),
        ("sam", "issue.transition", json!({"key": "LUME-8", "to": "Done"})),
        ("riley", "issue.assign", json!({"key": "LUME-8", "assignee": "jo"})),
        ("jo", "page.create", json!({"space": "ENG", "title": "Synced page", "body": "for @sam"})),
        ("riley", "page.comment", json!({"id": home, "body": "nice"})),
        ("sam", "inbox.markRead", json!({"all": true})),
        ("jo", "issue.unwatch", json!({"key": "LUME-8"})),
    ];
    for (who, id, p) in script {
        a.user = (*who).to_string();
        a.run(id, p).unwrap_or_else(|e| panic!("{who} {id}: {e}"));
    }
    assert!(!a.ws.inbox.is_empty());
    let entries = a.journal_since(0, usize::MAX).unwrap();
    let mut replicas = [Session::new(base(), "sam"), Session::new(base(), "jo")];
    for b in &mut replicas {
        b.set_remote(true);
        for e in &entries {
            b.apply_remote(e).unwrap_or_else(|err| panic!("rev {}: {err}", e.rev));
        }
        assert_eq!(b.ws.inbox, a.ws.inbox);
        assert!(b.ws == a.ws);
    }
    // And through JSON (what the server's snapshot sends).
    let back: trackcraft_model::Workspace = serde_json::from_str(&serde_json::to_string(&a.ws).unwrap()).unwrap();
    assert_eq!(back.inbox, a.ws.inbox);
}
