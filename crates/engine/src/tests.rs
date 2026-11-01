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
    assert!(done >= 52, "catalog coverage fell to {done}/{total}");
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
        json!({"key": ["LUME-1"]}),
        json!({"key": "LUME-0"}),
        json!({"key": "lume-1 "}),
        json!({"key": "LUME-18446744073709551616"}),
        json!({"jql": "status WAS IN (" }),
        json!({"jql": format!("status CHANGED {}", "BY x ".repeat(5_000))}),
        json!({"jql": format!("status WAS IN ({})", vec!["x"; 20_000].join(","))}),
        json!({"jql": "labels CHANGED AFTER yesterday"}),
        json!({"jql": "status CHANGED DURING (now(), nope())"}),
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

fn history(s: &mut Session, key: &str) -> Vec<Value> {
    run(s, "issue.history", json!({"key": key})).as_array().cloned().unwrap_or_default()
}

/// `(field, fromString, toString)` of each item of one history group.
fn items(group: &Value) -> Vec<(String, Value, Value)> {
    group["items"]
        .as_array()
        .map(|a| a.iter().map(|i| (i["field"].as_str().unwrap_or_default().to_string(), i["fromString"].clone(), i["toString"].clone())).collect())
        .unwrap_or_default()
}

#[test]
fn every_command_records_history() {
    let mut s = session();
    let key = run(&mut s, "issue.create", json!({"project": "LUME", "summary": "Track it"}))["key"].as_str().unwrap_or_default().to_string();
    let h = history(&mut s, &key);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0]["author"], "riley");
    assert_eq!(h[0]["created"], "2026-10-31T12:00:00+00:00");
    assert_eq!(items(&h[0])[0].0, "created");

    // One command, several fields: one group.
    run(
        &mut s,
        "issue.edit",
        json!({"key": key, "summary": "Track it all", "priority": "High", "labels": ["a"], "storyPoints": 2, "description": "Why"}),
    );
    let h = history(&mut s, &key);
    assert_eq!(h.len(), 2);
    let fields: Vec<String> = items(&h[0]).into_iter().map(|i| i.0).collect();
    assert_eq!(fields, ["summary", "description", "priority", "labels", "storyPoints"]);
    assert_eq!(items(&h[0])[0], ("summary".into(), json!("Track it"), json!("Track it all")));

    run(&mut s, "issue.assign", json!({"key": key, "assignee": "sam"}));
    assert_eq!(items(&history(&mut s, &key)[0]), [("assignee".into(), Value::Null, json!("sam"))]);
    run(&mut s, "issue.assignToMe", json!({"key": key}));
    assert_eq!(items(&history(&mut s, &key)[0]), [("assignee".into(), json!("sam"), json!("riley"))]);
    run(&mut s, "issue.transition", json!({"key": key, "to": "Done"}));
    assert_eq!(
        items(&history(&mut s, &key)[0]),
        [("status".into(), json!("To Do"), json!("Done")), ("resolution".into(), Value::Null, json!("Done"))]
    );
    let c = run(&mut s, "issue.comment", json!({"key": key, "body": "Shipped"}))["id"].clone();
    assert_eq!(items(&history(&mut s, &key)[0]), [("comment".into(), Value::Null, json!("Shipped"))]);
    run(&mut s, "issue.deleteComment", json!({"key": key, "id": c}));
    assert_eq!(items(&history(&mut s, &key)[0]), [("comment".into(), json!("Shipped"), Value::Null)]);
    run(&mut s, "issue.link", json!({"key": key, "type": "relates to", "target": "LUME-1"}));
    assert_eq!(items(&history(&mut s, &key)[0]), [("link".into(), Value::Null, json!("This issue relates to LUME-1"))]);

    let h = history(&mut s, &key);
    assert_eq!(h.len(), 8);
    assert_eq!(run(&mut s, "issue.get", json!({"key": key}))["history"], 8);
    let ids: std::collections::HashSet<u64> = h.iter().filter_map(|g| g["id"].as_u64()).collect();
    assert_eq!(ids.len(), 8, "one id per command");

    // A failed command records nothing; a query records nothing.
    assert!(s.run("issue.transition", &json!({"key": key, "to": "Nowhere"})).is_err());
    run(&mut s, "issue.get", json!({"key": key}));
    assert_eq!(history(&mut s, &key).len(), 8);
    assert!(s.run("issue.history", &json!({"key": "LUME-999"})).is_err());
}

#[test]
fn undo_removes_the_history_entry_with_the_change() {
    let mut s = session();
    let before = history(&mut s, "LUME-3");
    run(&mut s, "issue.transition", json!({"key": "LUME-3", "to": "In Progress"}));
    assert_eq!(history(&mut s, "LUME-3").len(), before.len() + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(history(&mut s, "LUME-3"), before);
    let r = run(&mut s, "search.jql", json!({"jql": "key = LUME-3 AND status CHANGED"}));
    assert_eq!(r["total"], 0);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(history(&mut s, "LUME-3").len(), before.len() + 1);
    assert_eq!(run(&mut s, "search.jql", json!({"jql": "key = LUME-3 AND status CHANGED BY riley"}))["total"], 1);
}

#[test]
fn jql_history_operators_on_the_sample() {
    let mut s = session();
    let keys = |s: &mut Session, jql: &str| -> Vec<String> {
        let r = run(s, "search.jql", json!({"jql": format!("{jql} ORDER BY key")}));
        r["issues"].as_array().map(|a| a.iter().filter_map(|i| i["key"].as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    assert_eq!(keys(&mut s, r#"status WAS "In Progress""#), ["LUME-2", "LUME-4", "LUME-5"]);
    assert_eq!(keys(&mut s, r#"status WAS NOT "In Progress" AND project = LUME"#), ["LUME-1", "LUME-3", "LUME-6", "LUME-7"]);
    assert_eq!(keys(&mut s, r#"status WAS IN ("In Review", Done)"#), ["LUME-4", "LUME-5", "OPS-2"]);
    assert_eq!(keys(&mut s, r#"status WAS NOT IN ("To Do") "#), [] as [&str; 0]);
    assert_eq!(keys(&mut s, r#"status CHANGED FROM "To Do" TO "In Progress""#), ["LUME-2", "LUME-4", "LUME-5"]);
    assert_eq!(keys(&mut s, "status CHANGED TO Done BY sam"), ["LUME-5"]);
    assert_eq!(keys(&mut s, r#"status WAS "In Progress" DURING (-25d, -20d)"#), ["LUME-5"]);
    assert_eq!(keys(&mut s, "status CHANGED AFTER -5d"), ["LUME-4"]);
    assert_eq!(keys(&mut s, "status CHANGED ON 2026-10-28"), ["LUME-4"]);
    assert_eq!(keys(&mut s, "assignee WAS jo AND NOT assignee CHANGED AND project = OPS"), ["OPS-1", "OPS-2"]);
    assert_eq!(keys(&mut s, "priority WAS Highest"), ["LUME-4"]);
    let e = s.run("search.jql", &json!({"jql": "labels WAS ui"})).unwrap_err();
    assert!(e.contains("not supported for field 'labels'"), "{e}");
    assert!(s.run("search.jql", &json!({"jql": "status CHANGED AFTER someday"})).is_err());
}
