use super::*;
use chrono::{TimeZone, Utc};
use trackcraft_jql::{Context, matches, parse};

fn t(h: u32) -> Timestamp {
    Utc.with_ymd_and_hms(2026, 10, 31, h, 0, 0).single().unwrap()
}

fn ws() -> Workspace {
    let mut w = Workspace::new("Test");
    w.add_user("ann", "Ann").unwrap();
    w.add_user("bob", "Bob").unwrap();
    w.create_project("ATLAS", "Atlas", ProjectKind::Software, Some("ann")).unwrap();
    w
}

fn new(summary: &str) -> NewIssue {
    NewIssue { project: "ATLAS".into(), summary: summary.into(), ..NewIssue::default() }
}

#[test]
fn issues_number_up_and_never_reuse_keys() {
    let mut w = ws();
    let a = w.create_issue(new("First"), "ann", t(1)).unwrap();
    let b = w.create_issue(new("Second"), "ann", t(1)).unwrap();
    assert_eq!((a.to_string(), b.to_string()), ("ATLAS-1".into(), "ATLAS-2".into()));
    w.delete_issue("ATLAS-2").unwrap();
    assert_eq!(w.create_issue(new("Third"), "ann", t(1)).unwrap().to_string(), "ATLAS-3");
    assert_eq!(w.issue("atlas-1").unwrap().status, "To Do");
}

#[test]
fn validation() {
    let mut w = ws();
    assert!(w.create_issue(new(""), "ann", t(1)).is_err());
    assert!(w.create_issue(new(&"x".repeat(300)), "ann", t(1)).is_err());
    assert!(w.create_issue(new("a\nb"), "ann", t(1)).is_err());
    assert!(w.create_issue(new("ok"), "nobody", t(1)).is_err());
    assert!(w.create_issue(NewIssue { priority: Some("Urgent!".into()), ..new("x") }, "ann", t(1)).is_err());
    assert!(w.create_issue(NewIssue { issue_type: Some("Sub-task".into()), ..new("x") }, "ann", t(1)).is_err());
    assert!(w.create_issue(NewIssue { story_points: Some(f64::NAN), ..new("x") }, "ann", t(1)).is_err());
    assert!(w.create_issue(NewIssue { labels: vec!["two words".into()], ..new("x") }, "ann", t(1)).is_err());
    assert!(w.create_project("ATLAS", "Again", ProjectKind::Software, None).is_err());
    assert!(w.create_project("bad key", "x", ProjectKind::Software, None).is_err());
    assert!(w.issue("NOPE-1").is_err());
    assert!(w.transition("ATLAS-99", "Done", "ann", t(1)).is_err());
}

#[test]
fn workflow_and_resolution() {
    let mut w = ws();
    w.create_issue(new("Work"), "ann", t(1)).unwrap();
    assert!(w.transition("ATLAS-1", "Nowhere", "ann", t(2)).is_err());
    w.transition("ATLAS-1", "done", "ann", t(2)).unwrap();
    assert_eq!(w.issue("ATLAS-1").unwrap().status, "Done");
    assert_eq!(w.issue("ATLAS-1").unwrap().resolved, Some(t(2)));
    assert!(matches!(w.transition("ATLAS-1", "Done", "ann", t(3)), Err(ModelError::NoTransition { .. })));
    w.transition("ATLAS-1", "In Progress", "ann", t(3)).unwrap();
    assert_eq!(w.issue("ATLAS-1").unwrap().resolved, None);
}

