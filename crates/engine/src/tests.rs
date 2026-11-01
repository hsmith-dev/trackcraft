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
    assert!(done >= 63, "catalog coverage fell to {done}/{total}");
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
        json!({"keys": vec!["LUME-1"; 5_000], "before": "LUME-2", "sprint": null}),
        json!({"keys": "LUME-1 ".repeat(5_000), "after": "LUME-2"}),
        json!({"keys": ["LUME-1"], "before": "LUME-1", "after": "LUME-2", "sprint": "active"}),
        json!({"keys": ["LUME-2", "OPS-1"], "before": "LUME-3", "sprint": u64::MAX}),
        json!({"keys": [1, {}], "before": 3, "sprint": []}),
        json!({"keys": ["LUME-7"], "sprint": "x", "after": "LUME-99999999999999999999"}),
        json!({"id": "nope", "moveOpenTo": {}, "startDate": "2026-13-45", "endDate": 5}),
        json!({"id": u64::MAX, "moveOpenTo": "new", "startDate": "2026-11-03", "endDate": "2026-11-01"}),
        json!({"project": "LUME", "sprint": "active", "name": "\u{0}", "goal": "é".repeat(5_000), "startDate": "+999999-01-01T00:00:00Z"}),
        json!({"project": "OPS", "sprint": "active", "jql": "sprint IN openSprints("}),
        json!({"project": "LUME", "jql": "sprint IN nope() ORDER BY rank"}),
        // M2.3 params: workspace.rename, user.edit, issue.unlink/editComment, issue.edit type/parent,
        // issue.create sprint, sprint.start name, sprint.edit null dates.
        json!({"name": "\u{0}\n", "id": "riley", "email": 7}),
        json!({"name": "n".repeat(100_000), "id": "riley", "email": format!("{}@x.org", "a".repeat(100_000))}),
        json!({"id": "riley", "email": "not an email", "name": []}),
        json!({"id": ["riley"], "email": null, "name": null}),
        // M5.1 workspace.import params.
        json!({"kind": "jiraCsv", "path": "/nonexistent/trackcraft/export.csv"}),
        json!({"kind": "confluence", "path": "", "space": "bad key"}),
        json!({"kind": ["jiraJson"], "path": 7, "space": 3}),
        json!({"kind": "nope", "path": "x"}),
        json!({"key": "LUME-6", "type": "blocks", "target": "LUME-2"}),
        json!({"key": "LUME-2", "type": "nope", "target": "LUME-6"}),
        json!({"key": "LUME-2", "type": 5, "target": ["LUME-6"]}),
        json!({"key": "LUME-4", "id": 0, "body": "x"}),
        json!({"key": "LUME-4", "id": "-1", "body": {"type": "doc", "content": [{"type": "paragraph"}]}}),
        json!({"key": "LUME-7", "type": "Task", "parent": "LUME-7"}),
        json!({"key": "LUME-2", "type": "Sub-task", "parent": "LUME-1"}),
        json!({"key": "LUME-1", "type": "Epic", "parent": "LUME-2"}),
        json!({"key": "LUME-2", "parent": {"key": "LUME-1"}, "issuetype": 3}),
        json!({"key": "LUME-1", "parent": "LUME-7", "type": "x".repeat(10_000)}),
        json!({"key": "LUME-3", "parent": null, "type": null}),
        json!({"project": "LUME", "summary": "s", "sprint": "nope"}),
        json!({"project": "LUME", "summary": "s", "sprint": u64::MAX}),
        json!({"project": "LUME", "summary": "s", "type": "Sub-task", "parent": "LUME-2", "sprint": 1}),
        json!({"project": "OPS", "summary": "s", "sprint": -3}),
        json!({"id": 1, "startDate": null, "endDate": null, "name": "\u{0}"}),
        json!({"id": "1", "startDate": null, "endDate": "2026-11-01", "name": "", "moveOpenTo": "backlog"}),
        json!({"id": 1, "startDate": "2026-11-03", "endDate": "2026-11-17", "name": "x".repeat(1_000)}),
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

