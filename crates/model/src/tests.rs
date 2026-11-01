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
    w.delete_issue("ATLAS-2", "ann", t(5)).unwrap();
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
    let gone = w.delete_issue("ATLAS-1", "ann", t(5)).unwrap();
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

// ---- rank ------------------------------------------------------------------------------------

#[test]
fn rank_between_neighbours_and_ends() {
    let r = |lo: Option<&str>, hi: Option<&str>| rank_between(lo, hi).unwrap();
    assert_eq!(r(None, None), "i");
    let mid = r(Some("a"), Some("b"));
    assert!("a" < mid.as_str() && mid.as_str() < "b", "{mid}");
    let tight = r(Some("a"), Some("a1"));
    assert!("a" < tight.as_str() && tight.as_str() < "a1", "{tight}");
    let first = r(None, Some("1"));
    assert!(first.as_str() < "1" && is_valid_rank(&first), "{first}");
    let last = r(Some("zzzzzz"), None);
    assert!(last.as_str() > "zzzzzz" && is_valid_rank(&last), "{last}");
    assert!(r(None, Some("i")).as_str() < "i");
    assert!(r(Some("i"), None).as_str() > "i");
    // Malformed or reversed input gives None, never a panic.
    assert_eq!(rank_between(Some("b"), Some("a")), None);
    assert_eq!(rank_between(Some("a"), Some("a")), None);
    assert_eq!(rank_between(Some("A!"), None), None);
    assert_eq!(rank_between(Some("a0"), None), None, "trailing zeros are not ranks");
    assert_eq!(rank_between(None, Some("")), None);
}

#[test]
fn rebalanced_ranks_are_ordered_short_and_leave_room() {
    for n in [0, 1, 2, 10, 1000, 50_000] {
        let r = rebalanced(n);
        assert_eq!(r.len(), n);
        assert!(r.windows(2).all(|w| w[0] < w[1]), "ordered for {n}");
        assert!(r.iter().all(|x| is_valid_rank(x)), "valid for {n}");
    }
    let r = rebalanced(3);
    assert!(rank_between(None, r.first().map(String::as_str)).is_some());
    assert!(rank_between(r.last().map(String::as_str), None).is_some_and(|x| x.len() <= 6));
}

#[test]
fn ranks_between_fits_many_in_a_tight_gap() {
    let v = ranks_between(Some("a"), Some("a1"), 1000).unwrap();
    assert_eq!(v.len(), 1000);
    assert!(v.windows(2).all(|w| w[0] < w[1]));
    assert!(v.first().is_some_and(|x| x.as_str() > "a") && v.last().is_some_and(|x| x.as_str() < "a1"));
    let tail = ranks_between(Some("z"), None, 50).unwrap();
    assert!(tail.windows(2).all(|w| w[0] < w[1]) && tail[0].as_str() > "z");
    let head = ranks_between(None, Some("1"), 50).unwrap();
    assert!(head.windows(2).all(|w| w[0] < w[1]) && head[49].as_str() < "1");
}

fn order(w: &Workspace) -> Vec<String> {
    w.rank_order(&w.project("ATLAS").unwrap().key).iter().map(ToString::to_string).collect()
}