#[test]
fn subtasks_links_and_deletes() {
    let mut w = ws();
    w.create_issue(new("Parent"), "ann", t(1)).unwrap();
    w.create_issue(NewIssue { issue_type: Some("sub-task".into()), parent: Some("ATLAS-1".into()), ..new("Child") }, "ann", t(1)).unwrap();
    w.create_issue(new("Other"), "ann", t(1)).unwrap();
    w.link("ATLAS-3", LinkKind::Blocks, "ATLAS-2", "ann", t(1)).unwrap();
    assert!(w.link("ATLAS-3", LinkKind::Blocks, "ATLAS-2", "ann", t(1)).is_err());
    assert!(w.link("ATLAS-3", LinkKind::Blocks, "ATLAS-3", "ann", t(1)).is_err());
    let gone = w.delete_issue("ATLAS-1").unwrap();
    assert_eq!(gone.len(), 2);
    assert!(w.issue("ATLAS-3").unwrap().links.is_empty());
}

#[test]
fn jql_over_issues() {
    let mut w = ws();
    w.create_issue(
        NewIssue { priority: Some("High".into()), assignee: Some("bob".into()), labels: vec!["api".into()], ..new("Fix login") },
        "ann",
        t(1),
    )
    .unwrap();
    w.create_issue(new("Write docs"), "ann", t(2)).unwrap();
    w.transition("ATLAS-2", "Done", "ann", t(3)).unwrap();
    let cx = Context { current_user: Some("bob".into()), now: t(5) };
    let hits = |q: &str| -> Vec<String> {
        let q = parse(q).unwrap();
        w.issues.values().filter(|i| matches(&q, &IssueRecord { ws: &w, issue: i }, &cx).unwrap()).map(|i| i.key.to_string()).collect()
    };
    assert_eq!(hits("assignee = currentUser()"), ["ATLAS-1"]);
    assert_eq!(hits("statusCategory = Done"), ["ATLAS-2"]);
    assert_eq!(hits("statusCategory != Done AND priority = high"), ["ATLAS-1"]);
    assert_eq!(hits("project = atlas AND text ~ login"), ["ATLAS-1"]);
    assert_eq!(hits("resolved IS NOT EMPTY"), ["ATLAS-2"]);
}

#[test]
fn page_tree_and_history() {
    let mut w = ws();
    w.create_space("ENG", "Engineering", "ann", t(1)).unwrap();
    let home = w.space("eng").unwrap().homepage;
    let a = w.create_page(NewPage { space: "ENG".into(), parent: None, title: "Architecture".into(), body: Doc::plain("v1") }, "ann", t(1)).unwrap();
    let b = w.create_page(NewPage { space: "ENG".into(), parent: Some(a), title: "Storage".into(), body: Doc::default() }, "ann", t(1)).unwrap();
    assert!(w.create_page(NewPage { space: "ENG".into(), title: "architecture".into(), ..NewPage::default() }, "ann", t(1)).is_err());
    assert_eq!(w.ancestors(b), vec![a, home]);
    assert!(w.move_page(a, b).is_err(), "cycle");
    assert!(w.move_page(home, a).is_err());
    assert!(w.delete_page(home).is_err());

    let v2 = w.edit_page(a, PageEdit { body: Some(Doc::plain("v2")), message: "rewrite".into(), ..PageEdit::default() }, "bob", t(2)).unwrap();
    assert_eq!(v2, 2);
    let stale = w.edit_page(a, PageEdit { body: Some(Doc::plain("v2b")), base_version: Some(1), ..PageEdit::default() }, "ann", t(3));
    assert_eq!(stale, Err(ModelError::StaleVersion { expected: 1, actual: 2 }));
    assert_eq!(w.restore_page(a, 1, "ann", t(4)).unwrap(), 3);
    let p = w.page(a).unwrap();
    assert_eq!(p.body.plain_text(), "v1");
    assert_eq!(p.history.len(), 2);
    assert_eq!(p.history.get(1).map(|v| v.message.as_str()), Some("rewrite"));

    assert_eq!(w.delete_page(a).unwrap(), vec![a, b]);
}

