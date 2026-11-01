//! The JQL issue list.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;

use super::*;
use crate::View;

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
fn list_selection_moves_with_keys_and_enter_opens() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::J);
    h.run_steps(2);
    assert_eq!(h.state().list_sel, Some(key("LUME-1")));
    h.key_press(egui::Key::J);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(h.state().list_sel, Some(key("LUME-3")));
    h.key_press(egui::Key::K);
    h.run_steps(2);
    assert_eq!(h.state().list_sel, Some(key("LUME-2")));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().view, View::Issue(key("LUME-2")));
    // Back on the list, the selection is still there.
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(h.state().view, View::Issues);
    assert_eq!(h.state().list_sel, Some(key("LUME-2")));
}

#[test]
fn clicking_a_sort_header_rewrites_order_by() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY priority"}})).unwrap();
    h.run_steps(3);
    h.get_by_label("Summary").click();
    h.run_steps(3);
    assert_eq!(h.state().jql, "project = LUME ORDER BY summary ASC");
    h.get_by_label("Summary").click();
    h.run_steps(3);
    assert_eq!(h.state().jql, "project = LUME ORDER BY summary DESC");
    assert_eq!(h.state().jql_result.as_ref().map(Vec::len), Ok(7));
}

/// The sample workspace plus `n` generated LUME issues (model primitives, so it's fast).
fn big_app(n: usize) -> crate::TrackApp {
    let mut ws = trackcraft_engine::sample::workspace(fixed_now());
    for i in 0..n {
        let new = trackcraft_model::NewIssue {
            project: "LUME".into(),
            summary: format!("Generated issue number {i} with a summary long enough to need truncating in a narrow column"),
            ..Default::default()
        };
        ws.create_issue(new, "riley", fixed_now()).unwrap();
    }
    crate::TrackApp::new(trackcraft_engine::Session::new(ws, "riley").with_clock(fixed_now))
}

fn rows_laid_out(h: &egui_kittest::Harness<'static, crate::TrackApp>) -> usize {
    h.query_all_by(|n| n.label().is_some_and(|l| l.starts_with("Open "))).count()
}

#[test]
fn list_draws_only_visible_rows_with_10k() {
    let mut h = harness(big_app(10_000));
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().jql_total, 10_007);
    assert_eq!(h.state().jql_result.as_ref().map(Vec::len), Ok(crate::LIST_PAGE));
    let rows = rows_laid_out(&h);
    assert!(rows > 5 && rows < 60, "{rows} rows laid out");
    // A cached frame stays cheap with a full page (loose bound: debug builds, shared CI).
    let t0 = std::time::Instant::now();
    h.run_steps(5);
    assert!(t0.elapsed() < std::time::Duration::from_secs(2), "5 frames took {:?}", t0.elapsed());
}

#[test]
fn list_paging_shows_range_and_next_advances() {
    let mut h = harness(big_app(2_500));
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    h.get_by_label("1\u{2013}1 000 of 2 507");
    h.get_by_label("Next").click();
    h.run_steps(3);
    h.get_by_label("1 001\u{2013}2 000 of 2 507");
    assert_eq!(h.state().jql_result.as_ref().ok().and_then(|k| k.first().cloned()), Some(key("LUME-1001")));
    h.get_by_label("Next").click();
    h.run_steps(3);
    h.get_by_label("2 001\u{2013}2 507 of 2 507");
    h.get_by_label("Prev").click();
    h.run_steps(3);
    h.get_by_label("1 001\u{2013}2 000 of 2 507");
    // A new query starts on its first page.
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key DESC"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().jql_start, 0);
    // Small results have no pager.
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "key = LUME-2"}})).unwrap();
    h.run_steps(3);
    h.get_by_label("1 issue");
    assert!(h.query_by_label("Next").is_none());
}

#[test]
fn list_selection_survives_paging() {
    let mut h = harness(big_app(1_500));
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::J);
    h.key_press(egui::Key::J);
    h.run_steps(2);
    assert_eq!(h.state().list_sel, Some(key("LUME-2")));
    h.get_by_label("Next").click();
    h.run_steps(3);
    assert_eq!(h.state().list_sel, Some(key("LUME-2")));
    h.get_by_label("Prev").click();
    h.run_steps(3);
    assert_eq!(h.state().list_sel, Some(key("LUME-2")));
    h.key_press(egui::Key::J);
    h.run_steps(2);
    assert_eq!(h.state().list_sel, Some(key("LUME-3")));
}

#[test]
fn list_selection_scrolls_into_view() {
    let mut h = harness(big_app(300));
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Open LUME-200").is_none());
    for _ in 0..200 {
        h.key_press(egui::Key::J);
    }
    h.run_steps(4);
    assert_eq!(h.state().list_sel, Some(key("LUME-200")));
    h.get_by_label("Open LUME-200");
}

#[test]
fn column_chooser_toggles_a_column() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Reporter").is_none());
    h.get_by_label("Columns").click();
    h.run_steps(2);
    h.get_by_label("Reporter").click();
    h.run_steps(3);
    // The checkbox and the new header.
    assert!(h.query_all_by_label("Reporter").count() >= 2);
    assert!(h.query_all_by_label("Assignee").count() >= 2);
    h.get_by(|n| n.label().as_deref() == Some("Assignee") && n.role() == egui::accesskit::Role::CheckBox).click();
    h.run_steps(3);
    // Kept in egui memory, so it survives leaving the list.
    h.key_press(egui::Key::Escape);
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "home"}})).unwrap();
    h.run_steps(2);
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "issues"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.query_all_by_label("Reporter").count(), 1);
    assert!(h.query_by_label("Assignee").is_none());
}

#[test]
fn list_empty_state_offers_clear_filter() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "summary ~ nothingmatchesthis ORDER BY key"}})).unwrap();
    h.run_steps(3);
    h.get_by_label("No issues match this search");
    h.get_by_label("Clear filter").click();
    h.run_steps(3);
    assert_eq!(h.state().jql, "ORDER BY key");
    assert!(h.state().jql_result.as_ref().is_ok_and(|k| !k.is_empty()));
}

#[test]
fn list_jql_error_shows_banner() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "status = "}})).unwrap();
    h.run_steps(3);
    let err = h.state().jql_result.clone().unwrap_err();
    h.get_by_label(&err);
    assert!(h.query_by_label("Columns").is_some());
}