#[test]
fn new_issues_rank_at_the_bottom_and_moves_keep_order() {
    let mut w = ws();
    for n in 1..=5 {
        w.create_issue(new(&format!("i{n}")), "ann", t(1)).unwrap();
    }
    assert_eq!(order(&w), ["ATLAS-1", "ATLAS-2", "ATLAS-3", "ATLAS-4", "ATLAS-5"]);
    w.rank_issues(&["ATLAS-5".into(), "ATLAS-4".into()], Anchor::Before("ATLAS-2")).unwrap();
    assert_eq!(order(&w), ["ATLAS-1", "ATLAS-5", "ATLAS-4", "ATLAS-2", "ATLAS-3"]);
    w.rank_issues(&["ATLAS-1".into()], Anchor::After("ATLAS-3")).unwrap();
    assert_eq!(order(&w), ["ATLAS-5", "ATLAS-4", "ATLAS-2", "ATLAS-3", "ATLAS-1"]);
    w.rank_issues(&["ATLAS-3".into()], Anchor::Before("ATLAS-5")).unwrap();
    assert_eq!(order(&w), ["ATLAS-3", "ATLAS-5", "ATLAS-4", "ATLAS-2", "ATLAS-1"]);
    assert!(w.rank_issues(&["ATLAS-3".into()], Anchor::Before("ATLAS-3")).is_err(), "anchor can't be moved");
    assert!(w.rank_issues(&[], Anchor::Before("ATLAS-3")).is_err());
    assert!(w.rank_issues(&["ATLAS-9".into()], Anchor::Before("ATLAS-3")).is_err());
    assert!(w.rank_issues(&vec!["ATLAS-1".to_string(); MAX_MOVE + 1], Anchor::Before("ATLAS-3")).is_err());
    w.create_project("OTHER", "Other", ProjectKind::Software, None).unwrap();
    w.create_issue(NewIssue { project: "OTHER".into(), ..new("o") }, "ann", t(1)).unwrap();
    assert!(w.rank_issues(&["OTHER-1".into()], Anchor::Before("ATLAS-3")).is_err(), "one project");
    assert!(w.rank_issues(&["OTHER-1".into(), "ATLAS-1".into()], Anchor::Before("ATLAS-3")).is_err());
    // Ranking is not history-worthy.
    assert_eq!(w.issue("ATLAS-3").unwrap().history.len(), 1);
}

#[test]
fn repeated_inserts_at_one_spot_stay_ordered_and_bounded() {
    let mut w = ws();
    for n in 1..=3 {
        w.create_issue(new(&format!("i{n}")), "ann", t(1)).unwrap();
    }
    // 600 issues, each ranked right after the previous one, just after ATLAS-1: the gap narrows
    // every time, so the project rebalances when keys get long.
    let mut want = vec!["ATLAS-1".to_string()];
    let mut prev = "ATLAS-1".to_string();
    for _ in 0..600 {
        let k = w.create_issue(new("x"), "ann", t(1)).unwrap().to_string();
        w.rank_issues(std::slice::from_ref(&k), Anchor::After(&prev)).unwrap();
        want.push(k.clone());
        prev = k;
    }
    want.extend(["ATLAS-2".to_string(), "ATLAS-3".to_string()]);
    assert_eq!(order(&w), want);
    assert!(w.issues.values().all(|i| is_valid_rank(&i.rank) && i.rank.len() <= MAX_RANK_LEN));
    // And 600 inserted at the same spot (always just before ATLAS-2).
    for _ in 0..600 {
        let k = w.create_issue(new("y"), "ann", t(1)).unwrap().to_string();
        w.rank_issues(&[k], Anchor::Before("ATLAS-2")).unwrap();
    }
    let o = order(&w);
    assert_eq!(o.first().map(String::as_str), Some("ATLAS-1"));
    assert_eq!(&o[o.len() - 2..], ["ATLAS-2", "ATLAS-3"]);
    let ranks: Vec<&str> = w.rank_order(&w.project("ATLAS").unwrap().key).iter().map(|k| w.issues[k].rank.as_str()).collect();
    assert!(ranks.windows(2).all(|p| p[0] < p[1]), "unique and ordered");
    assert!(ranks.iter().all(|r| r.len() <= MAX_RANK_LEN));
}

#[test]
fn repair_ranks_unranked_issues_in_key_order() {
    let mut w = ws();
    for n in 1..=4 {
        w.create_issue(new(&format!("i{n}")), "ann", t(1)).unwrap();
    }
    w.rank_issues(&["ATLAS-4".into()], Anchor::Before("ATLAS-1")).unwrap();
    let good = w.clone();
    w.repair();
    assert_eq!(w, good, "valid ranks untouched");
    for i in w.issues.values_mut() {
        i.rank.clear();
    }
    w.repair();
    assert_eq!(order(&w), ["ATLAS-1", "ATLAS-2", "ATLAS-3", "ATLAS-4"]);
    // Duplicates and junk: the valid order first, then the rest by key.
    let mut w = good.clone();
    let r = w.issue("ATLAS-1").unwrap().rank.clone();
    for i in w.issues.values_mut() {
        if i.key.number == 2 {
            i.rank = r.clone();
        }
        if i.key.number == 3 {
            i.rank = "NOT A RANK".into();
        }
    }
    w.repair();
    let ranks: std::collections::BTreeSet<_> = w.issues.values().map(|i| i.rank.clone()).collect();
    assert_eq!(ranks.len(), 4);
    assert_eq!(order(&w).last().map(String::as_str), Some("ATLAS-3"));
}

