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
