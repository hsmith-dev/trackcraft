//! Navigation: every view renders, the control channel navigates.

use serde_json::json;

use super::*;
use crate::View;

#[test]
fn every_view_renders_in_both_themes() {
    for dark in [false, true] {
        let mut h = harness(app());
        h.state_mut().dark = Some(dark);
        let home = h.state().session.ws.spaces.values().next().map(|s| s.homepage).unwrap();
        let page = h.state().session.ws.pages.values().find(|p| p.parent.is_some()).map(|p| p.id).unwrap();
        let views = [
            View::Home,
            View::Issues,
            View::Board(trackcraft_core::ProjectKey::parse("LUME").unwrap()),
            View::Issue(trackcraft_core::IssueKey::parse("LUME-2").unwrap()),
            View::Issue(trackcraft_core::IssueKey::parse("LUME-4").unwrap()),
            View::Page(home),
            View::Page(page),
            View::PageHistory(page),
        ];
        for v in views {
            h.state_mut().navigate(v.clone());
            h.run_steps(3);
        }
        for m in ["ui.create", "ui.palette"] {
            h.state_mut().control(&json!({"method": m})).unwrap();
            h.run_steps(3);
        }
    }
}

#[test]
fn control_runs_commands_and_navigates() {
    let mut h = harness(app());
    let r = h
        .state_mut()
        .control(
            &json!({"method": "engine.execute", "params": {"command": "issue.create", "params": {"project": "LUME", "summary": "From the UI test"}}}),
        )
        .unwrap();
    assert_eq!(r["key"], "LUME-8");
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "issue", "key": "LUME-8"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issue(trackcraft_core::IssueKey::parse("LUME-8").unwrap()));
    // Undo removes the issue; the view falls back to Home instead of showing a ghost.
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": "edit.undo"}})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().view, View::Home);
    assert!(h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "page", "id": 999_999}})).is_err());
    assert!(h.state_mut().control(&json!({"method": "nope"})).is_err());
}
