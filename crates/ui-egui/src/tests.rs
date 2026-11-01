//! Interface tests: every screen renders in both themes, and UI actions go through commands.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_engine::Session;

use crate::{TrackApp, View};

fn fixed_now() -> trackcraft_model::Timestamp {
    trackcraft_model::Timestamp::from_timestamp(1_793_448_000, 0).unwrap_or_default()
}

fn app() -> TrackApp {
    let ws = trackcraft_engine::sample::workspace(fixed_now());
    TrackApp::new(Session::new(ws, "riley").with_clock(fixed_now))
}

fn harness(app: TrackApp) -> Harness<'static, TrackApp> {
    Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut TrackApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

#[test]
fn every_view_renders_in_both_themes() {
    for dark in [false, true] {
        let mut h = harness(app());
        h.state_mut().dark = Some(dark);
        let home = h.state().session.ws.spaces.values().next().map(|s| s.homepage).unwrap();
        let page = h.state().session.ws.pages.values().find(|p| p.parent.is_some()).map(|p| p.id).unwrap();
        let views = [
            View::Home,
            View::Issues,
            View::Board(trackcraft_core::ProjectKey::parse("LUME").unwrap()),
            View::Issue(trackcraft_core::IssueKey::parse("LUME-2").unwrap()),
            View::Issue(trackcraft_core::IssueKey::parse("LUME-4").unwrap()),
            View::Page(home),
            View::Page(page),
            View::PageHistory(page),
        ];
        for v in views {
            h.state_mut().navigate(v.clone());
            h.run_steps(3);
        }
        for m in ["ui.create", "ui.palette"] {
            h.state_mut().control(&json!({"method": m})).unwrap();
            h.run_steps(3);
        }
    }
}

#[test]
fn control_runs_commands_and_navigates() {
    let mut h = harness(app());
    let r = h
        .state_mut()
        .control(
            &json!({"method": "engine.execute", "params": {"command": "issue.create", "params": {"project": "LUME", "summary": "From the UI test"}}}),
        )
        .unwrap();
    assert_eq!(r["key"], "LUME-8");
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "issue", "key": "LUME-8"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issue(trackcraft_core::IssueKey::parse("LUME-8").unwrap()));
    // Undo removes the issue; the view falls back to Home instead of showing a ghost.
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": "edit.undo"}})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().view, View::Home);
    assert!(h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "page", "id": 999_999}})).is_err());
    assert!(h.state_mut().control(&json!({"method": "nope"})).is_err());
}

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

fn key(k: &str) -> trackcraft_core::IssueKey {
    trackcraft_core::IssueKey::parse(k).unwrap()
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

#[test]
fn board_keys_select_and_shift_right_transitions() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Board(trackcraft_core::ProjectKey::parse("LUME").unwrap()));
    h.run_steps(3);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    let sel = h.state().board_sel.clone().unwrap();
    let before = h.state().session.ws.issues[&sel].status.clone();
    let cols = crate::issues::board_columns(h.state(), &sel.project);
    let at = cols.iter().position(|c| c.0 == before).unwrap();
    let next = cols[at + 1].0.clone();
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&sel].status, next);
    // `[` moves it back; the selection follows the card.
    h.key_press(egui::Key::OpenBracket);
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&sel].status, before);
    assert_eq!(h.state().board_sel, Some(sel.clone()));
    // j moves within the column, Enter opens.
    h.key_press(egui::Key::J);
    h.run_steps(2);
    let below = h.state().board_sel.clone().unwrap();
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().view, View::Issue(below));
}

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
fn issue_keys_assign_comment_and_edit() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-3")));
    h.run_steps(3);
    assert_ne!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("riley"));
    h.key_press(egui::Key::A);
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
