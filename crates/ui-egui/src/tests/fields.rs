//! M2.4: custom fields, components and versions in the issue view, the create dialog, the
//! fields editor and the releases view.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use serde_json::{Value, json};
use trackcraft_core::ProjectKey;
use trackcraft_model::FieldValue;

use super::*;
use crate::View;

fn exec(h: &mut Harness<'static, TrackApp>, command: &str, params: Value) -> Value {
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": command, "params": params}})).unwrap()
}

fn lume() -> ProjectKey {
    ProjectKey::parse("LUME").unwrap()
}

fn with_fields() -> Harness<'static, TrackApp> {
    let mut h = harness(app());
    exec(&mut h, "field.create", json!({"project": "LUME", "name": "Severity", "type": "select", "options": ["Minor", "Major", "Critical"]}));
    exec(&mut h, "field.create", json!({"project": "LUME", "name": "Due on", "type": "date"}));
    exec(&mut h, "component.create", json!({"project": "LUME", "name": "Sync"}));
    exec(&mut h, "version.create", json!({"project": "LUME", "name": "1.2"}));
    h
}

fn field_id(h: &Harness<'static, TrackApp>, name: &str) -> trackcraft_core::Id {
    h.state().session.ws.project_fields(&lume()).unwrap().fields.iter().find(|f| f.name == name).unwrap().id
}

#[test]
fn edit_a_select_field_on_the_issue_view() {
    let mut h = with_fields();
    h.state_mut().navigate(View::Issue(key("LUME-2")));
    h.run_steps(3);
    h.get_by_role_and_label(Role::ComboBox, "Severity").click();
    h.run_steps(3);
    h.get_by_label("Major").click();
    h.run_steps(3);
    let sev = field_id(&h, "Severity");
    assert_eq!(h.state().session.ws.issues[&key("LUME-2")].custom.get(&sev), Some(&FieldValue::Select("Major".into())));
    // One history entry named after the field.
    let last = h.state().session.ws.issues[&key("LUME-2")].history.last().cloned().unwrap();
    assert_eq!((last.field.as_str(), last.to.as_deref()), ("Severity", Some("Major")));
    // Both themes render the section.
    h.state_mut().dark = Some(true);
    h.run_steps(2);
    h.get_by_label("More fields");
}

#[test]
fn add_a_component_and_a_fix_version_on_the_issue_view() {
    let mut h = with_fields();
    h.state_mut().navigate(View::Issue(key("LUME-3")));
    h.run_steps(3);
    h.get_by_role_and_label(Role::ComboBox, "Components").click();
    h.run_steps(3);
    h.get_by_label("Sync").click();
    h.run_steps(3);
    let i = &h.state().session.ws.issues[&key("LUME-3")];
    assert_eq!(i.components.len(), 1);
    h.get_by_role_and_label(Role::ComboBox, "Fix versions").click();
    h.run_steps(3);
    h.get_by_label("1.2").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].fix_versions.len(), 1);
    // The chip's cross removes it again.
    h.get_by_label("Remove component Sync").click();
    h.run_steps(3);
    assert!(h.state().session.ws.issues[&key("LUME-3")].components.is_empty());
}

#[test]
fn add_a_component_in_the_fields_editor() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "fields", "project": "LUME"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().view, View::ProjectFields(lume()));
    h.get_by_role_and_label(Role::TextInput, "New component").click();
    h.run_steps(2);
    h.event(egui::Event::Text("Editor".into()));
    h.run_steps(2);
    h.get_by_label("Add component").click();
    h.run_steps(3);
    let comps = &h.state().session.ws.project_fields(&lume()).unwrap().components;
    assert_eq!(comps.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Editor"]);
    // A text field, then delete it and the component (the grid settles over a few frames).
    h.get_by_role_and_label(Role::TextInput, "Name").click();
    h.run_steps(2);
    h.event(egui::Event::Text("Customer".into()));
    h.run_steps(2);
    h.get_by_label("Add field").click();
    h.run_steps(10);
    assert_eq!(h.state().session.ws.project_fields(&lume()).unwrap().fields.len(), 1);
    h.get_by_label("Delete field Customer").click();
    h.run_steps(10);
    h.get_by_label("Delete component Editor").click();
    h.run_steps(3);
    assert!(h.state().session.ws.project_fields(&lume()).is_none(), "nothing left defined");
}