// ---- backlog, rank, sprints --------------------------------------------------------------------

fn keys_of(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|i| i["key"].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn sprint_ids(s: &mut Session) -> (u64, u64) {
    let l = run(s, "sprint.list", json!({"project": "LUME"}));
    (l[0]["id"].as_u64().unwrap_or_default(), l[1]["id"].as_u64().unwrap_or_default())
}

#[test]
fn backlog_on_the_sample() {
    let mut s = session();
    let b = run(&mut s, "board.backlog", json!({"project": "lume"}));
    let sprints = b["sprints"].as_array().cloned().unwrap_or_default();
    assert_eq!(sprints.len(), 2);
    assert_eq!(sprints[0]["state"], "active");
    assert_eq!(sprints[0]["name"], "LUME Sprint 1");
    assert_eq!(sprints[0]["startDate"], "2026-10-26T12:00:00+00:00");
    assert_eq!(sprints[0]["endDate"], "2026-11-09T12:00:00+00:00");
    assert_eq!(keys_of(&sprints[0]["issues"]), ["LUME-4", "LUME-2", "LUME-3"], "rank order, sub-task LUME-7 left out");
    assert_eq!(sprints[0]["points"], 8.0);
    assert_eq!(sprints[0]["counts"], json!({"todo": 1, "inProgress": 2, "done": 0}));
    assert_eq!(sprints[1]["state"], "future");
    assert_eq!(keys_of(&sprints[1]["issues"]), [] as [&str; 0]);
    // Epic LUME-1 and done LUME-5 are not backlog rows.
    assert_eq!(keys_of(&b["backlog"]), ["LUME-6"]);
    let mine = run(&mut s, "board.backlog", json!({"project": "LUME", "jql": "assignee = jo"}));
    assert_eq!(keys_of(&mine["sprints"][0]["issues"]), ["LUME-4"]);
    assert!(s.run("board.backlog", &json!({"project": "LUME", "jql": "status = "})).is_err());
    let ops = run(&mut s, "board.backlog", json!({"project": "OPS"}));
    assert_eq!(ops["sprints"], json!([]));
    assert_eq!(keys_of(&ops["backlog"]), ["OPS-1"]);
    assert!(!s.can_undo(), "queries add no undo step");
}

#[test]
fn rank_is_one_undo_step_and_validated() {
    let mut s = session();
    let order = |s: &mut Session| keys_of(&run(s, "board.backlog", json!({"project": "LUME"}))["sprints"][0]["issues"]);
    run(&mut s, "issue.rank", json!({"keys": ["LUME-3", "LUME-2"], "before": "LUME-4"}));
    assert_eq!(order(&mut s), ["LUME-3", "LUME-2", "LUME-4"]);
    run(&mut s, "issue.rank", json!({"keys": "LUME-3", "after": "LUME-4"}));
    assert_eq!(order(&mut s), ["LUME-2", "LUME-4", "LUME-3"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(order(&mut s), ["LUME-3", "LUME-2", "LUME-4"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(order(&mut s), ["LUME-4", "LUME-2", "LUME-3"]);
    assert!(!s.can_undo());
    let before = history(&mut s, "LUME-3");
    run(&mut s, "issue.rank", json!({"keys": ["LUME-3"], "before": "LUME-2"}));
    assert_eq!(history(&mut s, "LUME-3"), before, "rank is not history-worthy");
    for bad in [
        json!({"keys": ["LUME-3"]}),
        json!({"keys": ["LUME-3"], "before": "LUME-3"}),
        json!({"keys": ["LUME-3"], "before": "LUME-2", "after": "LUME-4"}),
        json!({"keys": ["LUME-3"], "before": "OPS-1"}),
        json!({"keys": [], "before": "LUME-2"}),
        json!({"keys": vec!["LUME-3"; 1001], "before": "LUME-2"}),
    ] {
        assert!(s.run("issue.rank", &bad).is_err(), "{bad}");
    }
    // Board cards follow rank within columns.
    run(&mut s, "issue.rank", json!({"keys": ["LUME-6"], "before": "LUME-3"}));
    let b = run(&mut s, "board.view", json!({"project": "LUME"}));
    assert_eq!(keys_of(&b["columns"][0]["issues"]), ["LUME-6", "LUME-3", "LUME-7", "LUME-1"]);
}

#[test]
fn set_sprint_moves_ranks_and_records_history() {
    let mut s = session();
    let (active, future) = sprint_ids(&mut s);
    run(&mut s, "issue.setSprint", json!({"keys": ["LUME-6"], "sprint": active, "before": "LUME-2"}));
    let b = run(&mut s, "board.backlog", json!({"project": "LUME"}));
    assert_eq!(keys_of(&b["sprints"][0]["issues"]), ["LUME-4", "LUME-6", "LUME-2", "LUME-3"]);
    assert_eq!(keys_of(&b["backlog"]), [] as [&str; 0]);
    let h = history(&mut s, "LUME-6");
    assert_eq!(items(&h[0]), [("sprint".into(), Value::Null, json!("LUME Sprint 1"))]);
    assert_eq!(run(&mut s, "issue.get", json!({"key": "LUME-6"}))["sprint"]["id"], active);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(keys_of(&run(&mut s, "board.backlog", json!({"project": "LUME"}))["backlog"]), ["LUME-6"], "one undo step");
    // Parents carry their sub-tasks.
    run(&mut s, "issue.setSprint", json!({"keys": ["LUME-2"], "sprint": future.to_string()}));
    assert_eq!(run(&mut s, "issue.get", json!({"key": "LUME-7"}))["sprint"]["id"], future);
    assert_eq!(items(&history(&mut s, "LUME-7")[0]), [("sprint".into(), json!("LUME Sprint 1"), json!("LUME Sprint 2"))]);
    run(&mut s, "issue.setSprint", json!({"keys": ["LUME-2"], "sprint": null}));
    assert_eq!(run(&mut s, "issue.get", json!({"key": "LUME-2"}))["sprint"], Value::Null);
    assert!(s.run("issue.setSprint", &json!({"keys": ["LUME-7"], "sprint": active})).is_err(), "sub-task");
    assert!(s.run("issue.setSprint", &json!({"keys": ["LUME-6"]})).is_err(), "sprint is required");
    assert!(s.run("issue.setSprint", &json!({"keys": ["OPS-1"], "sprint": active})).is_err(), "other project");
    assert!(s.run("issue.setSprint", &json!({"keys": ["LUME-6"], "sprint": 999_999})).is_err());
    let r = run(&mut s, "search.jql", json!({"jql": "sprint IN openSprints() ORDER BY rank"}));
    assert_eq!(keys_of(&r["issues"]), ["LUME-4", "LUME-3"]);
    let r = run(&mut s, "search.jql", json!({"jql": r#"sprint = "LUME Sprint 1" ORDER BY rank"#}));
    assert_eq!(r["total"], 2);
    let r = run(&mut s, "search.jql", json!({"jql": format!("sprint = {active} OR sprint IS EMPTY AND project = LUME ORDER BY rank")}));
    assert_eq!(keys_of(&r["issues"]), ["LUME-4", "LUME-2", "LUME-7", "LUME-3", "LUME-6", "LUME-1", "LUME-5"]);
}

#[test]
fn sprint_create_edit_delete() {
    let mut s = session();
    let c = run(&mut s, "sprint.create", json!({"project": "LUME", "goal": "Later", "startDate": "2026-11-17", "endDate": "2026-12-01T09:00:00Z"}));
    assert_eq!(c["name"], "LUME Sprint 3");
    assert_eq!(c["state"], "future");
    assert_eq!(c["startDate"], "2026-11-17T00:00:00+00:00");
    assert_eq!(c["endDate"], "2026-12-01T09:00:00+00:00");
    let id = c["id"].clone();
    assert!(s.run("sprint.create", &json!({"project": "LUME", "startDate": "2026-11-17", "endDate": "2026-11-10"})).is_err());
    assert!(s.run("sprint.create", &json!({"project": "LUME", "startDate": "tomorrow"})).is_err());
    let e = run(&mut s, "sprint.edit", json!({"id": id, "name": "Polish", "goal": "Shine"}));
    assert_eq!((e["name"].clone(), e["goal"].clone()), (json!("Polish"), json!("Shine")));
    assert!(s.run("sprint.edit", &json!({"id": id})).is_err(), "nothing to change");
    assert!(s.run("sprint.edit", &json!({"id": id, "endDate": "2026-11-01"})).is_err(), "end before start");
    run(&mut s, "issue.setSprint", json!({"keys": ["LUME-6"], "sprint": id}));
    let d = run(&mut s, "sprint.delete", json!({"id": id}));
    assert_eq!(d["backlog"], json!(["LUME-6"]));
    assert_eq!(items(&history(&mut s, "LUME-6")[0]), [("sprint".into(), json!("Polish"), Value::Null)]);
    assert!(s.run("sprint.delete", &json!({"id": id})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "sprint.list", json!({"project": "LUME"})).as_array().map(Vec::len), Some(3));
}

#[test]
fn sprint_start_and_complete() {
    let mut s = session();
    let (active, future) = sprint_ids(&mut s);
    let e = s.run("sprint.start", &json!({"id": future, "startDate": "2026-11-09", "endDate": "2026-11-23"})).unwrap_err();
    assert!(e.contains("already active"), "{e}");
    assert!(s.run("sprint.start", &json!({"id": future})).is_err(), "dates required");
    assert!(s.run("sprint.complete", &json!({"id": active})).is_err(), "moveOpenTo required");
    assert!(s.run("sprint.complete", &json!({"id": future, "moveOpenTo": "backlog"})).is_err(), "not active");
    assert!(s.run("sprint.complete", &json!({"id": active, "moveOpenTo": active})).is_err());

    run(&mut s, "issue.transition", json!({"key": "LUME-3", "to": "Done"}));
    let undo_before = s.ws.clone();
    let c = run(&mut s, "sprint.complete", json!({"id": active, "moveOpenTo": future}));
    assert_eq!(c["done"], json!(["LUME-3"]));
    assert_eq!(c["moved"], json!(["LUME-2", "LUME-4", "LUME-7"]));
    assert_eq!(c["sprint"]["state"], "closed");
    assert_eq!(c["sprint"]["completeDate"], "2026-10-31T12:00:00+00:00");
    assert_eq!(items(&history(&mut s, "LUME-4")[0]), [("sprint".into(), json!("LUME Sprint 1"), json!("LUME Sprint 2"))]);
    let b = run(&mut s, "board.backlog", json!({"project": "LUME"}));
    assert_eq!(b["sprints"].as_array().map(Vec::len), Some(1));
    assert_eq!(keys_of(&b["sprints"][0]["issues"]), ["LUME-4", "LUME-2"]);
    let closed = run(&mut s, "search.jql", json!({"jql": "sprint IN closedSprints() ORDER BY key"}));
    assert_eq!(keys_of(&closed["issues"]), ["LUME-2", "LUME-3", "LUME-4", "LUME-7"]);
    // The closed sprint's board still shows what it held.
    let old = run(&mut s, "board.view", json!({"project": "LUME", "sprint": active}));
    assert_eq!(keys_of(&old["columns"][3]["issues"]), ["LUME-3"]);
    assert!(s.run("board.view", &json!({"project": "LUME", "sprint": "active"})).is_err(), "no active sprint now");
    // One undo step restores the sprint and every issue.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.ws == undo_before);

    // Start the next one after completing into a new sprint.
    let c = run(&mut s, "sprint.complete", json!({"id": active, "moveOpenTo": "new"}));
    let new_id = c["movedTo"].clone();
    assert_eq!(run(&mut s, "sprint.list", json!({"project": "LUME"}))[1]["name"], "LUME Sprint 3");
    let st = run(&mut s, "sprint.start", json!({"id": new_id, "startDate": "2026-11-02", "endDate": "2026-11-16", "goal": "Finish offline"}));
    assert_eq!((st["state"].clone(), st["goal"].clone()), (json!("active"), json!("Finish offline")));
    let board = run(&mut s, "board.view", json!({"project": "LUME", "sprint": "active"}));
    assert_eq!(board["sprint"]["id"], new_id);
    let on_board: Vec<String> = board["columns"].as_array().into_iter().flatten().flat_map(|c| keys_of(&c["issues"])).collect();
    assert_eq!(on_board, ["LUME-7", "LUME-2", "LUME-4"]);
    assert!(s.run("sprint.start", &json!({"id": future, "startDate": "2026-11-02", "endDate": "2026-11-01"})).is_err());
}

#[test]
fn board_view_sprint_filter() {
    let mut s = session();
    let (active, future) = sprint_ids(&mut s);
    let b = run(&mut s, "board.view", json!({"project": "LUME", "sprint": "active"}));
    assert_eq!(b["sprint"]["id"], active);
    let all: Vec<String> = b["columns"].as_array().into_iter().flatten().flat_map(|c| keys_of(&c["issues"])).collect();
    assert_eq!(all, ["LUME-7", "LUME-3", "LUME-2", "LUME-4"]);
    let f = run(&mut s, "board.view", json!({"project": "LUME", "sprint": future}));
    assert!(f["columns"].as_array().into_iter().flatten().all(|c| c["issues"] == json!([])));
    assert_eq!(run(&mut s, "board.view", json!({"project": "LUME"}))["sprint"], Value::Null);
    assert!(s.run("board.view", &json!({"project": "OPS", "sprint": "active"})).is_err());
    assert!(s.run("board.view", &json!({"project": "OPS", "sprint": active})).is_err(), "another project's sprint");
    assert!(s.run("board.view", &json!({"project": "LUME", "sprint": {}})).is_err());
}

// ---- M2.3: sprint fixes, history gaps, new commands, revision -------------------------------

#[test]
fn sprint_complete_refuses_a_done_parent_with_open_subtasks() {
    let mut s = session();
    let (active, _) = sprint_ids(&mut s);
    run(&mut s, "issue.transition", json!({"key": "LUME-2", "to": "Done"}));
    let e = s.run("sprint.complete", &json!({"id": active, "moveOpenTo": "backlog"})).unwrap_err();
    assert!(e.contains("LUME-2 (LUME-7)"), "{e}");
    run(&mut s, "issue.transition", json!({"key": "LUME-7", "to": "Done"}));
    let c = run(&mut s, "sprint.complete", json!({"id": active, "moveOpenTo": "backlog"}));
    assert_eq!(c["done"], json!(["LUME-2", "LUME-7"]));
}

#[test]
fn sprint_start_name_and_edit_clears_dates() {
    let mut s = session();
    let (active, future) = sprint_ids(&mut s);
    run(&mut s, "sprint.edit", json!({"id": future, "startDate": "2026-11-10", "endDate": "2026-11-24"}));
    let e = run(&mut s, "sprint.edit", json!({"id": future, "startDate": null}));
    assert_eq!((e["startDate"].clone(), e["endDate"].clone()), (Value::Null, json!("2026-11-24T00:00:00+00:00")));
    let e = run(&mut s, "sprint.edit", json!({"id": future, "endDate": null}));
    assert_eq!(e["endDate"], Value::Null);
    assert!(s.run("sprint.edit", &json!({"id": active, "endDate": null})).is_err(), "only future sprints clear dates");
    assert!(s.run("sprint.edit", &json!({"id": future, "startDate": 5})).is_err());

    run(&mut s, "issue.transition", json!({"key": "LUME-7", "to": "Done"}));
    run(&mut s, "sprint.complete", json!({"id": active, "moveOpenTo": future}));
    let st = run(&mut s, "sprint.start", json!({"id": future, "name": "Harden", "startDate": "2026-11-02", "endDate": "2026-11-16"}));
    assert_eq!((st["name"].clone(), st["state"].clone()), (json!("Harden"), json!("active")));
}

#[test]
fn issue_create_in_a_sprint_is_one_undo_step() {
    let mut s = session();
    let (active, future) = sprint_ids(&mut s);
    let depth = s.undo_depth();
    let k = run(&mut s, "issue.create", json!({"project": "LUME", "summary": "Inline", "sprint": active}))["key"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert_eq!(run(&mut s, "issue.get", json!({"key": k}))["sprint"]["id"], active);
    assert_eq!(s.undo_depth(), depth + 1);
    assert_eq!(history(&mut s, &k).len(), 1, "created + sprint are one change");
    run(&mut s, "edit.undo", json!({}));
    assert!(s.run("issue.get", &json!({"key": k})).is_err());
    // A sub-task follows its parent: the parent's sprint is accepted, any other refused.
    run(&mut s, "issue.create", json!({"project": "LUME", "summary": "Sub", "type": "Sub-task", "parent": "LUME-2", "sprint": active}));
    let e =
        s.run("issue.create", &json!({"project": "LUME", "summary": "Sub", "type": "Sub-task", "parent": "LUME-2", "sprint": future})).unwrap_err();
    assert!(e.contains("follows its parent"), "{e}");
    assert!(s.run("issue.create", &json!({"project": "OPS", "summary": "x", "sprint": active})).is_err(), "another project's sprint");
    assert!(s.run("issue.create", &json!({"project": "LUME", "summary": "x", "sprint": 999_999})).is_err());
}

#[test]
fn delete_records_history_on_issues_that_lose_links_or_parents() {
    let mut s = session();
    let story = run(&mut s, "issue.create", json!({"project": "LUME", "summary": "Under the epic", "parent": "LUME-1"}))["key"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    run(&mut s, "issue.link", json!({"key": "LUME-6", "type": "relates to", "target": "LUME-1"}));
    run(&mut s, "user.switch", json!({"id": "sam"}));
    let d = run(&mut s, "issue.delete", json!({"key": "LUME-1"}));
    assert_eq!(d["deleted"], json!(["LUME-1"]), "an epic's stories stay");
    let h = history(&mut s, &story);
    assert_eq!(h[0]["author"], "sam");
    assert_eq!(items(&h[0]), [("parent".into(), json!("LUME-1"), Value::Null)]);
    assert_eq!(items(&history(&mut s, "LUME-6")[0]), [("link".into(), json!("This issue relates to LUME-1"), Value::Null)]);
    // Deleting a parent takes its sub-tasks and records the lost link on the linker.
    let d = run(&mut s, "issue.delete", json!({"key": "LUME-2"}));
    assert_eq!(d["deleted"], json!(["LUME-7", "LUME-2"]));
    assert_eq!(items(&history(&mut s, "LUME-6")[0]), [("link".into(), json!("This issue blocks LUME-2"), Value::Null)]);
}

#[test]
fn unlink_and_edit_comment() {
    let mut s = session();
    let e = s.run("issue.unlink", &json!({"key": "LUME-6", "type": "relates to", "target": "LUME-2"})).unwrap_err();
    assert!(e.contains("no 'relates to' link"), "{e}");
    // From the inward side, as the issue view of LUME-2 shows it.
    run(&mut s, "issue.unlink", json!({"key": "LUME-2", "type": "blocks", "target": "LUME-6"}));
    assert_eq!(run(&mut s, "issue.get", json!({"key": "LUME-6"}))["links"], json!([]));
    assert_eq!(items(&history(&mut s, "LUME-6")[0]), [("link".into(), json!("This issue blocks LUME-2"), Value::Null)]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "issue.get", json!({"key": "LUME-6"}))["links"][0]["key"], "LUME-2");

    let comments = run(&mut s, "issue.get", json!({"key": "LUME-4"}))["comments"].clone();
    let (mine, jos) = (comments[0]["id"].clone(), comments[1]["id"].clone());
    let e = s.run("issue.editComment", &json!({"key": "LUME-4", "id": jos, "body": "Mine now"})).unwrap_err();
    assert!(e.contains("only jo"), "{e}");
    assert!(s.run("issue.editComment", &json!({"key": "LUME-4", "id": mine, "body": ""})).is_err());
    assert!(s.run("issue.editComment", &json!({"key": "LUME-4", "id": mine})).is_err());
    run(&mut s, "issue.editComment", json!({"key": "LUME-4", "id": mine, "body": "Reproduced on **every** build."}));
    let c = run(&mut s, "issue.get", json!({"key": "LUME-4"}))["comments"][0].clone();
    assert_eq!(c["body"], "Reproduced on **every** build.");
    assert_eq!(c["updated"], "2026-10-31T12:00:00+00:00");
    let h = history(&mut s, "LUME-4");
    assert_eq!(
        items(&h[0]),
        [("comment".into(), json!("Reproduced on the latest build. Contrast ratio is about 2:1."), json!("Reproduced on every build."))]
    );
}

#[test]
fn issue_edit_type_and_parent() {
    let mut s = session();
    let e = s.run("issue.edit", &json!({"key": "LUME-6", "type": "Saga"})).unwrap_err();
    assert!(e.contains("no issue type"), "{e}");
    assert!(s.run("issue.edit", &json!({"key": "LUME-6", "type": "Sub-task"})).is_err(), "needs a parent");
    assert!(s.run("issue.edit", &json!({"key": "LUME-6", "parent": "LUME-7"})).is_err(), "a sub-task can't be a parent");
    assert!(s.run("issue.edit", &json!({"key": "LUME-1", "parent": "LUME-6"})).is_err(), "an epic has no parent");
    assert!(s.run("issue.edit", &json!({"key": "LUME-6", "parent": 3})).is_err());

    let r = run(&mut s, "issue.edit", json!({"key": "LUME-6", "parent": "LUME-1"}));
    assert_eq!(r["changed"], json!(["parent"]));
    assert!(s.run("issue.edit", &json!({"key": "LUME-1", "type": "Task", "parent": "LUME-6"})).is_err(), "cycle");
    // Story → sub-task of LUME-2: takes LUME-2's sprint; history has type, parent and sprint.
    run(&mut s, "issue.edit", json!({"key": "LUME-6", "type": "Sub-task", "parent": "LUME-2"}));
    let i = run(&mut s, "issue.get", json!({"key": "LUME-6"}));
    assert_eq!(
        (i["issuetype"].clone(), i["parent"].clone(), i["sprint"]["name"].clone()),
        (json!("Sub-task"), json!("LUME-2"), json!("LUME Sprint 1"))
    );
    assert_eq!(
        items(&history(&mut s, "LUME-6")[0]),
        [
            ("issuetype".into(), json!("Bug"), json!("Sub-task")),
            ("parent".into(), json!("LUME-1"), json!("LUME-2")),
            ("sprint".into(), Value::Null, json!("LUME Sprint 1")),
        ]
    );
    // Sub-task → Bug clears the parent and keeps the sprint.
    run(&mut s, "issue.edit", json!({"key": "LUME-7", "type": "Bug"}));
    let i = run(&mut s, "issue.get", json!({"key": "LUME-7"}));
    assert_eq!((i["parent"].clone(), i["sprint"]["name"].clone()), (Value::Null, json!("LUME Sprint 1")));
    run(&mut s, "issue.edit", json!({"key": "LUME-7", "parent": null}));
    assert!(s.run("issue.edit", &json!({"key": "LUME-7"})).is_err(), "nothing to change");
    let r = run(&mut s, "search.jql", json!({"jql": "parent = LUME-2"}));
    assert_eq!(keys_of(&r["issues"]), ["LUME-6"]);
}

#[test]
fn workspace_rename_and_user_edit_commands() {
    let mut s = session();
    assert_eq!(run(&mut s, "workspace.rename", json!({"name": " Harbor Labs "}))["name"], "Harbor Labs");
    assert_eq!(run(&mut s, "workspace.info", json!({}))["name"], "Harbor Labs");
    assert!(s.run("workspace.rename", &json!({"name": ""})).is_err());
    assert!(s.can_undo(), "rename is undoable");
    let u = run(&mut s, "user.edit", json!({"id": "sam", "name": "Samuel Okafor", "email": "sam@example.org"}));
    assert_eq!(u, json!({"id": "sam", "name": "Samuel Okafor", "email": "sam@example.org"}));
    assert_eq!(run(&mut s, "user.list", json!({}))[2]["email"], "sam@example.org");
    assert_eq!(run(&mut s, "user.edit", json!({"id": "sam", "email": null}))["email"], Value::Null);
    assert!(s.run("user.edit", &json!({"id": "sam"})).is_err(), "nothing to change");
    assert!(s.run("user.edit", &json!({"id": "sam", "email": "nope"})).is_err());
    assert!(s.run("user.edit", &json!({"id": "ghost", "name": "Ghost"})).is_err());
}

#[test]
fn revision_counts_changes_undo_and_redo() {
    let mut s = session();
    assert_eq!((s.revision(), s.undo_depth(), s.redo_depth()), (0, 0, 0));
    run(&mut s, "issue.transition", json!({"key": "LUME-3", "to": "In Progress"}));
    assert_eq!((s.revision(), s.undo_depth()), (1, 1));
    run(&mut s, "search.jql", json!({"jql": ""}));
    assert!(s.run("issue.transition", &json!({"key": "LUME-3", "to": "Nowhere"})).is_err());
    assert_eq!(s.revision(), 1, "queries and failures don't count");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((s.revision(), s.undo_depth(), s.redo_depth()), (2, 0, 1));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!((s.revision(), s.undo_depth(), s.redo_depth()), (3, 1, 0));
    run(&mut s, "workspace.new", json!({"name": "Fresh"}));
    assert_eq!((s.revision(), s.undo_depth()), (4, 0));
}

#[test]
fn workspace_import_is_one_undo_step() {
    let mut s = session();
    let dir = std::env::temp_dir().join(format!("trackcraft-engine-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("export.csv");
    // A synthetic CSV in the documented export columns (never a real export).
    std::fs::write(&path, "Summary,Issue key,Status,Labels,Labels\nImported one,IMP-4,Triage,a,b\n").unwrap();
    let r = run(&mut s, "workspace.import", json!({"kind": "jiraCsv", "path": path.display().to_string()}));
    assert_eq!(r["issues"], 1);
    assert_eq!(r["projects"], json!(["IMP"]));
    let i = s.ws.issue("IMP-4").unwrap();
    assert_eq!((i.status.as_str(), i.labels.clone()), ("Triage", vec!["a".to_string(), "b".to_string()]));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.ws.issue("IMP-4").is_err());
    assert!(s.run("workspace.import", &json!({"kind": "jiraJson", "path": path.display().to_string()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
