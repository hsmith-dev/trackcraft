//! The backlog (UI-11): sections, rank by keyboard and drag, move to sprint, inline create.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::SprintState;

use super::*;
use crate::View;

fn lume() -> ProjectKey {
    ProjectKey::parse("LUME").unwrap()
}

fn backlog_harness() -> Harness<'static, TrackApp> {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "backlog", "project": "lume"}})).unwrap();
    h.run_steps(3);
    h
}

fn sprint(h: &Harness<'static, TrackApp>, state: SprintState) -> Id {
    h.state().session.ws.sprints.values().find(|s| s.project == lume() && s.state == state).unwrap().id
}

/// Each section's keys, in display order.
fn order(h: &mut Harness<'static, TrackApp>) -> Vec<Vec<String>> {
    crate::backlog::sections(h.state_mut(), &lume()).iter().map(|s| s.keys.iter().map(ToString::to_string).collect()).collect()
}

#[test]
fn backlog_renders_sections_in_order() {
    let mut h = backlog_harness();
    assert_eq!(h.state().view, View::Backlog(lume()));
    let y = |h: &Harness<'static, TrackApp>, l: &str| h.get_by_label(l).rect().top();
    let (a, b, c) = (y(&h, "Collapse LUME Sprint 1"), y(&h, "Collapse LUME Sprint 2"), y(&h, "Collapse Backlog"));
    assert!(a < b && b < c, "{a} {b} {c}");
    assert_eq!(order(&mut h), vec![vec!["LUME-4", "LUME-2", "LUME-3"], vec![], vec!["LUME-6"]]);
    // Empty future sprint shows its hint; the active sprint offers Complete, not Start.
    assert!(h.query_by_label_contains("Drag issues here").is_some());
    assert!(h.query_by_label("Complete sprint").is_some());
    for theme in [true, false] {
        h.state_mut().dark = Some(theme);
        h.run_steps(2);
    }
}

#[test]
fn backlog_keyboard_rank_moves_one_step() {
    let mut h = backlog_harness();
    h.key_press(egui::Key::J);
    h.run_steps(2);
    assert_eq!(h.state().backlog.sel, Some(key("LUME-4")));
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    h.run_steps(3);
    assert_eq!(order(&mut h)[0], vec!["LUME-2", "LUME-4", "LUME-3"]);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::End);
    h.run_steps(3);
    assert_eq!(order(&mut h)[0], vec!["LUME-2", "LUME-3", "LUME-4"]);
    // One undo step per rank.
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": "edit.undo"}})).unwrap();
    h.run_steps(2);
    assert_eq!(order(&mut h)[0], vec!["LUME-2", "LUME-4", "LUME-3"]);
    // Enter opens the selected issue.
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().view, View::Issue(key("LUME-4")));
}

/// Presses at `from`, moves to `to` in a few frames, releases.
fn drag(h: &mut Harness<'static, TrackApp>, from: egui::Pos2, to: egui::Pos2) {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(button(from, true));
    h.run_steps(1);
    for n in 1..=6 {
        let p = from + (to - from) * (n as f32 / 6.0);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
    }
    h.event(button(to, false));
    h.run_steps(3);
}

#[test]
fn backlog_drag_ranks_and_moves_between_sections() {
    let mut h = backlog_harness();
    // LUME-3 by its handle onto the top half of LUME-4: first in the sprint.
    let from = h.get_by_label_contains("LUME-3 ").rect();
    let onto = h.get_by_label_contains("LUME-4 ").rect();
    drag(&mut h, egui::pos2(from.left() + 12.0, from.center().y), egui::pos2(onto.left() + 40.0, onto.top() + 6.0));
    assert_eq!(order(&mut h)[0], vec!["LUME-3", "LUME-4", "LUME-2"]);
    // LUME-6 from the backlog into the empty future sprint (its empty-state area).
    let from = h.get_by_label_contains("LUME-6 ").rect();
    let zone = h.get_by_label_contains("Drag issues here").rect();
    drag(&mut h, egui::pos2(from.left() + 12.0, from.center().y), zone.center());
    let future = sprint(&h, SprintState::Future);
    assert_eq!(h.state().session.ws.issues[&key("LUME-6")].sprint, Some(future));
    assert_eq!(order(&mut h)[1], vec!["LUME-6"]);
}

#[test]
fn backlog_move_to_sprint_picker() {
    let mut h = backlog_harness();
    // k from nothing selects the last row (LUME-6, in the backlog).
    h.key_press(egui::Key::K);
    h.run_steps(2);
    assert_eq!(h.state().backlog.sel, Some(key("LUME-6")));
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::S);
    h.run_steps(3);
    assert_eq!(h.state().backlog.move_pick, Some(key("LUME-6")));
    h.get_by_label("LUME Sprint 2 (planned)").click();
    h.run_steps(3);
    assert!(h.state().backlog.move_pick.is_none());
    assert_eq!(h.state().session.ws.issues[&key("LUME-6")].sprint, Some(sprint(&h, SprintState::Future)));
    // Into the active sprint: it lands at the bottom of that section.
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::S);
    h.run_steps(3);
    h.get_by_label("LUME Sprint 1 (active)").click();
    h.run_steps(3);
    assert_eq!(order(&mut h)[0], vec!["LUME-4", "LUME-2", "LUME-3", "LUME-6"]);
}

#[test]
fn backlog_inline_create_lands_in_section() {
    let mut h = backlog_harness();
    // The second section's "+ Create issue" is the future sprint's.
    let rows: Vec<_> = h.get_all_by_label("+ Create issue").collect();
    assert_eq!(rows.len(), 3);
    rows[1].click();
    h.run_steps(3);
    h.event(egui::Event::Text("Trim titles in the share sheet".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let future = sprint(&h, SprintState::Future);
    let made = h.state().session.ws.issues.values().find(|i| i.summary == "Trim titles in the share sheet").unwrap().key.clone();
    assert_eq!(h.state().session.ws.issues[&made].sprint, Some(future));
    assert_eq!(order(&mut h)[1], vec![made.to_string()]);
    assert_eq!(h.state().backlog.sel, Some(made));
    // The row stays open for the next one; Escape closes it.
    assert!(h.state().backlog.inline.is_some());
}

#[test]
fn backlog_shortcuts_need_no_text_focus() {
    let mut h = backlog_harness();
    h.get_all_by_label("+ Create issue").next().unwrap().click();
    h.run_steps(3);
    // Typing j into the summary does not move the selection.
    h.key_press(egui::Key::J);
    h.event(egui::Event::Text("j".into()));
    h.run_steps(2);
    assert!(h.state().backlog.sel.is_none());
    assert_eq!(h.state().backlog.inline.as_ref().unwrap().summary, "j");
}
