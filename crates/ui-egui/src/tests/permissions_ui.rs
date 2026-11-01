//! Permissions UI (UI-37): workspace roles, project and space access, page restrictions, and
//! controls disabled for people who may not use them.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::{AccessLevel, WorkspaceRole};

use super::*;
use crate::View;

fn nav(h: &mut Harness<'static, TrackApp>, p: serde_json::Value) {
    h.state_mut().control(&json!({"method": "ui.navigate", "params": p})).unwrap();
    h.run_steps(3);
}

fn exec(h: &mut Harness<'static, TrackApp>, command: &str, params: serde_json::Value) {
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": command, "params": params}})).unwrap();
    h.run_steps(2);
}

fn lume() -> ProjectKey {
    ProjectKey::parse("LUME").unwrap()
}

#[test]
fn workspace_role_changes_through_the_table() {
    let mut h = harness(app());
    nav(&mut h, json!({"view": "settings", "tab": "permissions"}));
    h.get_by_role_and_label(Role::ComboBox, "Sam Okafor").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Viewer").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.workspace_role("sam"), Some(WorkspaceRole::Viewer));
    assert!(h.state().settings.role_error.is_none());
}

#[test]
fn demoting_the_last_admin_is_refused_inline() {
    let mut h = harness(app());
    nav(&mut h, json!({"view": "settings", "tab": "permissions"}));
    h.get_by_role_and_label(Role::ComboBox, "Riley Park (you)").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Member").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.workspace_role("riley"), Some(WorkspaceRole::Admin));
    let err = h.state().settings.role_error.clone().expect("an inline error");
    assert!(h.query_by_label(&err).is_some(), "the refusal is shown under the table: {err}");
}

#[test]
fn private_project_leaves_a_members_sidebar() {
    let mut h = harness(app());
    let row = "Lume Mobile App  \u{b7}  LUME";
    h.run_steps(3);
    assert!(h.query_by_label(row).is_some());
    // The access level is set through the project's settings.
    nav(&mut h, json!({"view": "projectSettings", "project": "LUME"}));
    h.get_by_label("Private").click();
    h.run_steps(3);
    let level = h.state().session.ws.access.projects.get(&lume()).map(|g| g.access);
    assert_eq!(level, Some(AccessLevel::Private));
    // Sam is a member without a LUME role: with enforcement on, the project is gone.
    h.state_mut().session.set_enforce(true);
    exec(&mut h, "user.switch", json!({"id": "sam"}));
    nav(&mut h, json!({"view": "home"}));
    assert!(h.query_by_label(row).is_none(), "a private project is hidden from members without a role");
    assert!(h.query_by_label("Office Operations  \u{b7}  OPS").is_some());
}

#[test]
fn project_roles_add_change_and_remove() {
    let mut h = harness(app());
    nav(&mut h, json!({"view": "projectSettings", "project": "LUME"}));
    assert_eq!(h.state().view, View::ProjectSettings(lume(), crate::project_settings::ProjectTab::Access));
    h.get_by_role_and_label(Role::ComboBox, "Add person").click();
    h.run_steps(2);
    h.get_by_label("Jo Lindqvist").click();
    h.run_steps(2);
    h.get_by_label("Add").click();
    h.run_steps(3);
    let role = |h: &Harness<'static, TrackApp>| h.state().session.ws.access.projects.get(&lume()).and_then(|g| g.roles.get("jo").copied());
    assert_eq!(role(&h), Some(trackcraft_model::ProjectRole::Member));
    h.get_by_label("Remove Jo Lindqvist").click();
    h.run_steps(3);
    assert_eq!(role(&h), None);
    // Space settings render too.
    nav(&mut h, json!({"view": "spaceSettings", "space": "ENG"}));
    assert!(h.query_by_label("Engineering Handbook settings").is_some());
}

#[test]
fn restrictions_dialog_restricts_and_shows_the_lock() {
    let mut h = harness(app());
    nav(&mut h, json!({"view": "page", "id": 24}));
    assert!(h.query_by_label("Restricted page").is_none());
    h.get_by_label("More page actions").click();
    h.run_steps(2);
    h.get_by_label("Restrictions\u{2026}").click();
    h.run_steps(3);
    h.get_by_label("Only these people can view").click();
    h.run_steps(2);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let r = h.state().session.ws.access.pages.get(&Id(24)).cloned().unwrap();
    assert_eq!(r.view, Some(vec!["riley".to_string()]), "the restricting person stays on the list");
    assert_eq!(r.edit, None);
    assert!(crate::wiki::restricted(&h.state().session.ws, Id(24)));
    assert!(h.query_by_label("Restricted page").is_some(), "the page header shows a lock");
    // A child page notes the inherited limit.
    nav(&mut h, json!({"view": "page", "id": 25}));
    h.state_mut().control(&json!({"method": "ui.pageDialog", "params": {"kind": "restrict", "id": 25}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label_contains("a parent page, limits who can view it").is_some());
}

#[test]
fn a_viewer_sees_disabled_controls() {
    let mut h = harness(app());
    exec(&mut h, "access.setWorkspaceRole", json!({"user": "jo", "role": "viewer"}));
    // Local files don't enforce: everything stays enabled.
    exec(&mut h, "user.switch", json!({"id": "jo"}));
    nav(&mut h, json!({"view": "page", "id": 27}));
    assert!(!h.get_by_role_and_label(Role::Button, "Edit").accesskit_node().is_disabled());
    h.state_mut().session.set_enforce(true);
    h.run_steps(2);
    assert!(h.get_by_role_and_label(Role::Button, "Edit").accesskit_node().is_disabled());
    assert!(h.get_by_label("Delete").accesskit_node().is_disabled());
    assert!(h.get_by_label("+ Child page").accesskit_node().is_disabled());
    nav(&mut h, json!({"view": "issue", "key": "LUME-2"}));
    assert!(h.get_by_role_and_label(Role::ComboBox, "Status").accesskit_node().is_disabled());
    assert!(h.get_by_role_and_label(Role::ComboBox, "Priority").accesskit_node().is_disabled());
    assert!(h.get_by_label("Delete issue\u{2026}").accesskit_node().is_disabled());
    nav(&mut h, json!({"view": "settings", "tab": "permissions"}));
    assert!(h.get_by_role_and_label(Role::ComboBox, "Sam Okafor").accesskit_node().is_disabled());
    nav(&mut h, json!({"view": "projectSettings", "project": "LUME"}));
    assert!(h.get_by_label("Private").accesskit_node().is_disabled());
    // Both themes render.
    h.state_mut().dark = Some(true);
    h.run_steps(2);
}
