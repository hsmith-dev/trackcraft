//! Reports and the Home dashboard (UI-24): each report renders on the sample workspace in both
//! themes, empty states, the "Show data" table, and chart values matching the engine's.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::ProjectKey;

use super::*;
use crate::View;
use crate::charts;
use crate::reports::{self, Tab};
use crate::theme::Tokens;

fn lume() -> ProjectKey {
    ProjectKey::parse("LUME").unwrap()
}

fn on_reports(tab: &str) -> Harness<'static, TrackApp> {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "reports", "project": "LUME"}})).unwrap();
    h.state_mut().control(&json!({"method": "ui.reports", "params": {"tab": tab}})).unwrap();
    h.run_steps(3);
    h
}

#[test]
fn every_report_renders_in_both_themes() {
    for dark in [false, true] {
        let mut h = on_reports("burndown");
        h.state_mut().dark = Some(dark);
        for tab in ["burndown", "velocity", "cfd", "createdResolved"] {
            for data in [false, true] {
                h.state_mut().control(&json!({"method": "ui.reports", "params": {"tab": tab, "data": data, "unit": "issues"}})).unwrap();
                h.run_steps(3);
            }
        }
        h.state_mut().navigate(View::Home);
        h.run_steps(3);
        assert!(h.query_by_label("Active sprint").is_some());
        assert!(h.query_by_label("Created vs resolved (14 days)").is_some());
        assert!(h.query_by_label("Recently updated pages").is_some());
    }
}

#[test]
fn burndown_shows_the_active_sprint() {
    let h = on_reports("burndown");
    assert_eq!(h.state().view, View::Reports(lume()));
    assert!(h.query_by_label("8 of 8 points remaining \u{b7} committed 8 at the start \u{b7} sprint in progress").is_some());
    assert!(h.query_by_label_contains("Burndown chart of LUME Sprint 1").is_some(), "the chart has an accessible name");
    for legend in ["Remaining", "Scope", "Ideal"] {
        assert!(h.query_by_label(legend).is_some(), "{legend} in the legend");
    }
}

#[test]
fn show_data_lists_the_engine_values() {
    let mut h = on_reports("burndown");
    assert!(h.query_by_label("Sprint start, Oct 26").is_none(), "the table is hidden until asked for");
    h.get_by_label("Show data").click();
    h.run_steps(3);
    assert!(h.state().reports.show_data);
    assert!(h.query_by_label("Sprint start, Oct 26").is_some());
    // Every value in the table is the engine's.
    let r = h.state_mut().session.run("report.burndown", &json!({"sprint": "active", "project": "LUME"})).unwrap();
    let c = reports::burndown_chart(&Tokens::LIGHT, &r, false);
    let days = r["days"].as_array().unwrap();
    for (n, d) in days.iter().enumerate() {
        let lines = charts::tooltip_lines(&c, n + 1);
        let rem = lines.iter().find(|(k, _)| k == "Remaining").map(|(_, v)| v.clone()).unwrap();
        assert_eq!(rem, charts::num(d["remaining"].as_f64().unwrap()));
        let ideal = lines.iter().find(|(k, _)| k == "Ideal").map(|(_, v)| v.clone()).unwrap();
        assert_eq!(ideal, charts::num(d["ideal"].as_f64().unwrap()));
    }
    assert_eq!(charts::tooltip_lines(&c, 0).first().map(|(_, v)| v.clone()), Some("8".into()), "the start is the commitment");
    // The future days carry only the ideal.
    let last = c.x.len() - 1;
    assert_eq!(charts::tooltip_lines(&c, last), vec![("Ideal".to_string(), "0".to_string())]);
    // Toggling off hides it again.
    h.get_by_label("Show data").click();
    h.run_steps(3);
    assert!(h.query_by_label("Sprint start, Oct 26").is_none());
}

#[test]
fn hovering_the_chart_shows_exact_values() {
    let mut h = on_reports("createdResolved");
    h.get_by_label_contains("Created vs resolved chart").hover();
    // Tooltips wait for the pointer to rest.
    h.run_steps(60);
    assert!(h.query_by_label_contains("Created: ").is_some(), "the tooltip names each series with its value");
    assert!(h.query_by_label_contains("Resolved: ").is_some());
}

