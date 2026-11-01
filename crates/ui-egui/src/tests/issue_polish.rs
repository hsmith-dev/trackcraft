//! UI-9 / UI-13: issue view polish (links, comment edit, type and parent, story points, labels,
//! the person picker), the activity tabs and the create dialog's new fields.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use trackcraft_model::{LinkKind, SprintState};

use super::*;
use crate::View;
use crate::issues::ActivityTab;

fn on_issue(k: &str) -> Harness<'static, TrackApp> {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key(k)));
    h.run_steps(3);
    h
}

fn type_text(h: &mut Harness<'static, TrackApp>, s: &str) {
    h.event(egui::Event::Text(s.into()));
    h.run_steps(2);
}

#[test]
fn issue_link_add_and_remove() {
    let mut h = on_issue("LUME-3");
    h.get_by_label("Link issue").click();
    h.run_steps(2);
    assert!(h.state().issue_ui.polish.link_form.is_some());
    // The picker searches by key or summary; pick through its search field and Enter.
    h.get_by_role_and_label(Role::ComboBox, "Issue").click();
    h.run_steps(3);
    type_text(&mut h, "upgrade the sync");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().issue_ui.polish.link_form.as_ref().and_then(|f| f.target.clone()), Some(key("LUME-5")));
    h.get_by_label("Link").click();
    h.run_steps(3);
    let links = &h.state().session.ws.issues[&key("LUME-3")].links;
    assert!(links.iter().any(|l| l.target == key("LUME-5") && l.kind == LinkKind::Relates));
    assert!(h.state().issue_ui.polish.link_form.is_none());
    h.get_by_label("Remove link: relates to LUME-5").click();
    h.run_steps(3);
    assert!(h.state().session.ws.issues[&key("LUME-3")].links.is_empty());

    // An inward link (LUME-6 blocks LUME-2) is removed from the other side too.
    let mut h = on_issue("LUME-2");
    h.get_by_label("Remove link: is blocked by LUME-6").click();
    h.run_steps(3);
    assert!(h.state().session.ws.issues[&key("LUME-6")].links.is_empty());
}

#[test]
fn comment_edit_own_only() {
    let mut h = on_issue("LUME-4");
    // Description's Edit plus riley's own comment; jo's comment has none.
    let edits: Vec<_> = h.get_all_by_label("Edit").collect();
    assert_eq!(edits.len(), 2);
    let own = h.state().session.ws.issues[&key("LUME-4")].comments.iter().find(|c| c.author == "riley").map(|c| c.id).unwrap();
    h.get_all_by_label("Edit").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().issue_ui.polish.editing_comment.as_ref().map(|e| e.0), Some(own));
    h.state_mut().issue_ui.polish.editing_comment.as_mut().unwrap().1 = "Reproduced again today.".into();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(3);
    let c = h.state().session.ws.issues[&key("LUME-4")].comments.iter().find(|c| c.id == own).cloned().unwrap();
    assert_eq!(c.body.to_markdown().trim(), "Reproduced again today.");
    assert!(h.state().issue_ui.polish.editing_comment.is_none());
    h.get_by_label("edited");
    // The engine refuses someone else's comment.
    let theirs = h.state().session.ws.issues[&key("LUME-4")].comments.iter().find(|c| c.author == "jo").map(|c| c.id).unwrap();
    assert!(h.state_mut().session.run("issue.editComment", &json!({"key": "LUME-4", "id": theirs.0, "body": "x"})).is_err());
}

#[test]
fn history_tab_lists_transition() {
    let mut h = on_issue("LUME-3");
    h.state_mut().session.run("issue.transition", &json!({"key": "LUME-3", "to": "In Progress"})).unwrap();
    h.run_steps(2);
    h.get_by_label("History").click();
    h.run_steps(3);
    assert_eq!(h.state().activity_tab, ActivityTab::History);
    assert!(h.get_all_by_label("Status:").count() >= 1);
    // "All" interleaves the comments with the changes; Comments has no history lines.
    h.get_by_label("All").click();
    h.run_steps(3);
    assert!(h.get_all_by_label("Status:").count() >= 1);
    h.get_by_label("Comments").click();
    h.run_steps(3);
    assert!(h.query_by_label("Status:").is_none());
    // The tab is kept across issues.
    h.state_mut().activity_tab = ActivityTab::History;
    h.state_mut().navigate(View::Issue(key("LUME-2")));
    h.run_steps(3);
    assert_eq!(h.state().activity_tab, ActivityTab::History);
}

