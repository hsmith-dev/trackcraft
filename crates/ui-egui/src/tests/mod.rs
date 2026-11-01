//! Interface tests (egui_kittest): every screen renders in both themes, and UI actions go
//! through commands. Shared helpers live here; tests go in the file for their area.

mod backlog; // UI-11
mod board;
mod guard;
mod hostile; // UI-6
mod issue;
mod issue_polish; // UI-9
mod keys;
mod list;
mod nav;
mod nav_history; // UI-8
mod sprints; // UI-12
mod toast; // UI-5
mod wiki;

use egui_kittest::Harness;
use serde_json::json;
use trackcraft_engine::Session;

use crate::TrackApp;

pub(super) fn fixed_now() -> trackcraft_model::Timestamp {
    trackcraft_model::Timestamp::from_timestamp(1_793_448_000, 0).unwrap_or_default()
}

pub(super) fn app() -> TrackApp {
    let ws = trackcraft_engine::sample::workspace(fixed_now());
    TrackApp::new(Session::new(ws, "riley").with_clock(fixed_now))
}

pub(super) fn harness(app: TrackApp) -> Harness<'static, TrackApp> {
    Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut TrackApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

pub(super) fn key(k: &str) -> trackcraft_core::IssueKey {
    trackcraft_core::IssueKey::parse(k).unwrap()
}

pub(super) fn child_page(h: &Harness<'static, TrackApp>) -> trackcraft_core::Id {
    h.state().session.ws.pages.values().find(|p| p.parent.is_some()).map(|p| p.id).unwrap()
}

/// Opens a child page in the editor and types into its body.
pub(super) fn dirty_page_editor(h: &mut Harness<'static, TrackApp>) -> trackcraft_core::Id {
    let page = child_page(h);
    h.state_mut().control(&json!({"method": "ui.editPage", "params": {"id": page.0}})).unwrap();
    h.run_steps(3);
    h.state_mut().page_edit.as_mut().unwrap().body.push_str("\nTyped but not published.");
    h.run_steps(2);
    page
}
