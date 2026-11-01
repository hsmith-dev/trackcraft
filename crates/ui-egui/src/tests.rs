//! Interface tests: every screen renders in both themes, and UI actions go through commands.

use egui_kittest::Harness;
use serde_json::json;
use trackcraft_engine::Session;

use crate::{TrackApp, View};

fn fixed_now() -> trackcraft_model::Timestamp {
    trackcraft_model::Timestamp::from_timestamp(1_793_448_000, 0).unwrap_or_default()
}

fn app() -> TrackApp {
    let ws = trackcraft_engine::sample::workspace(fixed_now());
    TrackApp::new(Session::new(ws, "riley").with_clock(fixed_now))
}

fn harness(app: TrackApp) -> Harness<'static, TrackApp> {
    Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut TrackApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

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

#[test]
fn search_shows_jql_errors_without_panicking() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "status = "}})).unwrap();
    h.run_steps(3);
    assert!(h.state().jql_result.is_err());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "assignee = currentUser()"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().jql_result.as_ref().map(Vec::len), Ok(1));
}

#[test]
fn stale_page_save_offers_a_choice() {
    let mut h = harness(app());
    let page = h.state().session.ws.pages.values().find(|p| p.parent.is_some()).map(|p| p.id).unwrap();
    h.state_mut().control(&json!({"method": "ui.editPage", "params": {"id": page.0}})).unwrap();
    h.run_steps(2);
    // Someone else saves meanwhile.
    h.state_mut().session.run("page.edit", &json!({"id": page.0, "body": "theirs"})).unwrap();
    let base = h.state().page_edit.as_ref().map(|e| e.base).unwrap();
    h.state_mut().run("page.edit", &json!({"id": page.0, "body": "mine", "baseVersion": base}), crate::Then::EndPageEdit);
    assert_eq!(h.state().page_edit.as_ref().and_then(|e| e.conflict), Some(base + 1));
    h.run_steps(2);
}
