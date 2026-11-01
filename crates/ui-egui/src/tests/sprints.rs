//! Sprints (UI-12): start / complete / create / edit / delete dialogs and the sprint board.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::SprintState;

use super::*;
use crate::View;

fn lume() -> ProjectKey {
    ProjectKey::parse("LUME").unwrap()
}

fn sprint(h: &Harness<'static, TrackApp>, state: SprintState) -> Id {
    h.state().session.ws.sprints.values().find(|s| s.project == lume() && s.state == state).unwrap().id
}

fn on_backlog() -> Harness<'static, TrackApp> {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "backlog", "project": "LUME"}})).unwrap();
    h.run_steps(3);
    h
}

fn dialog(h: &mut Harness<'static, TrackApp>, kind: &str) {
    h.state_mut().control(&json!({"method": "ui.sprintDialog", "params": {"kind": kind, "project": "LUME"}})).unwrap();
    h.run_steps(3);
}

#[test]
fn sprint_start_refuses_bad_dates_inline() {
    let mut h = on_backlog();
    let future = sprint(&h, SprintState::Future);
    dialog(&mut h, "start");
    h.state_mut().sprint_dialog.as_mut().unwrap().start = "2026-13-40".into();
    h.run_steps(1);
    h.get_by_label("Start").click();
    h.run_steps(3);
    let err = h.state().sprint_dialog.as_ref().unwrap().error.clone().unwrap();
    assert!(err.contains("YYYY-MM-DD"), "{err}");
    assert!(h.query_by_label(&err).is_some(), "the message is shown in the dialog");
    // End before start.
    let d = h.state_mut().sprint_dialog.as_mut().unwrap();
    d.start = "2026-11-10".into();
    d.end = "2026-11-03".into();
    h.run_steps(1);
    h.get_by_label("Start").click();
    h.run_steps(3);
    assert!(h.state().sprint_dialog.as_ref().unwrap().error.as_deref().unwrap().contains("after the start"));
    // Good dates, but the engine refuses (one active sprint; this one is empty): inline too.
    h.state_mut().sprint_dialog.as_mut().unwrap().end = "2026-11-24".into();
    h.run_steps(1);
    h.get_by_label("Start").click();
    h.run_steps(3);
    assert!(h.state().sprint_dialog.as_ref().unwrap().error.is_some());
    assert_eq!(h.state().session.ws.sprints[&future].state, SprintState::Future);
    // Nothing half-happened (no rename left behind, nothing to undo).
    assert!(!h.state().session.can_undo());
}

#[test]
fn sprint_complete_moves_open_issues_then_start_next() {
    let mut h = on_backlog();
    let (active, future) = (sprint(&h, SprintState::Active), sprint(&h, SprintState::Future));
    dialog(&mut h, "complete");
    assert!(h.query_by_label("0 issues done, 3 open.").is_some());
    // The dialog's button is the last of the two "Complete sprint" buttons.
    h.get_all_by_label("Complete sprint").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().sprint_dialog.is_none());
    let ws = &h.state().session.ws;
    assert_eq!(ws.sprints[&active].state, SprintState::Closed);
    for k in ["LUME-2", "LUME-3", "LUME-4"] {
        assert_eq!(ws.issues[&key(k)].sprint, Some(future), "{k}");
    }
    // Now the planned sprint can start with the dialog's preset dates.
    h.get_by_label("Start sprint").click();
    h.run_steps(3);
    let d = h.state().sprint_dialog.as_ref().unwrap();
    assert_eq!(d.weeks, Some(2));
    assert_eq!((d.start.as_str(), d.end.as_str()), ("2026-10-31", "2026-11-14"));
    h.get_by_label("Start").click();
    h.run_steps(3);
    assert!(h.state().sprint_dialog.is_none());
    assert_eq!(h.state().session.ws.sprints[&future].state, SprintState::Active);
}

#[test]
fn sprint_create_edit_and_delete() {
    let mut h = on_backlog();
    h.get_by_label("+ Create sprint").click();
    h.run_steps(3);
    let d = h.state_mut().sprint_dialog.as_mut().unwrap();
    d.name = "Polish week".into();
    d.start = "2026-11-17".into();
    d.end = "2026-11-21".into();
    h.run_steps(1);
    h.get_by_label("Create").click();
    h.run_steps(3);
    let id = h.state().session.ws.sprints.values().find(|s| s.name == "Polish week").unwrap().id;
    h.state_mut().control(&json!({"method": "ui.sprintDialog", "params": {"kind": "edit", "id": id.0}})).unwrap();
    h.run_steps(3);
    h.state_mut().sprint_dialog.as_mut().unwrap().goal = "Fix the rough edges".into();
    h.run_steps(1);
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.sprints[&id].goal, "Fix the rough edges");
    // Edit → Delete sprint asks first.
    h.state_mut().control(&json!({"method": "ui.sprintDialog", "params": {"kind": "edit", "id": id.0}})).unwrap();
    h.run_steps(3);
    h.get_by_label("Delete sprint").click();
    h.run_steps(3);
    assert!(h.state().session.ws.sprints.contains_key(&id));
    h.get_all_by_label("Delete sprint").last().unwrap().click();
    h.run_steps(3);
    assert!(!h.state().session.ws.sprints.contains_key(&id));
}

#[test]
fn sprint_board_shows_only_sprint_issues() {
    let mut h = harness(app());
    // The sidebar's Board opens the active sprint's board.
    h.run_steps(2);
    h.get_all_by_label("Board").next().unwrap().click();
    h.run_steps(3);
    let active = sprint(&h, SprintState::Active);
    assert_eq!(h.state().view, View::SprintBoard(lume(), active));
    let shown: Vec<String> = crate::issues::board_columns(h.state(), &lume()).into_iter().flat_map(|c| c.2).map(|k| k.to_string()).collect();
    let mut sorted = shown.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["LUME-2", "LUME-3", "LUME-4", "LUME-7"]);
    assert!(h.query_by_label("9 days left").is_some());
    assert!(h.query_by_label("Recent notes open without a connection").is_some());
    // "Whole project" goes back to every issue.
    h.get_by_label("Whole project").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Board(lume()));
    assert!(crate::issues::board_columns(h.state(), &lume()).iter().any(|c| c.2.contains(&key("LUME-6"))));
    // ui.navigate takes "sprint"; a bad one fails, a project without an active sprint falls back.
    let nav = |h: &mut Harness<'static, TrackApp>, p| h.state_mut().control(&json!({"method": "ui.navigate", "params": p}));
    nav(&mut h, json!({"view": "board", "project": "LUME", "sprint": active.0})).unwrap();
    assert_eq!(h.state().view, View::SprintBoard(lume(), active));
    assert!(nav(&mut h, json!({"view": "board", "project": "LUME", "sprint": 999_999})).is_err());
    assert!(nav(&mut h, json!({"view": "board", "project": "LUME", "sprint": "soon"})).is_err());
    nav(&mut h, json!({"view": "board", "project": "OPS", "sprint": "active"})).unwrap();
    assert_eq!(h.state().view, View::Board(ProjectKey::parse("OPS").unwrap()));
    h.run_steps(2);
}

#[test]
fn sprint_dialog_control_rejects_junk() {
    let mut h = harness(app());
    for p in
        [json!({"kind": "nope"}), json!({"kind": "start"}), json!({"kind": "complete", "id": 999_999}), json!({"kind": "create", "project": "ZZZ"})]
    {
        assert!(h.state_mut().control(&json!({"method": "ui.sprintDialog", "params": p})).is_err());
    }
    assert!(h.state().sprint_dialog.is_none());
}