#[test]
fn release_a_version() {
    let mut h = with_fields();
    exec(&mut h, "version.create", json!({"project": "LUME", "name": "1.3"}));
    exec(&mut h, "issue.edit", json!({"key": "LUME-2", "fixVersions": ["1.2"]}));
    exec(&mut h, "issue.edit", json!({"key": "LUME-5", "fixVersions": ["1.2"]}));
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "releases", "project": "LUME"}})).unwrap();
    h.run_steps(3);
    // LUME-5 is done, LUME-2 isn't.
    h.get_by_label("1 of 2 issues done");
    h.get_all_by_label("Release").next().unwrap().click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ComboBox, "1 unresolved: move to").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "1.3").click();
    h.run_steps(3);
    h.get_by_label("Confirm release").click();
    h.run_steps(3);
    let d = h.state().session.ws.project_fields(&lume()).unwrap().clone();
    let v12 = d.versions.iter().find(|v| v.name == "1.2").unwrap();
    let v13 = d.versions.iter().find(|v| v.name == "1.3").unwrap();
    assert!(v12.released && v12.release_date.is_some());
    assert_eq!(h.state().session.ws.issues[&key("LUME-2")].fix_versions, vec![v13.id]);
    assert!(h.state().m24.releasing.is_none());
}

#[test]
fn create_dialog_asks_for_required_fields_and_components() {
    let mut h = with_fields();
    exec(&mut h, "field.create", json!({"project": "LUME", "name": "Customer", "type": "text", "required": true}));
    h.state_mut().control(&json!({"method": "ui.create", "params": {"project": "LUME"}})).unwrap();
    h.run_steps(3);
    h.state_mut().create.as_mut().unwrap().summary = "Needs a customer".into();
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(3);
    // Refused: the required field is empty; the dialog stays with the error.
    assert!(h.state().create.as_ref().and_then(|f| f.error.clone()).is_some_and(|e| e.contains("Customer")));
    h.get_by_role_and_label(Role::TextInput, "Customer *").click();
    h.run_steps(2);
    h.event(egui::Event::Text("Acme".into()));
    h.run_steps(2);
    h.get_by_role_and_label(Role::ComboBox, "Components").click();
    h.run_steps(3);
    h.get_by_label("Sync").click();
    h.run_steps(3);
    h.get_by_label("Create").click();
    h.run_steps(3);
    assert!(h.state().create.is_none());
    let cust = field_id(&h, "Customer");
    let i = h.state().session.ws.issues.values().find(|i| i.summary == "Needs a customer").unwrap();
    assert_eq!(i.custom.get(&cust), Some(&FieldValue::Text("Acme".into())));
    assert_eq!(i.components.len(), 1);
}

#[test]
fn a_bad_date_is_not_saved() {
    let mut h = with_fields();
    h.state_mut().navigate(View::Issue(key("LUME-2")));
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Due on").click();
    h.run_steps(2);
    h.event(egui::Event::Text("2026-02-30".into()));
    h.run_steps(2);
    h.get_by_label("Use YYYY-MM-DD");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let due = field_id(&h, "Due on");
    assert!(!h.state().session.ws.issues[&key("LUME-2")].custom.contains_key(&due));
    h.get_by_role_and_label(Role::TextInput, "Due on").click();
    h.run_steps(2);
    for _ in 0..12 {
        h.key_press(egui::Key::Backspace);
    }
    h.event(egui::Event::Text("2026-12-01".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let v = h.state().session.ws.issues[&key("LUME-2")].custom.get(&due).cloned();
    assert_eq!(v.map(|v| v.display()), Some("2026-12-01".to_string()));
}

#[test]
fn routes_palette_and_list_columns() {
    let mut h = with_fields();
    let bad = h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "releases", "project": "NOPE"}}));
    assert!(bad.is_err());
    // The palette offers each project's fields editor and releases.
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(2);
    h.event(egui::Event::Text("releases lume".into()));
    h.run_steps(3);
    h.get_by_label("Releases: Lume Mobile App").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Releases(lume()));
    // A custom field column in the issue list shows values.
    exec(&mut h, "issue.edit", json!({"key": "LUME-2", "custom": {"Severity": "Critical"}}));
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    h.get_by_label("Columns").click();
    h.run_steps(3);
    h.get_by_label("Severity").click();
    h.run_steps(3);
    h.get_by_label("Critical");
    // The header sorts by the field (quoted).
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.get_by_label("Severity").click();
    h.run_steps(3);
    assert!(h.state().jql.contains("ORDER BY \"Severity\" ASC"), "{}", h.state().jql);
}
