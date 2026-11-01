//! M2.5: reports (`report.*`) over hand-built histories with a fixed clock — scope added
//! mid-sprint, issues removed, estimates changed, reopened issues, renamed sprints — plus
//! permission filtering and hostile params.

use chrono::{Duration, TimeZone};
use serde_json::{Value, json};
use trackcraft_model::{MoveOpenTo, NewIssue, ProjectKind, SprintEdit, Timestamp, Workspace};

use crate::{Session, catalog, command};

fn day(d: i64, hour: u32) -> Timestamp {
    chrono::Utc.with_ymd_and_hms(2026, 10, 1, hour, 0, 0).single().unwrap() + Duration::days(d)
}

fn now() -> Timestamp {
    day(12, 15)
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.run(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn keys(k: &[&str]) -> Vec<String> {
    k.iter().map(|s| s.to_string()).collect()
}

/// Project ATL: sprint 1 (renamed "Rocket" on day 1) ran days 0–9; sprint 2 is active since day 10.
///
/// | when | what | remaining (points) |
/// |---|---|---|
/// | day 0 | ATL-1 (3), ATL-2 (5), ATL-3 (2) planned, sprint starts | 10 |
/// | day 2 | ATL-1 done | 7 |
/// | day 3 | ATL-4 (8) added | 15 |
/// | day 4 | ATL-3 removed | 13 |
/// | day 5 | ATL-2 re-estimated 5 → 3 | 11 |
/// | day 6 | ATL-1 reopened | 14 |
/// | day 7 | ATL-1 done again | 11 |
/// | day 9 | completed; ATL-2 and ATL-4 move to sprint 2 | 11 |
struct Fx {
    ws: Workspace,
    s1: u64,
    s2: u64,
}

fn fixture() -> Fx {
    let mut w = Workspace::new("Reports");
    w.add_user("ann", "Ann Example").unwrap();
    w.add_user("bo", "Bo Example").unwrap();
    w.create_project("ATL", "Atlas", ProjectKind::Software, Some("ann")).unwrap();
    for (summary, pts) in [("One", 3.0), ("Two", 5.0), ("Three", 2.0), ("Four", 8.0)] {
        let n =
            NewIssue { project: "ATL".into(), summary: summary.into(), story_points: Some(pts), assignee: Some("ann".into()), ..NewIssue::default() };
        w.create_issue(n, "ann", day(-2, 9)).unwrap();
    }
    let s1 = w.create_sprint("ATL", None, None, None, None).unwrap();
    w.set_sprint(&keys(&["ATL-1", "ATL-2", "ATL-3"]), Some(s1), "ann", day(0, 9)).unwrap();
    w.start_sprint(s1, None, day(0, 10), day(10, 10), None).unwrap();
    w.edit_sprint(s1, SprintEdit { name: Some("Rocket".into()), ..SprintEdit::default() }).unwrap();
    w.transition("ATL-1", "In Progress", "ann", day(1, 12)).unwrap();
    w.transition("ATL-1", "Done", "ann", day(2, 12)).unwrap();
    w.set_sprint(&keys(&["ATL-4"]), Some(s1), "ann", day(3, 12)).unwrap();
    w.set_sprint(&keys(&["ATL-3"]), None, "ann", day(4, 12)).unwrap();
    w.set_story_points("ATL-2", Some(3.0), "ann", day(5, 12)).unwrap();
    w.transition("ATL-1", "To Do", "ann", day(6, 12)).unwrap();
    w.transition("ATL-1", "Done", "ann", day(7, 12)).unwrap();
    let s2 = w.create_sprint("ATL", None, None, None, None).unwrap();
    w.complete_sprint(s1, MoveOpenTo::Sprint(s2), "ann", day(9, 12)).unwrap();
    w.start_sprint(s2, None, day(10, 9), day(17, 9), None).unwrap();
    w.transition("ATL-2", "Done", "ann", day(11, 12)).unwrap();
    // After the fact: none of this may change sprint 1's report.
    w.set_story_points("ATL-1", Some(13.0), "ann", day(12, 9)).unwrap();
    w.transition("ATL-3", "Done", "ann", day(12, 10)).unwrap();
    Fx { ws: w, s1: s1.0, s2: s2.0 }
}

fn session(ws: Workspace) -> Session {
    Session::new(ws, "ann").with_clock(now)
}

fn nums(rows: &Value, field: &str) -> Vec<f64> {
    rows.as_array().unwrap().iter().map(|r| r[field].as_f64().unwrap()).collect()
}

#[test]
fn catalog_floor_with_reports() {
    for id in ["report.burndown", "report.velocity", "report.cfd", "report.createdResolved", "report.pie"] {
        assert!(command(id).is_some_and(|c| !c.mutates), "{id} missing or not a query");
    }
    for row in ["Burndown chart", "Velocity chart", "Cumulative flow diagram", "Dashboards and gadgets"] {
        assert!(catalog::FEATURES.iter().any(|f| f.name == row && !f.command.is_empty()), "{row} not mapped");
    }
    let (done, total) = catalog::coverage();
    assert!(done >= 71, "catalog coverage fell to {done}/{total}");
}

#[test]
fn burndown_of_a_closed_sprint_follows_its_history() {
    let f = fixture();
    let mut s = session(f.ws);
    let b = run(&mut s, "report.burndown", json!({"sprint": f.s1}));
    assert_eq!(b["unit"], "points");
    assert_eq!(b["committed"], 10.0);
    assert_eq!(b["sprint"]["name"], "Rocket");
    // Oct 1 … Oct 10 (completion day), each at the end of the day.
    assert_eq!(nums(&b["days"], "remaining"), vec![10.0, 10.0, 7.0, 15.0, 13.0, 11.0, 14.0, 11.0, 11.0, 11.0]);
    let days = b["days"].as_array().unwrap();
    assert_eq!(days[0]["date"], "2026-10-01");
    assert_eq!(days[9]["date"], "2026-10-10");
    assert_eq!(days[9]["at"], "2026-10-10T12:00:00Z", "the last point is the completion");
    assert_eq!((b["remaining"].as_f64(), b["done"].as_f64(), b["scope"].as_f64()), (Some(11.0), Some(3.0), Some(14.0)));
    // Ideal: committed at the start, zero at the end date.
    let ideal = b["ideal"].as_array().unwrap();
    assert_eq!(ideal.first().unwrap()["remaining"], 10.0);
    assert_eq!(ideal.last().unwrap()["remaining"], 0.0);
    assert_eq!(ideal.last().unwrap()["at"], "2026-10-11T10:00:00Z");
    let kinds: Vec<(String, String, f64)> = b["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["key"].as_str().unwrap().into(), e["kind"].as_str().unwrap().into(), e["remainingDelta"].as_f64().unwrap()))
        .collect();
    let want = [
        ("ATL-1", "completed", -3.0),
        ("ATL-4", "added", 8.0),
        ("ATL-3", "removed", -2.0),
        ("ATL-2", "estimateChanged", -2.0),
        ("ATL-1", "reopened", 3.0),
        ("ATL-1", "completed", -3.0),
    ];
    assert_eq!(kinds, want.iter().map(|(k, w, d)| (k.to_string(), w.to_string(), *d)).collect::<Vec<_>>());
    let est = b["events"].as_array().unwrap().iter().find(|e| e["kind"] == "estimateChanged").unwrap();
    assert_eq!((est["from"].as_f64(), est["to"].as_f64()), (Some(5.0), Some(3.0)));
    assert_eq!(b["truncated"], false);
}

#[test]
fn burndown_counts_issues_and_follows_the_active_sprint() {
    let f = fixture();
    let mut s = session(f.ws);
    let b = run(&mut s, "report.burndown", json!({"sprint": f.s1, "unit": "issues"}));
    assert_eq!(b["committed"], 3.0);
    assert_eq!(nums(&b["days"], "remaining"), vec![3.0, 3.0, 2.0, 3.0, 2.0, 2.0, 3.0, 2.0, 2.0, 2.0]);
    let a = run(&mut s, "report.burndown", json!({"sprint": "active", "project": "atl"}));
    assert_eq!(a["sprint"]["id"], f.s2);
    // ATL-2 (3) and ATL-4 (8) moved in at the completion, before sprint 2 started; ATL-2 done day 11.
    assert_eq!(a["committed"], 11.0);
    assert_eq!(nums(&a["days"], "remaining"), vec![11.0, 8.0, 8.0]);
    assert_eq!(a["days"].as_array().unwrap().last().unwrap()["at"], "2026-10-13T15:00:00Z", "an active sprint runs to now");
    assert!(a.get("completed").is_none());
}

#[test]
fn velocity_compares_commitment_with_completion() {
    let f = fixture();
    let mut s = session(f.ws.clone());
    let v = run(&mut s, "report.velocity", json!({"project": "ATL"}));
    let rows = v["sprints"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "only closed sprints");
    assert_eq!((rows[0]["committed"].as_f64(), rows[0]["completedValue"].as_f64()), (Some(10.0), Some(3.0)));
    assert_eq!(v["average"], 3.0);
    let v = run(&mut s, "report.velocity", json!({"project": "ATL", "unit": "issues", "sprints": 0}));
    assert_eq!((v["sprints"][0]["committed"].as_f64(), v["sprints"][0]["completedValue"].as_f64()), (Some(3.0), Some(1.0)));
    // A project without closed sprints: an empty report, not an error.
    let mut s = Session::new(crate::sample::workspace(now()), "riley").with_clock(now);
    let v = run(&mut s, "report.velocity", json!({"project": "LUME"}));
    assert!(v["sprints"].as_array().unwrap().is_empty());
    assert_eq!(v["average"], 0.0);
}

#[test]
fn cumulative_flow_and_created_vs_resolved() {
    let f = fixture();
    let mut s = session(f.ws);
    let c = run(&mut s, "report.cfd", json!({"project": "ATL", "from": "2026-09-28", "to": "2026-10-03"}));
    let days = c["days"].as_array().unwrap();
    assert_eq!(days.len(), 6);
    assert_eq!((days[0]["toDo"].as_u64(), days[0]["done"].as_u64()), (Some(0), Some(0)), "before anything was created");
    assert_eq!((days[1]["toDo"].as_u64(), days[1]["inProgress"].as_u64()), (Some(4), Some(0)));
    assert_eq!((days[4]["toDo"].as_u64(), days[4]["inProgress"].as_u64()), (Some(3), Some(1)), "Oct 2: ATL-1 in progress");
    assert_eq!((days[5]["toDo"].as_u64(), days[5]["done"].as_u64()), (Some(3), Some(1)), "Oct 3: ATL-1 done");
    let d = run(&mut s, "report.cfd", json!({"project": "ATL"}));
    assert_eq!((d["from"].as_str(), d["to"].as_str()), (Some("2026-09-14"), Some("2026-10-13")));
    let last = d["days"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["toDo"].as_u64(), last["done"].as_u64()), (Some(1), Some(3)));

    let r = run(&mut s, "report.createdResolved", json!({"project": "ATL", "days": 15}));
    assert_eq!(r["days"].as_array().unwrap().len(), 15);
    assert_eq!((r["created"].as_u64(), r["resolved"].as_u64()), (Some(4), Some(3)));
    let rows = r["days"].as_array().unwrap();
    let on = |date: &str, f: &str| rows.iter().find(|x| x["date"] == date).unwrap()[f].as_u64().unwrap();
    assert_eq!(on("2026-09-29", "created"), 4);
    // Resolution date is the current one: ATL-1's (day 7), not its first (day 2).
    assert_eq!((on("2026-10-03", "resolved"), on("2026-10-08", "resolved")), (0, 1));
    let j = run(&mut s, "report.createdResolved", json!({"jql": "statusCategory = Done", "days": 3}));
    assert_eq!((j["created"].as_u64(), j["resolved"].as_u64()), (Some(0), Some(2)));
}

#[test]
fn pie_counts_by_field() {
    let f = fixture();
    let mut s = session(f.ws);
    let p = run(&mut s, "report.pie", json!({"jql": "project = ATL", "field": "status"}));
    assert_eq!(p["total"], 4);
    assert_eq!(p["slices"][0]["name"], "Done");
    assert_eq!((p["slices"][0]["count"].as_u64(), p["slices"][0]["percent"].as_f64()), (Some(3), Some(75.0)));
    let a = run(&mut s, "report.pie", json!({"jql": "", "field": "assignee"}));
    assert_eq!(a["slices"][0]["name"], "Ann Example");
    let l = run(&mut s, "report.pie", json!({"jql": "project = ATL", "field": "labels"}));
    assert_eq!(l["slices"][0]["name"], "No labels");
    assert!(s.run("report.pie", &json!({"jql": "", "field": "reporter"})).is_err());
}

#[test]
fn sample_workspace_reports() {
    let mut s = Session::new(crate::sample::workspace(now()), "riley").with_clock(now);
    let b = run(&mut s, "report.burndown", json!({"sprint": "active", "project": "LUME"}));
    // LUME-2 (5), LUME-3 (2), LUME-4 (1); the sub-task LUME-7 is left out.
    assert_eq!(b["committed"], 8.0);
    assert_eq!(b["days"].as_array().unwrap().len(), 6);
    let future = s.ws.sprints.values().find(|sp| sp.state == trackcraft_model::SprintState::Future).unwrap().id.0;
    let e = s.run("report.burndown", &json!({"sprint": future})).unwrap_err();
    assert!(e.contains("hasn't started"), "{e}");
    let e = s.run("report.burndown", &json!({"sprint": "active", "project": "OPS"})).unwrap_err();
    assert!(e.contains("no active sprint"), "{e}");
}

/// Reports count only what the acting user may browse: with enforcement on, queries run on
/// their filtered view, so a private project's sprints, issues and history are invisible.
#[test]
fn reports_only_count_what_the_user_can_browse() {
    let f = fixture();
    let mut ws = f.ws;
    ws.set_workspace_role("bo", trackcraft_model::WorkspaceRole::Member).unwrap();
    ws.create_project("PUB", "Public", ProjectKind::Software, Some("ann")).unwrap();
    ws.create_issue(NewIssue { project: "PUB".into(), summary: "Open".into(), ..NewIssue::default() }, "ann", day(11, 9)).unwrap();
    ws.set_project_access("ATL", trackcraft_model::AccessLevel::Private).unwrap();
    let mut s = session(ws);
    s.set_enforce(true);
    s.user = "ann".into();
    assert_eq!(run(&mut s, "report.pie", json!({"jql": "", "field": "type"}))["total"], 5);
    s.user = "bo".into();
    let e = s.run("report.burndown", &json!({"sprint": f.s1})).unwrap_err();
    assert!(e.contains("no sprint"), "hidden reads as missing: {e}");
    assert!(s.run("report.velocity", &json!({"project": "ATL"})).unwrap_err().contains("no project"));
    assert!(s.run("report.cfd", &json!({"project": "ATL"})).is_err());
    assert!(s.run("report.createdResolved", &json!({"project": "ATL"})).is_err());
    assert_eq!(run(&mut s, "report.pie", json!({"jql": "", "field": "type"}))["total"], 1);
    let r = run(&mut s, "report.createdResolved", json!({"jql": "project = ATL OR project = PUB", "days": 30}));
    assert_eq!((r["created"].as_u64(), r["resolved"].as_u64()), (Some(1), Some(0)));
    assert!(s.run("report.burndown", &json!({"sprint": f.s2})).is_err());
}

#[test]
fn hostile_report_params() {
    let junk = [
        json!({"sprint": u64::MAX}),
        json!({"sprint": -1, "unit": 7}),
        json!({"sprint": "active"}),
        json!({"sprint": "active", "project": "NOPE"}),
        json!({"sprint": {}, "project": []}),
        json!({"sprint": 1, "unit": "furlongs"}),
        json!({"project": "ATL", "sprints": u64::MAX}),
        json!({"project": "ATL", "sprints": -3, "unit": null}),
        json!({"project": "ATL", "from": "0001-01-01", "to": "9999-12-31"}),
        json!({"project": "ATL", "from": "2026-10-05", "to": "2026-10-01"}),
        json!({"project": "ATL", "from": "+262142-12-31", "to": "+262142-12-31"}),
        json!({"project": "ATL", "from": "262142-12-31", "to": "262142-12-31"}),
        json!({"project": "ATL", "from": 5, "to": []}),
        json!({"project": "ATL", "to": "-0001-01-01"}),
        json!({"project": "ATL", "days": u64::MAX}),
        json!({"project": "ATL", "days": 0}),
        json!({"jql": "(".repeat(10_000), "days": 3}),
        json!({"jql": 7, "field": 3}),
        json!({"jql": "", "field": "x".repeat(100_000)}),
        json!({"project": "", "jql": ""}),
        json!({"project": "ATL", "jql": ") OR (project = ATL"}),
    ];
    let ids = ["report.burndown", "report.velocity", "report.cfd", "report.createdResolved", "report.pie"];
    let f = fixture();
    for id in ids {
        for p in &junk {
            let mut s = session(f.ws.clone());
            let before = s.ws.clone();
            if let Err(e) = s.run(id, p) {
                assert!(!e.starts_with("internal error"), "{id} panicked on {p}: {e}");
            }
            assert!(s.ws == before && !s.dirty && !s.can_undo(), "{id} changed something");
        }
    }
    // A sprint with extreme dates still answers within the day cap.
    let mut ws = f.ws.clone();
    let far = chrono::Utc.with_ymd_and_hms(9000, 1, 1, 0, 0, 0).single().unwrap();
    let id = ws.create_sprint("ATL", Some("Long"), None, None, None).unwrap();
    ws.complete_sprint(trackcraft_core::Id(f.s2), MoveOpenTo::Backlog, "ann", day(12, 10)).unwrap();
    ws.set_sprint(&keys(&["ATL-4"]), Some(id), "ann", day(12, 11)).unwrap();
    ws.start_sprint(id, None, chrono::Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0).single().unwrap(), far, None).unwrap();
    let mut s = session(ws);
    let b = run(&mut s, "report.burndown", json!({"sprint": id.0}));
    assert_eq!(b["days"].as_array().unwrap().len(), crate::cmd::report::MAX_DAYS);
    assert_eq!(b["truncated"], true);
}