#[test]
fn created_vs_resolved_matches_the_engine() {
    let mut h = on_reports("createdResolved");
    let r = h.state_mut().session.run("report.createdResolved", &json!({"project": "LUME", "days": 30})).unwrap();
    let c = reports::created_resolved_chart(&Tokens::DARK, &r, false);
    for (i, d) in r["days"].as_array().unwrap().iter().enumerate() {
        let got = charts::tooltip_lines(&c, i);
        assert_eq!(got, vec![("Created".into(), d["created"].to_string()), ("Resolved".into(), d["resolved"].to_string())]);
    }
    let total = format!("{} created \u{b7} {} resolved in the last 30 days", r["created"], r["resolved"]);
    assert!(h.query_by_label(&total).is_some(), "{total}");
    // CFD and velocity build from the same results.
    let cfd = h.state_mut().session.run("report.cfd", &json!({"project": "LUME", "days": 30})).unwrap();
    let c = reports::cfd_chart(&Tokens::LIGHT, &cfd);
    let last = cfd["days"].as_array().unwrap().last().unwrap().clone();
    let lines = charts::tooltip_lines(&c, c.x.len() - 1);
    assert!(lines.contains(&("To Do".into(), last["toDo"].to_string())));
    assert!(lines.contains(&("Done".into(), last["done"].to_string())));
}

#[test]
fn empty_states() {
    // The sample has no completed sprints yet.
    let h = on_reports("velocity");
    assert!(h.query_by_label("No completed sprints yet").is_some());
    // OPS has never run a sprint.
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "reports", "project": "OPS"}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("No sprints to chart yet").is_some());
    h.get_by_label("Open backlog").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Backlog(ProjectKey::parse("OPS").unwrap()));
    // A new project has no history at all.
    let mut h = harness(app());
    h.state_mut()
        .control(&json!({"method": "engine.execute", "params": {"command": "project.create", "params": {"key": "NEW", "name": "Brand new"}}}))
        .unwrap();
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "reports", "project": "NEW"}})).unwrap();
    h.state_mut().control(&json!({"method": "ui.reports", "params": {"tab": "cfd"}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("No issues in this range").is_some());
    h.state_mut().control(&json!({"method": "ui.reports", "params": {"tab": "createdResolved"}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("0 created \u{b7} 0 resolved in the last 30 days").is_some());
    // A workspace without sprints: the Home gadget says so.
    h.state_mut().navigate(View::Home);
    let id = active(&h);
    h.state_mut()
        .control(&json!({"method": "engine.execute", "params": {"command": "sprint.complete", "params": {"id": id, "moveOpenTo": "backlog"}}}))
        .unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("No sprint is running. Start one from a project's backlog.").is_some());
}

fn active(h: &Harness<'static, TrackApp>) -> u64 {
    h.state().session.ws.active_sprint(&lume()).unwrap().id.0
}

#[test]
fn velocity_after_completing_a_sprint() {
    let mut h = on_reports("velocity");
    let id = active(&h);
    h.state_mut()
        .control(&json!({"method": "engine.execute", "params": {"command": "issue.transition", "params": {"key": "LUME-4", "to": "Done"}}}))
        .unwrap();
    h.state_mut()
        .control(&json!({"method": "engine.execute", "params": {"command": "sprint.complete", "params": {"id": id, "moveOpenTo": "new"}}}))
        .unwrap();
    h.state_mut().control(&json!({"method": "ui.reports", "params": {"data": true}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Average completed: 1 point over the last sprint").is_some());
    assert!(h.query_by_label("LUME Sprint 1").is_some(), "the sprint is a row of the table");
    // The closed sprint stays in the burndown's sprint picker.
    h.state_mut().control(&json!({"method": "ui.reports", "params": {"tab": "burndown", "sprint": id}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label_contains("sprint completed").is_some());
}

#[test]
fn sidebar_and_home_open_reports() {
    let mut h = harness(app());
    h.run_steps(2);
    h.get_all_by_label("Reports").next().unwrap().click();
    h.run_steps(3);
    assert!(matches!(h.state().view, View::Reports(_)));
    h.state_mut().navigate(View::Home);
    h.run_steps(3);
    h.get_by_label("Open reports").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Reports(lume()));
    assert_eq!(h.state().reports.tab, Tab::Burndown);
    // Bad requests are refused, not half-applied.
    assert!(h.state_mut().control(&json!({"method": "ui.reports", "params": {"tab": "nope"}})).is_err());
    assert!(h.state_mut().control(&json!({"method": "ui.reports", "params": {"unit": "furlongs"}})).is_err());
    assert!(h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "reports", "project": "not a key!"}})).is_err());
    // A deleted project's reports fall back to Home.
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "reports", "project": "ZZZ"}})).unwrap();
    h.run_steps(1);
    assert_eq!(h.state().view, View::Reports(ProjectKey::parse("ZZZ").unwrap()));
    h.run_steps(2);
    assert!(h.query_by_label("No such project").is_some());
}
