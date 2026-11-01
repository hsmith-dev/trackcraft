//! Toasts and the status bar (UI-5).

use egui_kittest::kittest::Queryable as _;

use super::*;
use crate::toast::{Kind, MAX_VISIBLE};
use crate::{Then, View};

fn on_board(h: &mut Harness<'static, TrackApp>) {
    h.state_mut().navigate(View::Board(trackcraft_core::ProjectKey::parse("LUME").unwrap()));
    h.run_steps(3);
}

/// Selects the first card and moves it one column right (Shift+ArrowRight); returns the card
/// and its status before.
fn transition_from_board(h: &mut Harness<'static, TrackApp>) -> (trackcraft_core::IssueKey, String) {
    on_board(h);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    let sel = h.state().board_sel.clone().unwrap();
    let before = h.state().session.ws.issues[&sel].status.clone();
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    h.run_steps(3);
    assert_ne!(h.state().session.ws.issues[&sel].status, before);
    (sel, before)
}

#[test]
fn failing_command_shows_error_toast() {
    let mut h = harness(app());
    let bad = json!({"key": "LUME-2", "to": "No such status"});
    let err = app().session.run("issue.transition", &bad).unwrap_err();
    h.state_mut().run("issue.transition", &bad, Then::Nothing);
    h.run_steps(3);
    let last = h.state().toasts.items.last().unwrap().clone();
    assert_eq!(last.kind, Kind::Error);
    assert_eq!(last.text, err);
    h.get_by_label(&err);
    h.get_by_label("Copy message");
}

#[test]
fn toast_undo_reverts_transition() {
    let mut h = harness(app());
    let (k, before) = transition_from_board(&mut h);
    let last = h.state().toasts.items.last().unwrap().clone();
    assert_eq!(last.kind, Kind::Success);
    h.get_by_label("Undo this change").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&k].status, before);
    // The toast is gone; an "Undone" note replaces it, without Undo of its own.
    assert!(h.query_by_label("Undo this change").is_none());
}

#[test]
fn stale_toast_hides_undo() {
    let mut h = harness(app());
    transition_from_board(&mut h);
    assert!(h.query_by_label("Undo this change").is_some());
    // A later change (through the control channel, no toast of its own) makes Undo stale.
    h.state_mut()
        .control(&json!({"method": "engine.execute", "params": {"command": "issue.edit", "params": {"key": "LUME-2", "summary": "Edited later"}}}))
        .unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Undo this change").is_none());

    // A view calling `Session::run` directly (no counter bump) is caught too.
    transition_from_board(&mut h);
    assert!(h.query_by_label("Undo this change").is_some());
    h.state_mut().session.run("issue.create", &json!({"project": "LUME", "summary": "Direct"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Undo this change").is_none());
}

#[test]
fn at_most_three_toasts() {
    let mut h = harness(app());
    for i in 0..5 {
        h.state_mut().notify(format!("Message {i}"));
    }
    h.run_steps(3);
    assert_eq!(h.state().toasts.items.len(), MAX_VISIBLE);
    assert_eq!(h.query_all_by_label("Dismiss").count(), MAX_VISIBLE);
    // Newest kept: the two oldest were pushed out.
    assert!(h.query_by_label("Message 0").is_none());
    h.get_by_label("Message 4");
    h.get_by_label("Message 4");
    // Dismiss closes one.
    h.query_all_by_label("Dismiss").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().toasts.items.len(), MAX_VISIBLE - 1);
}

#[test]
fn error_toast_persists_and_hover_pauses() {
    let mut a = app();
    a.notify("Info");
    a.notify_error("Broken");
    let t = &mut a.toasts;
    t.tick(0.0);
    for i in 1..=50 {
        t.tick(f64::from(i) * 0.1);
    }
    // 5 s in: the info toast went at 4 s, the error stays.
    assert_eq!(t.items.len(), 1);
    assert_eq!(t.items[0].kind, Kind::Error);
    // Hovered, it does not age.
    let id = t.items[0].id;
    t.set_hovered(id, true);
    for i in 51..=200 {
        t.tick(f64::from(i) * 0.1);
    }
    assert_eq!(t.items.len(), 1);
    t.set_hovered(id, false);
    for i in 201..=260 {
        t.tick(f64::from(i) * 0.1);
    }
    assert!(t.items.is_empty());
}

#[test]
fn status_bar_shows_unsaved_after_edit_and_file_after_save() {
    let mut h = harness(app());
    h.run_steps(2);
    h.get_by_label("Not saved yet");
    h.state_mut().run("issue.edit", &json!({"key": "LUME-2", "summary": "Changed"}), Then::Nothing);
    h.run_steps(2);
    h.get_by_label("Unsaved changes");

    let dir = std::env::temp_dir().join(format!("tc-ui5-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("status.track");
    h.state_mut().run("file.save", &json!({"path": path.display().to_string()}), Then::Nothing);
    h.run_steps(2);
    h.get_by_label("Saved");
    h.get_by_label("status.track");
    assert!(h.query_by_label("Unsaved changes").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_bar_context_follows_view() {
    let mut h = harness(app());
    on_board(&mut h);
    let n = h.state().session.ws.issues.keys().filter(|k| k.project.to_string() == "LUME").count();
    h.get_by_label(&format!("{n} issues on the LUME board"));
    h.state_mut().navigate(View::Issue(key("LUME-2")));
    h.run_steps(2);
    let status = h.state().session.ws.issues[&key("LUME-2")].status.clone();
    h.get_by_label(&format!("LUME-2 is {status}"));
}

#[test]
fn control_toast_method() {
    let mut a = app();
    a.control(&json!({"method": "ui.toast", "params": {"text": "Hello", "kind": "success"}})).unwrap();
    assert_eq!(a.toasts.items.last().unwrap().kind, Kind::Success);
    assert!(a.control(&json!({"method": "ui.toast", "params": {"text": "x", "kind": "loud"}})).is_err());
}