// ---- sprints ---------------------------------------------------------------------------------

fn sprint_ws() -> (Workspace, Id) {
    let mut w = ws();
    for n in 1..=4 {
        w.create_issue(new(&format!("i{n}")), "ann", t(1)).unwrap();
    }
    let s = w.create_sprint("atlas", None, Some("Ship it"), None, None).unwrap();
    (w, s)
}

#[test]
fn sprint_lifecycle() {
    let (mut w, s) = sprint_ws();
    assert_eq!(w.sprint(s).unwrap().name, "ATLAS Sprint 1");
    assert_eq!(w.sprint(s).unwrap().state, SprintState::Future);
    assert!(w.start_sprint(s, None, t(1), t(9), None).is_err(), "no issues yet");
    w.set_sprint(&["ATLAS-1".into(), "ATLAS-2".into()], Some(s), "ann", t(2)).unwrap();
    let h = w.issue("ATLAS-1").unwrap().history.last().unwrap().clone();
    assert_eq!((h.field.as_str(), h.from.clone(), h.to.clone()), ("sprint", None, Some("ATLAS Sprint 1".into())));
    assert!(w.start_sprint(s, None, t(9), t(1), None).is_err(), "end before start");
    w.start_sprint(s, None, t(1), t(9), Some("Ship it now")).unwrap();
    assert_eq!(w.sprint(s).unwrap().state, SprintState::Active);
    assert_eq!(w.sprint(s).unwrap().goal, "Ship it now");
    assert!(w.start_sprint(s, None, t(1), t(9), None).is_err(), "already active");

    let s2 = w.create_sprint("ATLAS", None, None, None, None).unwrap();
    assert_eq!(w.sprint(s2).unwrap().name, "ATLAS Sprint 2");
    w.set_sprint(&["ATLAS-3".into()], Some(s2), "ann", t(2)).unwrap();
    let e = w.start_sprint(s2, None, t(1), t(9), None).unwrap_err().to_string();
    assert!(e.contains("already active"), "{e}");

    w.transition("ATLAS-1", "Done", "ann", t(3)).unwrap();
    let c = w.complete_sprint(s, MoveOpenTo::Sprint(s2), "bob", t(10)).unwrap();
    assert_eq!(c.done.iter().map(ToString::to_string).collect::<Vec<_>>(), ["ATLAS-1"]);
    assert_eq!(c.moved.iter().map(ToString::to_string).collect::<Vec<_>>(), ["ATLAS-2"]);
    let sp = w.sprint(s).unwrap();
    assert_eq!((sp.state, sp.completed), (SprintState::Closed, Some(t(10))));
    let i1 = w.issue("ATLAS-1").unwrap();
    assert_eq!((i1.sprint, i1.past_sprints.clone()), (None, vec![s]));
    let i2 = w.issue("ATLAS-2").unwrap();
    assert_eq!((i2.sprint, i2.past_sprints.clone()), (Some(s2), vec![s]));
    let h = i2.history.last().unwrap();
    assert_eq!((h.field.as_str(), h.author.as_str(), h.to.as_deref()), ("sprint", "bob", Some("ATLAS Sprint 2")));

    // Closed sprints refuse changes.
    assert!(w.set_sprint(&["ATLAS-4".into()], Some(s), "ann", t(11)).is_err());
    assert!(w.delete_sprint(s, "ann", t(11)).is_err());
    assert!(w.complete_sprint(s, MoveOpenTo::Backlog, "ann", t(11)).is_err());
    assert!(w.edit_sprint(s, SprintEdit { start: Some(Some(t(1))), ..SprintEdit::default() }).is_err());
    w.edit_sprint(s, SprintEdit { name: Some("Renamed".into()), ..SprintEdit::default() }).unwrap();
    assert!(w.complete_sprint(s2, MoveOpenTo::Backlog, "ann", t(11)).is_err(), "not active");
    assert!(w.complete_sprint(s2, MoveOpenTo::Sprint(s), "ann", t(11)).is_err());

    // Complete into a new sprint.
    w.start_sprint(s2, None, t(11), t(20), None).unwrap();
    assert!(w.complete_sprint(s2, MoveOpenTo::Sprint(s), "ann", t(20)).is_err(), "can't move into a closed sprint");
    let c = w.complete_sprint(s2, MoveOpenTo::New, "ann", t(20)).unwrap();
    let new_id = c.to.unwrap();
    assert_eq!(w.sprint(new_id).unwrap().state, SprintState::Future);
    assert_eq!(w.issue("ATLAS-2").unwrap().sprint, Some(new_id));
    assert_eq!(w.issue("ATLAS-2").unwrap().past_sprints, vec![s, s2]);
}

