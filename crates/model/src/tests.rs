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
    assert!(w.transition("ATLAS-99", "Done", t(1)).is_err());
}

#[test]
fn workflow_and_resolution() {
    let mut w = ws();
    w.create_issue(new("Work"), "ann", t(1)).unwrap();
    assert!(w.transition("ATLAS-1", "Nowhere", t(2)).is_err());
    w.transition("ATLAS-1", "done", t(2)).unwrap();
    assert_eq!(w.issue("ATLAS-1").unwrap().status, "Done");
    assert_eq!(w.issue("ATLAS-1").unwrap().resolved, Some(t(2)));
    assert!(matches!(w.transition("ATLAS-1", "Done", t(3)), Err(ModelError::NoTransition { .. })));
    w.transition("ATLAS-1", "In Progress", t(3)).unwrap();
    assert_eq!(w.issue("ATLAS-1").unwrap().resolved, None);
}

#[test]
fn subtasks_links_and_deletes() {
    let mut w = ws();
    w.create_issue(new("Parent"), "ann", t(1)).unwrap();
    w.create_issue(NewIssue { issue_type: Some("sub-task".into()), parent: Some("ATLAS-1".into()), ..new("Child") }, "ann", t(1)).unwrap();
    w.create_issue(new("Other"), "ann", t(1)).unwrap();
    w.link("ATLAS-3", LinkKind::Blocks, "ATLAS-2", t(1)).unwrap();
    assert!(w.link("ATLAS-3", LinkKind::Blocks, "ATLAS-2", t(1)).is_err());
    assert!(w.link("ATLAS-3", LinkKind::Blocks, "ATLAS-3", t(1)).is_err());
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
    w.transition("ATLAS-2", "Done", t(3)).unwrap();
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