#[test]
fn corrupt_parent_cycle_does_not_hang() {
    let mut w = ws();
    w.create_space("ENG", "Engineering", "ann", t(1)).unwrap();
    let a = w.create_page(NewPage { space: "ENG".into(), title: "A".into(), ..NewPage::default() }, "ann", t(1)).unwrap();
    let b = w.create_page(NewPage { space: "ENG".into(), parent: Some(a), title: "B".into(), ..NewPage::default() }, "ann", t(1)).unwrap();
    // Simulate a hand-edited file with a cycle.
    if let Some(p) = w.pages.get_mut(&a) {
        p.parent = Some(b);
    }
    assert!(w.ancestors(a).len() <= MAX_PAGE_DEPTH * 4 + 1);
    assert!(w.subtree(a).len() <= w.pages.len());
}

fn fields(w: &Workspace, key: &str) -> Vec<(String, Option<String>, Option<String>)> {
    w.issue(key).unwrap().history.iter().map(|c| (c.field.clone(), c.from.clone(), c.to.clone())).collect()
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[test]
fn every_primitive_records_history() {
    let mut w = ws();
    w.create_issue(new("One"), "ann", t(1)).unwrap();
    w.create_issue(new("Two"), "ann", t(1)).unwrap();
    w.set_summary("ATLAS-1", "Uno", "bob", t(2)).unwrap();
    w.set_summary("ATLAS-1", "Uno", "bob", t(2)).unwrap(); // no change, no entry
    w.set_description("ATLAS-1", Doc::plain(&"word ".repeat(100)), "bob", t(2)).unwrap();
    w.set_priority("ATLAS-1", "high", "bob", t(2)).unwrap();
    w.set_labels("ATLAS-1", &["a".into(), "b".into()], "bob", t(2)).unwrap();
    w.set_story_points("ATLAS-1", Some(3.0), "bob", t(2)).unwrap();
    w.assign("ATLAS-1", Some("bob"), "ann", t(3)).unwrap();
    w.transition("ATLAS-1", "Done", "bob", t(4)).unwrap();
    w.transition("ATLAS-1", "In Progress", "bob", t(5)).unwrap();
    w.link("ATLAS-1", LinkKind::Blocks, "ATLAS-2", "ann", t(6)).unwrap();
    let c = w.add_comment("ATLAS-1", "ann", Doc::plain("Looks good"), t(7)).unwrap();
    w.delete_comment("ATLAS-1", c, "ann", t(8)).unwrap();
    let h = fields(&w, "ATLAS-1");
    let names: Vec<&str> = h.iter().map(|(f, ..)| f.as_str()).collect();
    assert_eq!(
        names,
        [
            "created",
            "summary",
            "description",
            "priority",
            "labels",
            "storyPoints",
            "assignee",
            "status",
            "resolution",
            "status",
            "resolution",
            "link",
            "comment",
            "comment"
        ]
    );
    assert_eq!(h[1], ("summary".into(), s("One"), s("Uno")));
    let desc = h[2].2.clone().unwrap();
    assert!(desc.chars().count() <= HISTORY_PREVIEW + 1 && desc.ends_with('…'), "preview, not a full copy: {desc}");
    assert_eq!(h[3], ("priority".into(), s("Medium"), s("High")));
    assert_eq!(h[4], ("labels".into(), None, s("a b")));
    assert_eq!(h[6], ("assignee".into(), None, s("bob")));
    assert_eq!(h[7], ("status".into(), s("To Do"), s("Done")));
    assert_eq!(h[8], ("resolution".into(), None, s(RESOLUTION_DONE)));
    assert_eq!(h[10], ("resolution".into(), s(RESOLUTION_DONE), None));
    assert_eq!(h[11].2, s("This issue blocks ATLAS-2"));
    assert_eq!((h[12].2.clone(), h[13].1.clone()), (s("Looks good"), s("Looks good")));
    let i = w.issue("ATLAS-1").unwrap();
    assert_eq!(i.history[0].author, "ann");
    assert_eq!(i.history[1].author, "bob");
    // Outside a batch each primitive gets its own id; the two entries of a transition share one.
    assert_ne!(i.history[1].id, i.history[2].id);
    assert_eq!(i.history[7].id, i.history[8].id);
    // A failed primitive records nothing.
    assert!(w.transition("ATLAS-1", "Nowhere", "bob", t(9)).is_err());
    assert_eq!(w.issue("ATLAS-1").unwrap().history.len(), 14);
}

#[test]
fn batches_group_changes_and_history_is_capped() {
    let mut w = ws();
    w.create_issue(new("One"), "ann", t(1)).unwrap();
    w.begin_batch();
    w.set_summary("ATLAS-1", "Uno", "ann", t(2)).unwrap();
    w.set_priority("ATLAS-1", "Low", "ann", t(2)).unwrap();
    w.end_batch();
    let h = &w.issue("ATLAS-1").unwrap().history;
    assert_eq!(h[1].id, h[2].id);
    assert_ne!(h[0].id, h[1].id);
    for n in 0..MAX_HISTORY + 5 {
        w.set_summary("ATLAS-1", &format!("s{n}"), "ann", t(3)).unwrap();
    }
    let h = &w.issue("ATLAS-1").unwrap().history;
    assert_eq!(h.len(), MAX_HISTORY);
    assert_eq!(h.last().and_then(|c| c.to.clone()), Some(format!("s{}", MAX_HISTORY + 4)));
    assert_ne!(h[0].field, "created", "oldest dropped first");
}

#[test]
fn issues_without_history_still_load() {
    let mut w = ws();
    w.create_issue(new("Old"), "ann", t(1)).unwrap();
    let mut v = serde_json::to_value(&w).unwrap();
    v["issues"]["ATLAS-1"].as_object_mut().unwrap().remove("history");
    let back: Workspace = serde_json::from_value(v).unwrap();
    assert!(back.issue("ATLAS-1").unwrap().history.is_empty());
}

#[test]
fn jql_history_over_issues() {
    let mut w = ws();
    w.create_issue(new("Moves"), "ann", t(1)).unwrap();
    w.create_issue(new("Stays"), "ann", t(1)).unwrap();
    w.transition("ATLAS-1", "In Progress", "bob", t(2)).unwrap();
    w.transition("ATLAS-1", "Done", "ann", t(4)).unwrap();
    w.assign("ATLAS-2", Some("bob"), "ann", t(3)).unwrap();
    w.set_priority("ATLAS-2", "Low", "bob", t(3)).unwrap();
    let cx = Context { current_user: Some("bob".into()), now: t(6) };
    let hits = |q: &str| -> Vec<String> {
        let q = parse(q).unwrap();
        w.issues.values().filter(|i| matches(&q, &IssueRecord { ws: &w, issue: i }, &cx).unwrap()).map(|i| i.key.to_string()).collect()
    };
    assert_eq!(hits(r#"status WAS "In Progress""#), ["ATLAS-1"]);
    assert_eq!(hits(r#"status WAS "To Do""#), ["ATLAS-1", "ATLAS-2"]);
    assert_eq!(hits(r#"status WAS NOT "Done""#), ["ATLAS-2"]);
    assert_eq!(hits(r#"status CHANGED FROM "To Do" TO "In Progress" BY currentUser()"#), ["ATLAS-1"]);
    assert_eq!(hits("status WAS \"To Do\" BY ann"), ["ATLAS-1", "ATLAS-2"], "creator set the initial status");
    assert_eq!(hits("assignee WAS EMPTY"), ["ATLAS-1", "ATLAS-2"]);
    assert_eq!(hits("assignee WAS bob"), ["ATLAS-2"]);
    assert_eq!(hits("assignee CHANGED"), ["ATLAS-2"]);
    assert_eq!(hits("priority WAS Medium AND priority CHANGED TO Low"), ["ATLAS-2"]);
    assert_eq!(hits("priority WAS IN (Low, Lowest)"), ["ATLAS-2"]);
    let q = parse("labels WAS x").unwrap();
    let i = w.issue("ATLAS-1").unwrap();
    assert!(matches(&q, &IssueRecord { ws: &w, issue: i }, &cx).is_err());
}
