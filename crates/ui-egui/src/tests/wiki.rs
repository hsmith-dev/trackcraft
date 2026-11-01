//! Pages: editing and save conflicts.

use serde_json::json;

use super::*;
use crate::View;

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

#[test]
fn page_e_edits_but_not_while_typing() {
    let mut h = harness(app());
    let page = h.state().session.ws.pages.values().find(|p| p.parent.is_some()).map(|p| p.id).unwrap();
    h.state_mut().navigate(View::Page(page));
    h.run_steps(3);
    h.key_press(egui::Key::Slash);
    h.run_steps(3);
    h.key_press(egui::Key::E);
    h.run_steps(2);
    assert!(h.state().page_edit.is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.key_press(egui::Key::E);
    h.run_steps(2);
    assert!(h.state().page_edit.is_some());
}
