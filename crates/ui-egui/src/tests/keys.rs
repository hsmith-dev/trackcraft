//! Keyboard: the help overlay, typing vs shortcuts, injected keys.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;

use super::*;

#[test]
fn question_mark_opens_help_and_escape_closes_it() {
    let mut h = harness(app());
    h.run_steps(2);
    h.key_press(egui::Key::Questionmark);
    h.run_steps(2);
    assert!(h.state().help);
    // Single keys are off behind the overlay.
    h.key_press(egui::Key::C);
    h.run_steps(2);
    assert!(h.state().create.is_none());
    // Every shortcut in the table is listed.
    for (_, _, what) in crate::keys::SHORTCUTS {
        assert!(h.query_all_by_label(what).next().is_some(), "{what} missing from the overlay");
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.state().help);
    // Shift+/ from backends that report the physical key.
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Slash);
    h.run_steps(2);
    assert!(h.state().help);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // The palette entry opens it too.
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text("keyboard shortcuts".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().palette.is_none());
    assert!(h.state().help);
}

#[test]
fn typing_in_the_search_box_does_not_trigger_shortcuts() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME"}})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::Slash);
    h.run_steps(3);
    for (k, text) in [(egui::Key::J, "j"), (egui::Key::C, "c"), (egui::Key::Questionmark, "?"), (egui::Key::K, "k")] {
        h.key_press(k);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(2);
    }
    assert!(h.state().list_sel.is_none());
    assert!(h.state().create.is_none());
    assert!(!h.state().help);
    assert_eq!(h.state().jql, "project = LUMEjc?k");
}

#[test]
fn control_key_injects_presses() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    h.state_mut().control(&json!({"method": "ui.key", "params": {"key": "ArrowDown"}})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().list_sel, Some(key("LUME-1")));
    h.state_mut().control(&json!({"method": "ui.key", "params": {"key": "?"}})).unwrap();
    h.run_steps(2);
    assert!(h.state().help);
    assert!(h.state_mut().control(&json!({"method": "ui.key", "params": {"key": "NotAKey"}})).is_err());
}
