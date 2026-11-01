use serde_json::{Value, json};

use crate::{Session, catalog, command, commands};

fn fixed_now() -> trackcraft_model::Timestamp {
    chrono::DateTime::from_timestamp(1_793_448_000, 0).unwrap_or_default() // 2026-10-31T12:00:00Z
}

fn session() -> Session {
    let ws = crate::sample::workspace(fixed_now());
    Session::new(ws, "riley").with_clock(fixed_now)
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.run(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn command_ids_are_unique_and_well_formed() {
    let mut seen = std::collections::HashSet::new();
    for c in commands() {
        assert!(seen.insert(c.id), "duplicate command id {}", c.id);
        assert!(c.id.contains('.') && c.id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '.'), "bad id {}", c.id);
        assert!(!c.label.is_empty() && !c.location.is_empty());
    }
}

#[test]
fn catalog_ids_exist_and_floor_holds() {
    for f in catalog::FEATURES {
        assert!(f.command.is_empty() || command(f.command).is_some(), "catalog names missing command {}", f.command);
    }
    let (done, total) = catalog::coverage();
    // The floor only rises: raise it when you implement catalog rows.
    assert!(done >= 49, "catalog coverage fell to {done}/{total}");
}

/// Every command, fed junk, returns an error or a value — never a panic, and a failed command
/// never changes the workspace.
#[test]
fn hostile_params_never_panic() {
    let junk = [
        Value::Null,
        json!({}),
        json!([]),
        json!("x"),
        json!(42),
        json!({"key": 5, "id": "nope", "project": [], "summary": {}, "jql": 7, "to": null}),
        json!({"key": "LUME-1", "id": u64::MAX, "parent": -1, "version": u64::MAX, "startAt": u64::MAX, "maxResults": u64::MAX}),
        json!({"key": "X".repeat(100_000), "summary": "s".repeat(100_000), "jql": "(".repeat(10_000), "text": "é".repeat(10_000)}),
        json!({"key": "LUME-1", "to": "Done", "summary": "\u{0}", "labels": [1, 2], "storyPoints": 1e308, "body": {"type": "doc", "content": 3}}),
        json!({"project": "LUME", "summary": "ok", "type": "Sub-task", "parent": "LUME-999999999999999999999"}),
        json!({"space": "ENG", "title": "t", "parent": "2", "id": 2, "body": {"type": "nope"}}),
    ];
    for c in commands() {
        if c.id == "file.open" || c.id == "file.save" {
            continue; // touch the file system; covered by store tests
        }
        for p in &junk {
            let mut s = session();
            let before = s.ws.clone();
            match s.run(c.id, p) {
                Ok(_) => {}
                Err(e) => {
                    assert!(!e.starts_with("internal error"), "{} panicked on {p}: {e}", c.id);
                    assert!(s.ws == before, "{} changed the workspace on error ({e})", c.id);
                }
            }
        }
    }
}

#[test]
fn issue_lifecycle_with_undo() {
    let mut s = session();
    let k = run(&mut s, "issue.create", json!({"project": "LUME", "summary": "Add widgets", "type": "Story", "labels": "ui widgets"}));
    let key = k["key"].as_str().unwrap_or_default().to_string();
    run(&mut s, "issue.transition", json!({"key": key, "to": "In Progress"}));
    run(&mut s, "issue.comment", json!({"key": key, "body": "Started"}));
    assert_eq!(run(&mut s, "issue.get", json!({"key": key}))["status"]["name"], "In Progress");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "issue.get", json!({"key": key}))["status"]["name"], "To Do");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(run(&mut s, "issue.get", json!({"key": key}))["status"]["statusCategory"], "In Progress");
    // A query command never adds an undo step.
    run(&mut s, "search.jql", json!({"jql": ""}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.run("issue.get", &json!({"key": key})).is_err());
    assert!(s.run("edit.undo", &json!({})).is_err(), "nothing left to undo");
}

#[test]
fn search_and_board() {
    let mut s = session();
    let r = run(&mut s, "search.jql", json!({"jql": "project = LUME AND statusCategory != Done ORDER BY priority, key"}));
    let keys: Vec<&str> = r["issues"].as_array().map(|a| a.iter().filter_map(|i| i["key"].as_str()).collect()).unwrap_or_default();
    assert_eq!(keys.first().copied(), Some("LUME-4"), "Highest first: {keys:?}");
    assert!(!keys.contains(&"LUME-5"), "Done excluded");
    let mine = run(&mut s, "search.jql", json!({"jql": "assignee = currentUser()"}));
    assert_eq!(mine["total"], 1);
    assert!(s.run("search.jql", &json!({"jql": "status = "})).is_err());
    let paged = run(&mut s, "search.jql", json!({"jql": "", "startAt": 2, "maxResults": 3}));
    assert_eq!(paged["issues"].as_array().map(Vec::len), Some(3));

    let b = run(&mut s, "board.view", json!({"project": "lume"}));
    let cols = b["columns"].as_array().cloned().unwrap_or_default();
    assert_eq!(cols.len(), 4);
    assert_eq!(cols[2]["status"], "In Review");
    assert_eq!(cols[2]["issues"][0]["key"], "LUME-4");
}

#[test]
fn wiki_flow() {
    let mut s = session();
    let tree = run(&mut s, "space.tree", json!({"key": "ENG"}));
    assert_eq!(tree["children"][0]["title"], "Architecture overview");
    assert_eq!(tree["children"][0]["children"].as_array().map(Vec::len), Some(2));
    let id = run(&mut s, "page.create", json!({"space": "ENG", "title": "Runbooks", "body": {"type": "doc", "version": 1, "content": [{"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "On call"}]}]}}))["id"].clone();
    assert_eq!(run(&mut s, "page.get", json!({"id": id}))["body"], "# On call");
    run(&mut s, "page.edit", json!({"id": id, "body": "Updated", "baseVersion": 1, "message": "tidy"}));
    assert!(s.run("page.edit", &json!({"id": id, "body": "Lost update", "baseVersion": 1})).is_err());
    let hist = run(&mut s, "page.history", json!({"id": id}));
    assert_eq!(hist[0]["number"], 2);
    assert_eq!(hist[0]["message"], "tidy");
    run(&mut s, "page.restore", json!({"id": id, "version": 1}));
    assert_eq!(run(&mut s, "page.get", json!({"id": id}))["version"]["number"], 3);
    let found = run(&mut s, "page.search", json!({"text": "operation log"}));
    assert_eq!(found[0]["title"], "Sync protocol");
}

#[test]
fn save_and_open_round_trip() {
    let dir = std::env::temp_dir().join(format!("trackcraft-engine-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample.track");
    let mut s = session();
    run(&mut s, "file.save", json!({"path": path.display().to_string()}));
    let mut t = Session::new(trackcraft_model::Workspace::default(), "nobody");
    run(&mut t, "file.open", json!({"path": path.display().to_string()}));
    assert!(t.ws == s.ws);
    assert_eq!(t.user, "jo", "falls back to the first user");
    std::fs::remove_dir_all(&dir).unwrap();
}
