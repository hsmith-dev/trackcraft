//! M4.5: the bell's unread count, the Inbox (click opens and marks read, Mark all read), the
//! Watch toggles and the `G then N` jump.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;
use crate::View;
use crate::inbox::bell_label;

fn exec(h: &mut Harness<'static, TrackApp>, command: &str, params: serde_json::Value) {
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": command, "params": params}})).unwrap();
}

fn unread(h: &Harness<'static, TrackApp>) -> usize {
    let s = &h.state().session;
    s.ws.inbox.unread(&s.user)
}

#[test]
fn bell_count_updates_after_another_user_assigns_you() {
    let mut h = harness(app());
    h.run_steps(2);
    let before = unread(&h);
    assert!(h.query_by_label(&bell_label(before)).is_some(), "no bell");
    exec(&mut h, "user.switch", json!({"id": "sam"}));
    exec(&mut h, "issue.assign", json!({"key": "LUME-3", "assignee": "riley"}));
    exec(&mut h, "user.switch", json!({"id": "riley"}));
    h.run_steps(2);
    assert_eq!(unread(&h), before + 1);
    assert!(h.query_by_label(&bell_label(before + 1)).is_some(), "the bell did not update");
    h.get_by_label(&bell_label(before + 1)).click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Inbox);
}

#[test]
fn inbox_click_opens_and_marks_read_then_mark_all() {
    for dark in [false, true] {
        let mut h = harness(app());
        h.state_mut().dark = Some(dark);
        exec(&mut h, "user.switch", json!({"id": "sam"}));
        exec(&mut h, "issue.assign", json!({"key": "LUME-3", "assignee": "riley"}));
        exec(&mut h, "issue.comment", json!({"key": "LUME-3", "body": "@riley over to you"}));
        exec(&mut h, "user.switch", json!({"id": "riley"}));
        h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "inbox"}})).unwrap();
        h.run_steps(3);
        let before = unread(&h);
        assert!(before >= 2);
        h.get_by_label_contains("mentioned you in LUME-3").click();
        h.run_steps(3);
        assert_eq!(h.state().view, View::Issue(key("LUME-3")));
        assert_eq!(unread(&h), before - 1);
        let s = &h.state().session;
        assert!(s.ws.inbox.of("riley").iter().any(|n| n.read && n.excerpt == "@riley over to you"));
        // Back in the inbox: Mark all read empties the count.
        h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "inbox"}})).unwrap();
        h.run_steps(3);
        h.get_by_label("Mark all read").click();
        h.run_steps(3);
        assert_eq!(unread(&h), 0);
        assert!(h.query_all_by_label("Notifications").count() >= 2, "the bell shows no count when all are read");
    }
}

#[test]
fn watch_toggles_on_issue_and_page_headers() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-3")));
    h.run_steps(3);
    let target = trackcraft_model::Target::Issue(key("LUME-3"));
    let watching = |h: &Harness<'static, TrackApp>, t: &trackcraft_model::Target| h.state().session.ws.is_watching("riley", t);
    let was = watching(&h, &target);
    h.get_by_label(if was { "Watching" } else { "Watch" }).click();
    h.run_steps(3);
    assert_eq!(watching(&h, &target), !was);
    let page = child_page(&h);
    h.state_mut().navigate(View::Page(page));
    h.run_steps(3);
    let target = trackcraft_model::Target::Page(page);
    let was = watching(&h, &target);
    h.get_by_label(if was { "Watching" } else { "Watch" }).click();
    h.run_steps(3);
    assert_eq!(watching(&h, &target), !was);
}

#[test]
fn g_then_n_opens_the_inbox_and_empty_states_render() {
    let mut h = harness(app());
    h.run_steps(2);
    h.key_press(egui::Key::G);
    h.run_steps(1);
    h.key_press(egui::Key::N);
    h.run_steps(3);
    assert_eq!(h.state().view, View::Inbox);
    // jo has no notifications in the sample: the empty state, then "Unread only".
    exec(&mut h, "user.switch", json!({"id": "jo"}));
    exec(&mut h, "inbox.markRead", json!({"all": true}));
    h.run_steps(3);
    assert!(h.query_by_label_contains("No notifications yet").is_some() || h.query_by_label("Unread only").is_some());
    h.get_by_label("Unread only").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("No notifications yet").is_some() || h.query_by_label_contains("Nothing unread").is_some());
}
