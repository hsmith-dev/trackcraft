//! M4.7: the status bar's connection indicator, "Reconnect now", and remote-mode undo/save.

use egui_kittest::kittest::{NodeT as _, Queryable as _};

use super::*;
use crate::{ConnectionKind, ConnectionState, FileRequest, REMOTE_SAVED, REMOTE_UNDO};

fn state(kind: ConnectionKind, label: &str, detail: &str) -> Option<ConnectionState> {
    Some(ConnectionState::new(kind, label, detail))
}

fn remote_app() -> Harness<'static, TrackApp> {
    let mut a = app();
    a.set_remote(true);
    let mut h = harness(a);
    h.run_steps(2);
    h
}

#[test]
fn every_state_renders_in_both_themes() {
    let mut h = remote_app();
    let cases = [
        (ConnectionKind::Online, "Online"),
        (ConnectionKind::Syncing, "Syncing · 2 unsent"),
        (ConnectionKind::Offline, "Offline · 3 unsent"),
        (ConnectionKind::Error, "Token refused"),
    ];
    for dark in [false, true] {
        h.state_mut().dark = Some(dark);
        for (kind, label) in cases {
            h.state_mut().set_connection(state(kind, label, "Server: https://track.example.org:443/w/team"));
            h.run_steps(2);
            h.get_by_label(&format!("Connection: {label}"));
            // A server workspace has no file: no save state beside the indicator.
            assert!(h.query_by_label("Not saved yet").is_none());
        }
    }
    // Hidden again: the local save state comes back once remote mode ends.
    h.state_mut().set_connection(None);
    h.state_mut().set_remote(false);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Connection:").is_none());
    h.get_by_label("Not saved yet");
    // An empty label falls back to the kind's word; hostile labels are cut short.
    h.state_mut().set_connection(state(ConnectionKind::Error, " ", ""));
    h.run_steps(2);
    h.get_by_label("Connection: Connection error");
    h.state_mut().set_connection(state(ConnectionKind::Offline, &format!("{}\nsecond line", "x".repeat(5000)), &"d".repeat(100_000)));
    h.run_steps(2);
    let n = h.get_by_label_contains("Connection: xxx");
    assert!(n.accesskit_node().label().unwrap_or_default().chars().count() < 60);
}

#[test]
fn hover_shows_the_details() {
    let mut h = remote_app();
    h.state_mut().set_connection(state(
        ConnectionKind::Offline,
        "Offline · 2 unsent",
        "2 changes waiting to be sent\nLast error: connection refused",
    ));
    h.run_steps(2);
    h.get_by_label("Connection: Offline · 2 unsent").hover();
    h.run_steps(4);
    h.get_by_label("2 changes waiting to be sent");
    h.get_by_label("Last error: connection refused");
    h.get_by_label("Click to reconnect now");
}

#[test]
fn reconnect_now_is_offered_when_offline_or_in_error() {
    let mut h = remote_app();
    for kind in [ConnectionKind::Offline, ConnectionKind::Error] {
        h.state_mut().set_connection(state(kind, "Down", ""));
        h.run_steps(2);
        assert!(!h.state_mut().take_reconnect_request());
        h.get_by_label("Connection: Down").click();
        h.run_steps(3);
        h.get_by_label("Reconnect now").click();
        h.run_steps(3);
        assert!(h.state_mut().take_reconnect_request(), "{kind:?}");
        assert!(!h.state_mut().take_reconnect_request(), "taken once");
        assert!(h.query_by_label("Reconnect now").is_none(), "the menu closes");
    }
    // Online: a click offers nothing.
    h.state_mut().set_connection(state(ConnectionKind::Online, "Up", ""));
    h.run_steps(2);
    h.get_by_label("Connection: Up").click();
    h.run_steps(3);
    assert!(h.query_by_label("Reconnect now").is_none());
    assert!(!h.state_mut().take_reconnect_request());
}