#[test]
fn type_change_to_epic_clears_parent() {
    let mut h = on_issue("LUME-3");
    h.state_mut().session.run("issue.edit", &json!({"key": "LUME-3", "parent": "LUME-1"})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].parent, Some(key("LUME-1")));
    h.get_by_role_and_label(Role::ComboBox, "Type").click();
    h.run_steps(3);
    h.get_by_label("Epic").click();
    h.run_steps(3);
    let i = &h.state().session.ws.issues[&key("LUME-3")];
    assert_eq!(i.issue_type, "Epic");
    assert_eq!(i.parent, None);
    h.get_by_label("None (an epic has no parent)");
    // A refused change (a sub-task needs a parent) is a toast and changes nothing.
    h.get_by_role_and_label(Role::ComboBox, "Type").click();
    h.run_steps(3);
    h.get_by_label("Sub-task").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].issue_type, "Epic");
}

#[test]
fn story_points_one_undo_step_per_commit() {
    let mut h = on_issue("LUME-3");
    let before = h.state().session.ws.issues[&key("LUME-3")].story_points;
    let depth = h.state().session.undo_depth();
    h.get_by_role_and_label(Role::TextInput, "Story points").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    type_text(&mut h, "1");
    type_text(&mut h, "2");
    type_text(&mut h, ".5");
    // Nothing is committed while typing.
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].story_points, before);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].story_points, Some(12.5));
    assert_eq!(h.state().session.undo_depth(), depth + 1);
    h.state_mut().session.run("edit.undo", &json!({})).unwrap();
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].story_points, before);
    // Empty clears.
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Story points").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].story_points, None);
}

#[test]
fn label_editor_adds_and_removes_chips() {
    let mut h = on_issue("LUME-3");
    h.get_by_role_and_label(Role::TextInput, "Labels").click();
    h.run_steps(2);
    type_text(&mut h, "polish");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().session.ws.issues[&key("LUME-3")].labels.contains(&"polish".to_string()));
    // A comma also adds; a label used elsewhere is suggested.
    type_text(&mut h, "later,");
    h.run_steps(2);
    assert!(h.state().session.ws.issues[&key("LUME-3")].labels.contains(&"later".to_string()));
    type_text(&mut h, "cra");
    h.get_by_label("crash").click();
    h.run_steps(3);
    assert!(h.state().session.ws.issues[&key("LUME-3")].labels.contains(&"crash".to_string()));
    h.get_by_label("Remove label polish").click();
    h.run_steps(3);
    let labels = &h.state().session.ws.issues[&key("LUME-3")].labels;
    assert!(!labels.contains(&"polish".to_string()));
    assert!(labels.contains(&"offline".to_string()));
}

#[test]
fn user_picker_searches_and_assigns() {
    let mut h = on_issue("LUME-3");
    h.get_by_role_and_label(Role::ComboBox, "Assignee").click();
    h.run_steps(3);
    type_text(&mut h, "jo");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("jo"));
    h.get_by_role_and_label(Role::ComboBox, "Assignee").click();
    h.run_steps(3);
    h.get_by_label("Unassigned").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].assignee, None);
}

#[test]
fn create_with_sprint_lands_in_the_sprint() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.create", "params": {"project": "LUME"}})).unwrap();
    h.run_steps(3);
    let (id, name) = h.state().session.ws.sprints.values().find(|s| s.state == SprintState::Active).map(|s| (s.id, s.name.clone())).unwrap();
    h.state_mut().create.as_mut().unwrap().summary = "In the sprint".into();
    h.run_steps(1);
    h.get_by_role_and_label(Role::ComboBox, "Sprint").click();
    h.run_steps(3);
    h.get_by_label(&format!("{name} (active)")).click();
    h.run_steps(2);
    assert_eq!(h.state().create.as_ref().unwrap().sprint, Some(id));
    h.get_by_label("Create").click();
    h.run_steps(3);
    let made = h.state().session.ws.issues.values().find(|i| i.summary == "In the sprint").unwrap();
    assert_eq!(made.sprint, Some(id));
    assert!(h.state().create.is_none());
}

#[test]
fn create_another_keeps_project_and_type() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.create", "params": {"project": "OPS"}})).unwrap();
    h.run_steps(3);
    {
        let f = h.state_mut().create.as_mut().unwrap();
        f.summary = "First".into();
        f.issue_type = "Task".into();
        f.labels = vec!["desk".into()];
    }
    // The chip row grows the dialog; let it settle before clicking.
    h.run_steps(4);
    h.get_by_role_and_label(Role::CheckBox, "Create another").click();
    h.run_steps(2);
    assert!(h.state().create.as_ref().unwrap().create_another);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(3);
    let made = h.state().session.ws.issues.values().find(|i| i.summary == "First").unwrap();
    assert_eq!(made.labels, vec!["desk".to_string()]);
    let f = h.state().create.as_ref().unwrap();
    assert_eq!((f.project.as_str(), f.issue_type.as_str(), f.summary.as_str()), ("OPS", "Task", ""));
    assert!(f.create_another && f.labels.is_empty());
}
