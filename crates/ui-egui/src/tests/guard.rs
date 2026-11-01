//! The unsaved-edit guard (UI-1).

use egui_kittest::kittest::Queryable as _;
use serde_json::json;

use super::*;
use crate::View;

#[test]
fn leaving_a_dirty_page_editor_asks_and_keep_editing_keeps_the_text() {
    let mut h = harness(app());
    let page = dirty_page_editor(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Home").click();
    h.run_steps(3);
    assert!(h.state().guard.is_some());
    assert_eq!(h.state().view, View::Page(page));
    h.get_by_label("Discard your changes?");
    h.get_by_label("Keep editing").click();
    h.run_steps(3);
    assert!(h.state().guard.is_none());
    assert_eq!(h.state().view, View::Page(page));
    assert!(h.state().page_edit.as_ref().unwrap().body.ends_with("Typed but not published."));
    // Escape on the dialog is "Keep editing" too, and does not cancel the editor underneath.
    h.state_mut().push(crate::Action::Nav(View::Issues));
    h.run_steps(3);
    assert!(h.state().guard.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().guard.is_none());
    assert!(h.state().page_edit.is_some());
    // An untouched editor leaves without asking.
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.editPage", "params": {"id": page.0}})).unwrap();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Home").click();
    h.run_steps(3);
    assert!(h.state().guard.is_none());
    assert_eq!(h.state().view, View::Home);
}

#[test]
fn discard_lets_the_navigation_through() {
    let mut h = harness(app());
    dirty_page_editor(&mut h);
    // A palette jump is held the same way.
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text("lume-2".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().guard.is_some());
    h.get_by_label("Discard").click();
    h.run_steps(3);
    assert!(h.state().guard.is_none());
    assert!(h.state().page_edit.is_none());
    assert_eq!(h.state().view, View::Issue(key("LUME-2")));
}

#[test]
fn back_is_guarded_for_issue_edits() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issues);
    h.state_mut().navigate(View::Issue(key("LUME-2")));
    h.run_steps(3);
    h.key_press(egui::Key::E);
    h.run_steps(3);
    h.state_mut().issue_ui.editing_description = Some("A new description".into());
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    h.run_steps(3);
    assert!(h.state().guard.is_some());
    assert_eq!(h.state().view, View::Issue(key("LUME-2")));
    h.get_by_label("Keep editing").click();
    h.run_steps(3);
    assert_eq!(h.state().issue_ui.editing_description.as_deref(), Some("A new description"));
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    h.run_steps(3);
    h.get_by_label("Discard").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issues);
    assert!(h.state().issue_ui.editing_description.is_none());
    // An open but unchanged summary edit is not "changes".
    h.state_mut().navigate(View::Issue(key("LUME-2")));
    let summary = h.state().session.ws.issues[&key("LUME-2")].summary.clone();
    h.state_mut().issue_ui.editing_summary = Some(summary);
    h.state_mut().push(crate::Action::Back);
    h.run_steps(3);
    assert!(h.state().guard.is_none());
    assert_eq!(h.state().view, View::Issues);
}

#[test]
fn escape_in_the_create_dialog_keeps_a_draft() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.create", "params": {"project": "LUME"}})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text("Half-typed summary".into()));
    h.run_steps(2);
    assert_eq!(h.state().create.as_ref().map(|f| f.summary.as_str()), Some("Half-typed summary"));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().create.is_some());
    assert!(h.state().guard.is_some());
    // Escape again keeps editing.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().guard.is_none());
    assert_eq!(h.state().create.as_ref().map(|f| f.summary.as_str()), Some("Half-typed summary"));
    // Discard closes it.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.get_by_label("Discard").click();
    h.run_steps(3);
    assert!(h.state().create.is_none());
    assert!(h.state().guard.is_none());
    // An empty dialog still closes on Escape, and Cancel always closes.
    h.state_mut().control(&json!({"method": "ui.create"})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().create.is_none());
    h.state_mut().control(&json!({"method": "ui.create"})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text("Something".into()));
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().create.is_none());
    assert!(h.state().guard.is_none());
}

#[test]
fn control_navigate_fails_on_unsaved_changes_unless_forced() {
    let mut h = harness(app());
    let page = dirty_page_editor(&mut h);
    let go = json!({"method": "ui.navigate", "params": {"view": "home"}});
    let e = h.state_mut().control(&go).unwrap_err();
    assert!(e.starts_with("unsaved changes"), "{e}");
    assert!(h.state().guard.is_none(), "programmatic calls never open dialogs");
    assert_eq!(h.state().view, View::Page(page));
    assert!(h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME"}})).is_err());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "home", "force": true}})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().view, View::Home);
    assert!(h.state().page_edit.is_none());
    // Nothing typed: no error.
    h.state_mut().control(&go).unwrap();
}
