//! UI-8: the bounded sidebar (filter, starred, recent), directories, back/forward and palette
//! routing.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;

use super::*;
use crate::View;
use crate::directory::{self, Kind, SideUi, Star};

fn lume() -> trackcraft_core::ProjectKey {
    trackcraft_core::ProjectKey::parse("LUME").unwrap()
}

/// Adds `n` projects (`P01`…) and `n` spaces (`S01`…).
fn grow(h: &mut Harness<'static, TrackApp>, n: usize) {
    for i in 1..=n {
        for (cmd, key, name) in
            [("project.create", format!("P{i:02}"), format!("Project {i:02}")), ("space.create", format!("S{i:02}"), format!("Space {i:02}"))]
        {
            h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": cmd, "params": {"key": key, "name": name}}})).unwrap();
        }
    }
    h.run_steps(2);
}

fn type_into(h: &mut Harness<'static, TrackApp>, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(3);
}

#[test]
fn sidebar_is_bounded_and_the_filter_narrows_every_section() {
    let mut h = harness(app());
    grow(&mut h, 20);
    // 21 projects: at most eight rows, then a link to the directory.
    let total = h.state().session.ws.projects.len();
    let rows = h.query_all_by_label_contains("Expand Project ").count() + h.query_all_by_label_contains("Collapse Project ").count();
    assert!(rows <= 8, "{rows} project rows");
    h.get_by_label(&format!("{} more projects\u{2026}", total - 8));
    assert!(h.query_by_label("All projects\u{2026}").is_some());
    assert!(h.query_by_label("My open issues").is_some());
    type_into(&mut h, "Filter projects and spaces", "project 17");
    assert!(h.query_by_label("Project 17  \u{b7}  P17").is_some());
    assert!(h.query_by_label("Lume  \u{b7}  LUME").is_none());
    assert!(h.query_by_label("My open issues").is_none(), "filters narrow too");
    assert!(h.query_by_label("Space 03").is_none());
    // Page titles match too: the space is listed with the matching page.
    // With 21 spaces, page trees are closed: the title shows only on Home until filtered for.
    let page = h.state().session.ws.pages.values().find(|p| p.parent.is_some()).cloned().unwrap();
    h.state_mut().control(&json!({"method": "ui.sidebar", "params": {"filter": ""}})).unwrap();
    h.run_steps(3);
    let before = h.query_all_by_label(&page.title).count();
    h.state_mut().control(&json!({"method": "ui.sidebar", "params": {"filter": page.title}})).unwrap();
    h.run_steps(3);
    assert!(h.query_all_by_label(&page.title).count() > before);
    let space = h.state().session.ws.spaces.get(&page.space).unwrap().name.clone();
    assert!(h.query_by_label(&space).is_some());
    h.state_mut().control(&json!({"method": "ui.sidebar", "params": {"filter": "zzz nothing"}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label_contains("Nothing matches").is_some());
}

#[test]
fn starring_persists_across_frames() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "projects"}})).unwrap();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Star Lume Mobile App").click();
    h.run_steps(3);
    assert!(h.state().side.starred(&Star::Project("LUME".into())));
    // The sidebar's Starred section lists it.
    assert!(h.query_by_label("Collapse Starred").is_some());
    for _ in 0..3 {
        h.run_steps(5);
    }
    assert_eq!(directory::saved_stars(&h.ctx), vec!["project:LUME".to_string()]);
    // A fresh sidebar state (a restarted app on the same memory) reads it back.
    let mut side = SideUi::default();
    side.sync(&h.ctx);
    assert!(side.starred(&Star::Project("LUME".into())));
    // Opening it from Starred goes to the board.
    h.get_by_label("Collapse Starred");
    h.state_mut().control(&json!({"method": "ui.sidebar", "params": {"star": "project:LUME", "on": false}})).unwrap();
    h.run_steps(3);
    assert!(directory::saved_stars(&h.ctx).is_empty());
    assert!(h.query_by_label("Collapse Starred").is_none());
    assert!(h.state_mut().control(&json!({"method": "ui.sidebar", "params": {"star": "nope"}})).is_err());
}

#[test]
fn back_then_forward_returns() {
    let mut h = harness(app());
    h.run_steps(2);
    h.state_mut().navigate(View::Issues);
    h.state_mut().navigate(View::Issue(key("LUME-2")));
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issues);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issue(key("LUME-2")));
    // The top bar's buttons do the same.
    h.get_by_role_and_label(Role::Button, "Back").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issues);
    h.get_by_role_and_label(Role::Button, "Forward").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issue(key("LUME-2")));
    // Going somewhere new drops the forward history.
    h.state_mut().control(&json!({"method": "ui.back"})).unwrap();
    h.state_mut().navigate(View::Home);
    assert!(!h.state().can_forward());
    assert!(h.state_mut().control(&json!({"method": "ui.forward"})).is_err());
    // Visited issues and boards are recent destinations, shown in the sidebar.
    h.run_steps(3);
    assert!(h.query_by_label("Collapse Recent").is_some());
    assert!(h.state().recent.contains(&View::Issue(key("LUME-2"))));
    assert!(!h.state().recent.contains(&View::Home));
}