#[test]
fn sprint_refusals_and_delete() {
    let (mut w, s) = sprint_ws();
    assert!(w.create_sprint("NOPE", None, None, None, None).is_err());
    assert!(w.create_sprint("ATLAS", Some("x\ny"), None, None, None).is_err());
    assert!(w.create_sprint("ATLAS", None, Some(&"g".repeat(MAX_GOAL + 1)), None, None).is_err());
    assert!(w.create_sprint("ATLAS", None, None, Some(t(5)), Some(t(5))).is_err());
    assert!(w.sprint(Id(999_999)).is_err());
    assert!(w.set_sprint(&["ATLAS-1".into()], Some(Id(999_999)), "ann", t(1)).is_err());
    w.create_project("OTHER", "Other", ProjectKind::Software, None).unwrap();
    w.create_issue(NewIssue { project: "OTHER".into(), ..new("o") }, "ann", t(1)).unwrap();
    assert!(w.set_sprint(&["OTHER-1".into()], Some(s), "ann", t(1)).is_err(), "another project's sprint");

    // Sub-tasks follow their parent.
    w.create_issue(NewIssue { issue_type: Some("Sub-task".into()), parent: Some("ATLAS-1".into()), ..new("sub") }, "ann", t(1)).unwrap();
    assert!(w.set_sprint(&["ATLAS-5".into()], Some(s), "ann", t(1)).is_err());
    w.set_sprint(&["ATLAS-1".into()], Some(s), "ann", t(2)).unwrap();
    assert_eq!(w.issue("ATLAS-5").unwrap().sprint, Some(s));
    w.create_issue(NewIssue { issue_type: Some("Sub-task".into()), parent: Some("ATLAS-1".into()), ..new("sub2") }, "ann", t(1)).unwrap();
    assert_eq!(w.issue("ATLAS-6").unwrap().sprint, Some(s), "a new sub-task joins its parent's sprint");

    // A sprint of only done issues can't start.
    for k in ["ATLAS-1", "ATLAS-5", "ATLAS-6"] {
        w.transition(k, "Done", "ann", t(3)).unwrap();
    }
    assert!(w.start_sprint(s, None, t(1), t(9), None).unwrap_err().to_string().contains("no open issues"));

    let back = w.delete_sprint(s, "bob", t(4)).unwrap();
    assert_eq!(back.len(), 3);
    assert!(w.sprints.is_empty());
    let i = w.issue("ATLAS-1").unwrap();
    assert_eq!(i.sprint, None);
    assert_eq!(i.history.last().map(|c| (c.field.as_str(), c.from.as_deref())), Some(("sprint", Some("ATLAS Sprint 1"))));
}

