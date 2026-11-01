//! The issue view.

use super::*;
use crate::View;

#[test]
fn issue_keys_assign_comment_and_edit() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-3")));
    h.run_steps(3);
    assert_ne!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("riley"));
    h.key_press(egui::Key::I);
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("riley"));
    // `m` focuses the comment box; typing there is text, not shortcuts.
    h.key_press(egui::Key::M);
    h.run_steps(3);
    h.key_press(egui::Key::E);
    h.event(egui::Event::Text("e".into()));
    h.run_steps(2);
    assert_eq!(h.state().issue_ui.comment, "e");
    assert!(h.state().issue_ui.editing_description.is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.key_press(egui::Key::E);
    h.run_steps(3);
    assert!(h.state().issue_ui.editing_description.is_some());
    // Escape cancels the edit.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().issue_ui.editing_description.is_none());
}