#[test]
fn recent_destinations_come_first_in_the_palette() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-4")));
    h.state_mut().navigate(View::Home);
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(3);
    assert!(h.query_all_by_label("RECENT").count() >= 2, "sidebar and palette");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issue(key("LUME-4")));
}

#[test]
fn directory_search_narrows_the_table() {
    let mut h = harness(app());
    grow(&mut h, 12);
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "projects"}})).unwrap();
    h.run_steps(3);
    let total = h.state().session.ws.projects.len();
    assert!(h.query_by_label(&format!("{total} projects")).is_some());
    type_into(&mut h, "Search", "p1");
    assert!(h.query_by_label(&format!("3 of {total} projects")).is_some(), "P10, P11, P12");
    assert_eq!(directory::project_rows(h.state(), "p1").len(), 3);
    // A name opens its board.
    h.get_by_role_and_label(Role::Link, "Project 11").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Board(trackcraft_core::ProjectKey::parse("P11").unwrap()));
    // Spaces: by name, opening the home page.
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "spaces"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().side.query, "", "each directory starts with an empty search");
    assert_eq!(directory::space_rows(h.state(), "space 05").len(), 1);
    h.get_by_role_and_label(Role::Link, "Space 05").click();
    h.run_steps(3);
    let home = h.state().session.ws.space("S05").unwrap().homepage;
    assert_eq!(h.state().view, View::Page(home));
}

fn palette_pick(h: &mut Harness<'static, TrackApp>, query: &str) {
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text(query.into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

#[test]
fn palette_create_project_opens_the_dialog_and_creating_runs_project_create() {
    let mut h = harness(app());
    palette_pick(&mut h, "create project");
    assert!(h.state().palette.is_none());
    assert_eq!(h.state().side.dialog.as_ref().map(|d| d.kind), Some(Kind::Project));
    type_into(&mut h, "Name", "Ops Team");
    assert_eq!(h.state().side.dialog.as_ref().unwrap().key, "OT", "a key is suggested from the name");
    h.state_mut().side.dialog.as_mut().unwrap().key = "TEAM".into();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Create project").click();
    h.run_steps(3);
    assert!(h.state().side.dialog.is_none());
    let ops = trackcraft_core::ProjectKey::parse("TEAM").unwrap();
    assert_eq!(h.state().session.ws.projects.get(&ops).map(|p| p.name.as_str()), Some("Ops Team"));
    assert_eq!(h.state().view, View::Board(ops));
    // A bad key keeps the dialog open with the error.
    palette_pick(&mut h, "create space");
    assert_eq!(h.state().side.dialog.as_ref().map(|d| d.kind), Some(Kind::Space));
    {
        let d = h.state_mut().side.dialog.as_mut().unwrap();
        d.name = "Bad".into();
        d.key = "1BAD".into();
    }
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Create space").click();
    h.run_steps(3);
    assert!(h.state().side.dialog.as_ref().is_some_and(|d| d.error.is_some()));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().side.dialog.is_none());
}

#[test]
fn palette_routes_commands_to_their_forms() {
    let mut h = harness(app());
    // Create Issue on a board: the create dialog, in that project.
    h.state_mut().navigate(View::Board(lume()));
    palette_pick(&mut h, "create issue");
    assert_eq!(h.state().create.as_ref().map(|c| c.project.as_str()), Some("LUME"));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // Create Page on a page: the new-page dialog under it.
    let page = child_page(&h);
    h.state_mut().navigate(View::Page(page));
    palette_pick(&mut h, "create page");
    assert_eq!(h.state().new_page.as_ref().and_then(|f| f.parent), Some(page));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // All Projects opens the directory.
    palette_pick(&mut h, "all projects");
    assert_eq!(h.state().view, View::Projects);
    // A command the context cannot fill stays in the palette with what it needs.
    h.state_mut().navigate(View::Home);
    palette_pick(&mut h, "issue.transition");
    let note = h.state().palette.as_ref().and_then(|p| p.note.clone());
    assert!(note.is_some_and(|n| n.2.contains("\"key\"")));
    assert!(h.query_by_label_contains("Params:").is_some());
}