#[test]
fn jql_sprint_fields_and_functions() {
    let (mut w, s) = sprint_ws();
    w.set_sprint(&["ATLAS-1".into(), "ATLAS-2".into()], Some(s), "ann", t(2)).unwrap();
    w.start_sprint(s, None, t(1), t(9), None).unwrap();
    w.transition("ATLAS-1", "Done", "ann", t(3)).unwrap();
    let s2 = w.create_sprint("ATLAS", Some("Next"), None, None, None).unwrap();
    w.set_sprint(&["ATLAS-3".into()], Some(s2), "ann", t(2)).unwrap();
    let keys = |w: &Workspace, q: &str| -> Vec<String> {
        let q = parse(q).unwrap();
        let cx = Context { current_user: None, now: t(12) };
        let mut hits: Vec<IssueRecord<'_>> =
            w.issues.values().map(|issue| IssueRecord { ws: w, issue }).filter(|r| matches(&q, r, &cx).unwrap()).collect();
        trackcraft_jql::sort(&q, &mut hits);
        hits.iter().map(|r| r.issue.key.to_string()).collect()
    };
    assert_eq!(keys(&w, &format!("sprint = {} ORDER BY key", s.0)), ["ATLAS-1", "ATLAS-2"]);
    assert_eq!(keys(&w, r#"sprint = "ATLAS Sprint 1" ORDER BY key"#), ["ATLAS-1", "ATLAS-2"]);
    assert_eq!(keys(&w, "sprint IN openSprints() ORDER BY key"), ["ATLAS-1", "ATLAS-2", "ATLAS-3"]);
    assert_eq!(keys(&w, "sprint IN closedSprints()"), [] as [&str; 0]);
    assert_eq!(keys(&w, "sprint IS EMPTY"), ["ATLAS-4"]);
    assert_eq!(keys(&w, "sprint NOT IN openSprints()"), [] as [&str; 0]);
    w.complete_sprint(s, MoveOpenTo::Backlog, "ann", t(10)).unwrap();
    assert_eq!(keys(&w, "sprint IN closedSprints() ORDER BY key"), ["ATLAS-1", "ATLAS-2"]);
    assert_eq!(keys(&w, "sprint IN openSprints()"), ["ATLAS-3"]);
    assert_eq!(keys(&w, "sprint = Next"), ["ATLAS-3"]);
    // ORDER BY rank follows the backlog order, byte-wise.
    w.rank_issues(&["ATLAS-4".into(), "ATLAS-3".into()], Anchor::Before("ATLAS-1")).unwrap();
    assert_eq!(keys(&w, "ORDER BY rank"), ["ATLAS-4", "ATLAS-3", "ATLAS-1", "ATLAS-2"]);
    assert_eq!(keys(&w, "ORDER BY rank DESC"), ["ATLAS-2", "ATLAS-1", "ATLAS-3", "ATLAS-4"]);
    let q = parse("sprint IN futureSprints()").unwrap();
    let r = IssueRecord { ws: &w, issue: w.issue("ATLAS-1").unwrap() };
    assert!(matches(&q, &r, &Context { current_user: None, now: t(12) }).is_err(), "unknown list function");
}

// ---- M2.3: hierarchy edits, unlink, comment edit, delete history, sprint rules ----------------

fn sub(parent: &str, summary: &str) -> NewIssue {
    NewIssue { issue_type: Some("Sub-task".into()), parent: Some(parent.into()), ..new(summary) }
}

fn epic(summary: &str) -> NewIssue {
    NewIssue { issue_type: Some("Epic".into()), ..new(summary) }
}

#[test]
fn delete_records_lost_links_and_parents_on_other_issues() {
    let mut w = ws();
    w.create_issue(epic("Epic"), "ann", t(1)).unwrap(); // 1
    w.create_issue(NewIssue { parent: Some("ATLAS-1".into()), ..new("Story") }, "ann", t(1)).unwrap(); // 2
    w.create_issue(new("Other"), "ann", t(1)).unwrap(); // 3
    w.link("ATLAS-3", LinkKind::Blocks, "ATLAS-1", "ann", t(1)).unwrap();
    w.delete_issue("ATLAS-1", "bob", t(6)).unwrap();
    assert_eq!(fields(&w, "ATLAS-2").last().cloned(), Some(("parent".into(), s("ATLAS-1"), None)));
    assert_eq!(fields(&w, "ATLAS-3").last().cloned(), Some(("link".into(), s("This issue blocks ATLAS-1"), None)));
    let c = w.issue("ATLAS-3").unwrap().history.last().unwrap().clone();
    assert_eq!((c.author.as_str(), c.at), ("bob", t(6)));
    assert_eq!(w.issue("ATLAS-2").unwrap().updated, t(6));
}

#[test]
fn unlink_removes_either_side_and_records() {
    let mut w = ws();
    w.create_issue(new("A"), "ann", t(1)).unwrap();
    w.create_issue(new("B"), "ann", t(1)).unwrap();
    w.link("ATLAS-1", LinkKind::Blocks, "ATLAS-2", "ann", t(1)).unwrap();
    assert!(w.unlink("ATLAS-1", LinkKind::Relates, "ATLAS-2", "ann", t(2)).is_err(), "wrong kind");
    w.unlink("ATLAS-2", LinkKind::Blocks, "ATLAS-1", "bob", t(2)).unwrap(); // from the inward side
    assert!(w.issue("ATLAS-1").unwrap().links.is_empty());
    assert_eq!(fields(&w, "ATLAS-1").last().cloned(), Some(("link".into(), s("This issue blocks ATLAS-2"), None)));
    assert!(w.unlink("ATLAS-1", LinkKind::Blocks, "ATLAS-2", "ann", t(2)).is_err(), "already gone");
    assert!(w.unlink("ATLAS-1", LinkKind::Blocks, "ATLAS-99", "ann", t(2)).is_err());
}

#[test]
fn edit_comment_is_author_only() {
    let mut w = ws();
    w.create_issue(new("A"), "ann", t(1)).unwrap();
    let id = w.add_comment("ATLAS-1", "ann", Doc::plain("first"), t(1)).unwrap();
    assert!(w.edit_comment("ATLAS-1", id, Doc::plain("hijack"), "bob", t(2)).is_err());
    assert!(w.edit_comment("ATLAS-1", id, Doc::default(), "ann", t(2)).is_err());
    assert!(w.edit_comment("ATLAS-1", Id(999_999), Doc::plain("x"), "ann", t(2)).is_err());
    w.edit_comment("ATLAS-1", id, Doc::plain("second"), "ann", t(3)).unwrap();
    let c = &w.issue("ATLAS-1").unwrap().comments[0];
    assert_eq!((c.body.plain_text().trim(), c.created, c.updated), ("second", t(1), t(3)));
    assert_eq!(fields(&w, "ATLAS-1").last().cloned(), Some(("comment".into(), s("first"), s("second"))));
}

#[test]
fn type_and_parent_changes_keep_the_hierarchy_valid() {
    let mut w = ws();
    w.create_issue(epic("Epic"), "ann", t(1)).unwrap(); // 1
    w.create_issue(new("Story"), "ann", t(1)).unwrap(); // 2
    w.create_issue(sub("ATLAS-2", "Sub"), "ann", t(1)).unwrap(); // 3
    w.create_issue(new("Loose"), "ann", t(1)).unwrap(); // 4
    let set = |w: &mut Workspace, k: &str, ty: Option<&str>, p: Option<Option<&str>>| w.set_type_and_parent(k, ty, p, "bob", t(4));

    assert!(set(&mut w, "ATLAS-4", Some("Saga"), None).is_err(), "unknown type");
    assert!(set(&mut w, "ATLAS-4", Some("Sub-task"), None).is_err(), "a sub-task needs a parent");
    assert!(set(&mut w, "ATLAS-4", Some("Sub-task"), Some(Some("ATLAS-1"))).is_err(), "not under an epic");
    assert!(set(&mut w, "ATLAS-4", Some("Sub-task"), Some(Some("ATLAS-3"))).is_err(), "not under a sub-task");
    assert!(set(&mut w, "ATLAS-4", None, Some(Some("ATLAS-3"))).is_err(), "nothing under a sub-task");
    assert!(set(&mut w, "ATLAS-1", None, Some(Some("ATLAS-4"))).is_err(), "an epic has no parent");
    assert!(set(&mut w, "ATLAS-4", None, Some(Some("ATLAS-4"))).is_err(), "not its own parent");
    assert!(set(&mut w, "ATLAS-2", Some("Sub-task"), Some(Some("ATLAS-4"))).is_err(), "has children");
    assert!(set(&mut w, "ATLAS-2", Some("Epic"), None).is_err(), "an epic can't hold sub-tasks");
    assert!(set(&mut w, "ATLAS-3", None, Some(None)).is_err(), "a sub-task keeps a parent");

    // Story under the epic, then cycle refusal.
    set(&mut w, "ATLAS-2", None, Some(Some("ATLAS-1"))).unwrap();
    assert_eq!(fields(&w, "ATLAS-2").last().cloned(), Some(("parent".into(), None, s("ATLAS-1"))));
    set(&mut w, "ATLAS-4", None, Some(Some("ATLAS-2"))).unwrap();
    set(&mut w, "ATLAS-1", Some("Task"), None).unwrap();
    assert!(set(&mut w, "ATLAS-1", None, Some(Some("ATLAS-4"))).is_err(), "cycle");

    // Sub-task → task clears the parent; task → sub-task takes one and its sprint.
    set(&mut w, "ATLAS-3", Some("Task"), None).unwrap();
    let i = w.issue("ATLAS-3").unwrap();
    assert_eq!((i.issue_type.as_str(), i.parent.clone()), ("Task", None));
    let f = fields(&w, "ATLAS-3");
    assert_eq!(f[f.len() - 2..], [("issuetype".into(), s("Sub-task"), s("Task")), ("parent".into(), s("ATLAS-2"), None)]);
    let sp = w.create_sprint("ATLAS", None, None, None, None).unwrap();
    w.set_sprint(&["ATLAS-1".into()], Some(sp), "ann", t(4)).unwrap();
    set(&mut w, "ATLAS-3", Some("sub-task"), Some(Some("atlas-1"))).unwrap();
    let i = w.issue("ATLAS-3").unwrap();
    assert_eq!((i.issue_type.as_str(), i.parent.as_ref().map(ToString::to_string), i.sprint), ("Sub-task", Some("ATLAS-1".into()), Some(sp)));
    // Becoming an epic drops the parent.
    set(&mut w, "ATLAS-4", Some("Epic"), None).unwrap();
    assert_eq!(w.issue("ATLAS-4").unwrap().parent, None);
    // A no-op edit records nothing.
    let n = w.issue("ATLAS-4").unwrap().history.len();
    set(&mut w, "ATLAS-4", Some("Epic"), None).unwrap();
    assert_eq!(w.issue("ATLAS-4").unwrap().history.len(), n);
}

#[test]
fn sprint_complete_refuses_done_parents_with_open_subtasks() {
    let (mut w, s) = sprint_ws();
    w.create_issue(sub("ATLAS-1", "sub"), "ann", t(1)).unwrap(); // 5
    w.set_sprint(&["ATLAS-1".into(), "ATLAS-2".into()], Some(s), "ann", t(2)).unwrap();
    w.start_sprint(s, Some("Renamed on start"), t(1), t(9), None).unwrap();
    assert_eq!(w.sprint(s).unwrap().name, "Renamed on start");
    w.transition("ATLAS-1", "Done", "ann", t(3)).unwrap();
    let e = w.complete_sprint(s, MoveOpenTo::Backlog, "ann", t(9)).unwrap_err().to_string();
    assert!(e.contains("ATLAS-1 (ATLAS-5)"), "{e}");
    w.transition("ATLAS-5", "Done", "ann", t(4)).unwrap();
    w.complete_sprint(s, MoveOpenTo::Backlog, "ann", t(9)).unwrap();
}

#[test]
fn sprint_dates_clear_only_on_future_sprints() {
    let (mut w, s) = sprint_ws();
    w.edit_sprint(s, SprintEdit { start: Some(Some(t(1))), end: Some(Some(t(9))), ..SprintEdit::default() }).unwrap();
    w.edit_sprint(s, SprintEdit { end: Some(None), ..SprintEdit::default() }).unwrap();
    assert_eq!((w.sprint(s).unwrap().start, w.sprint(s).unwrap().end), (Some(t(1)), None));
    w.set_sprint(&["ATLAS-1".into()], Some(s), "ann", t(2)).unwrap();
    w.start_sprint(s, None, t(1), t(9), None).unwrap();
    assert!(w.edit_sprint(s, SprintEdit { start: Some(None), ..SprintEdit::default() }).is_err());
    w.edit_sprint(s, SprintEdit { end: Some(Some(t(10))), ..SprintEdit::default() }).unwrap();
}

#[test]
fn workspace_rename_and_user_edit() {
    let mut w = ws();
    w.rename("  New name ").unwrap();
    assert_eq!(w.name, "New name");
    assert!(w.rename("").is_err());
    assert!(w.rename("a\nb").is_err());
    w.edit_user("ann", Some("Ann Lee"), Some(Some("ann@example.com"))).unwrap();
    assert_eq!((w.users["ann"].display_name.as_str(), w.users["ann"].email.as_deref()), ("Ann Lee", Some("ann@example.com")));
    let long = format!("{}@x.org", "a".repeat(300));
    for bad in ["nope", "a@b", "@x.org", "a b@x.org", "a@@x.org", "a@x.org.", long.as_str()] {
        assert!(w.edit_user("ann", None, Some(Some(bad))).is_err(), "{bad}");
    }
    w.edit_user("ann", None, Some(None)).unwrap();
    assert_eq!(w.users["ann"].email, None);
    assert!(w.edit_user("nobody", Some("x"), None).is_err());
    assert!(w.edit_user("ann", Some(" "), None).is_err());
}