#[test]
fn undo_is_disabled_on_a_server_workspace() {
    let mut a = app();
    // A change with an undo step, so only remote mode stands in the way.
    a.session.run("issue.assignToMe", &json!({"key": "LUME-3"})).unwrap();
    a.notify_done("Assigned LUME-3 to you", true);
    assert!(a.session.can_undo());
    let mut h = harness(a);
    h.run_steps(2);
    assert!(!h.get_by_label("Undo").accesskit_node().is_disabled(), "local: enabled");
    h.get_by_label("Undo this change");
    h.state_mut().set_remote(true);
    h.run_steps(2);
    assert!(h.get_by_label("Undo").accesskit_node().is_disabled());
    assert!(h.get_by_label("Redo").accesskit_node().is_disabled());
    assert!(h.query_by_label("Undo this change").is_none(), "no Undo on the toast");
    h.get_by_label("Undo").hover();
    h.run_steps(4);
    h.get_by_label(REMOTE_UNDO);
    // The shortcuts say why instead of undoing.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("riley"), "nothing was undone");
    assert!(h.state().session.can_undo());
    assert!(h.state().toasts.items.iter().filter(|t| t.text == REMOTE_UNDO).count() >= 2);
}

#[test]
fn a_remote_mode_session_counts_as_remote() {
    let mut s = Session::new(trackcraft_engine::sample::workspace(fixed_now()), "riley").with_clock(fixed_now);
    s.set_remote(true);
    let a = TrackApp::new(s);
    assert!(a.is_remote());
    let mut h = harness(a);
    h.run_steps(2);
    assert!(h.get_by_label("Undo").accesskit_node().is_disabled());
}

#[test]
fn save_on_a_server_workspace_needs_no_dialog() {
    let mut h = remote_app();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(3);
    assert_eq!(h.state_mut().take_file_request(), None);
    assert!(h.state().toasts.items.iter().any(|t| t.text == REMOTE_SAVED));
    // File › Save from the menu too.
    h.get_by_label("File").click();
    h.run_steps(3);
    // The item's accessible name carries its shortcut text.
    let save = h.get_all_by_label_contains("Save").find(|n| !n.accesskit_node().label().unwrap_or_default().contains("As")).unwrap();
    save.click();
    h.run_steps(3);
    assert_eq!(h.state_mut().take_file_request(), None);
    // A local workspace still asks the host for a dialog.
    h.state_mut().set_remote(false);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(3);
    assert_eq!(h.state_mut().take_file_request(), Some(FileRequest::Save));
}

#[test]
fn connection_control_method() {
    let mut a = app();
    let ok =
        a.control(&json!({"method": "ui.connection", "params": {"kind": "offline", "label": "Offline", "detail": "d", "remote": true}})).unwrap();
    assert_eq!((ok["remote"].as_bool(), ok["reconnect"].as_bool()), (Some(true), Some(false)));
    assert_eq!(a.connection().map(|c| (c.kind, c.label.as_str(), c.detail.as_str())), Some((ConnectionKind::Offline, "Offline", "d")));
    a.control(&json!({"method": "ui.connection", "params": {"kind": "online"}})).unwrap();
    assert_eq!(a.connection().map(|c| c.label.as_str()), Some("Online"));
    // Without "kind" the state stays; null hides it.
    a.control(&json!({"method": "ui.connection", "params": {"remote": false}})).unwrap();
    assert!(a.connection().is_some() && !a.is_remote());
    a.control(&json!({"method": "ui.connection", "params": {"kind": null}})).unwrap();
    assert!(a.connection().is_none());
    for bad in [json!({"kind": "sideways"}), json!({"kind": 3}), json!({"kind": ["online"]})] {
        assert!(a.control(&json!({"method": "ui.connection", "params": bad})).is_err(), "{bad}");
    }
    // Junk params never panic.
    for junk in [json!(null), json!([]), json!("x"), json!({"label": 5, "detail": {}})] {
        let _ = a.control(&json!({"method": "ui.connection", "params": junk}));
    }
}
