//! The create-issue dialog.

use egui::RichText;
use serde_json::json;
use trackcraft_core::IssueKey;
use trackcraft_model::{PRIORITIES, SprintState, Workspace};

use crate::theme::Tokens;
use crate::widgets::{Candidate, issue_picker, label_editor, short, user_picker};
use crate::{Action, ComboBox, CreateForm, TrackApp, View, icons};

pub(crate) fn new_form(app: &TrackApp, project: Option<String>) -> CreateForm {
    let project = project.or_else(|| app.session.ws.projects.keys().next().map(ToString::to_string)).unwrap_or_default();
    let parent = match &app.view {
        View::Issue(k) => app.session.ws.issues.get(k).filter(|i| i.issue_type != "Sub-task").map(|i| i.key.to_string()).unwrap_or_default(),
        _ => String::new(),
    };
    // Opened from a story or task, the new issue is its sub-task; from an epic, a child story.
    let parent_type = IssueKey::parse(&parent).ok().and_then(|k| app.session.ws.issues.get(&k)).map(|i| i.issue_type.clone());
    let issue_type = match parent_type.as_deref() {
        Some("Epic") => "Story",
        Some(_) => "Sub-task",
        None => "Task",
    };
    CreateForm { project, issue_type: issue_type.into(), priority: "Medium".into(), parent, ..CreateForm::default() }
}

/// Whether `ty` is a sub-task type in `project`.
fn subtask_type(ws: &Workspace, project: &str, ty: &str) -> bool {
    ws.project(project).ok().and_then(|p| p.issue_types.iter().find(|it| it.name.eq_ignore_ascii_case(ty))).is_some_and(|it| it.subtask)
}

/// Issues the new issue may hang under: for a sub-task, a standard issue of the project; for
/// other types (not epics), any issue that isn't a sub-task, epics first.
fn parent_candidates(ws: &Workspace, project: &str, subtask: bool) -> Vec<Candidate> {
    let mut v: Vec<Candidate> = ws
        .issues
        .values()
        .filter(|o| !ws.is_subtask(o))
        .filter(|o| !subtask || (o.key.project.as_str() == project && !o.issue_type.eq_ignore_ascii_case("Epic")))
        .map(|o| Candidate { key: o.key.clone(), summary: o.summary.clone(), issue_type: o.issue_type.clone() })
        .collect();
    v.sort_by_key(|c| !c.issue_type.eq_ignore_ascii_case("Epic"));
    v
}

pub(crate) fn create_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut form) = app.create.take() else { return };
    let mut close = false;
    let mut submit = false;
    let can_create = crate::widgets::can(app, "issue.create", &json!({"project": form.project})); // UI-37
    let ws = &app.session.ws;
    let me = app.session.user.clone();
    let suggestions = super::detail::workspace_labels(&mut None, 0, ws);
    let modal = egui::Modal::new(egui::Id::unique("create-issue")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.heading("Create issue");
        ui.add_space(8.0);
        egui::Grid::new("create-grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            let l_c_project = ui.label("Project").id;
            let pname = ws.project(&form.project).map(|p| format!("{} ({})", p.name, p.key)).unwrap_or_else(|_| form.project.clone());
            ComboBox::from_id_salt("c-project")
                .selected_text(pname)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for p in ws.projects.values() {
                        let k = p.key.to_string();
                        if ui.selectable_label(form.project == k, format!("{} ({k})", p.name)).clicked() && form.project != k {
                            form.project = k;
                            // Sprints belong to one project.
                            form.sprint = None;
                        }
                    }
                })
                .response
                .labelled_by(l_c_project);
            ui.end_row();
            let l_c_type = ui.label("Type").id;
            let types: Vec<String> = ws.project(&form.project).map(|p| p.issue_types.iter().map(|t| t.name.clone()).collect()).unwrap_or_default();
            ui.horizontal(|ui| {
                icons::issue_type(ui, &form.issue_type, 16.0);
                ComboBox::from_id_salt("c-type")
                    .selected_text(form.issue_type.clone())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for ty in &types {
                            if ui.selectable_label(&form.issue_type == ty, ty).clicked() {
                                form.issue_type = ty.clone();
                            }
                        }
                    })
                    .response
                    .labelled_by(l_c_type);
            });
            ui.end_row();
            ui.label("Summary");
            let r = ui.add(egui::TextEdit::singleline(&mut form.summary).desired_width(360.0).hint_text("What needs doing?"));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            if form.summary.is_empty() && !r.has_focus() && ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.end_row();
            ui.label("Description");
            ui.add(egui::TextEdit::multiline(&mut form.description).desired_rows(4).desired_width(360.0).hint_text("Markdown supported"));
            ui.end_row();
            let l_c_priority = ui.label("Priority").id;
            ComboBox::from_id_salt("c-priority")
                .selected_text(form.priority.clone())
                .width(160.0)
                .show_ui(ui, |ui| {
                    for p in PRIORITIES {
                        if ui.selectable_label(form.priority == p, p).clicked() {
                            form.priority = p.to_string();
                        }
                    }
                })
                .response
                .labelled_by(l_c_priority);
            ui.end_row();
            // UI-9: the shared person picker (search, Unassigned, Assign to me).
            let l_c_assignee = ui.label("Assignee").id;
            let picked = user_picker(ui, ws, &me, "c-assignee", form.assignee.as_deref(), l_c_assignee, 200.0);
            if let Some(choice) = picked.choice {
                form.assignee = choice;
            }
            ui.end_row();
            // UI-9: labels as chips, suggestions from the workspace.
            let l_c_labels = ui.label("Labels").id;
            if let Some(next) = label_editor(ui, t, "c-labels", &mut form.label_draft, &form.labels, &suggestions, l_c_labels, 360.0) {
                form.labels = next;
            }
            ui.end_row();
            let subtask = subtask_type(ws, &form.project, &form.issue_type);
            // UI-9: an open sprint of the project (a sub-task follows its parent's sprint).
            let sprints: Vec<(trackcraft_core::Id, String, SprintState)> = ws
                .sprints
                .values()
                .filter(|s| s.project.as_str() == form.project && s.state != SprintState::Closed)
                .map(|s| (s.id, s.name.clone(), s.state))
                .collect();
            if !subtask && !sprints.is_empty() {
                let l_c_sprint = ui.label("Sprint").id;
                let shown = form.sprint.and_then(|id| sprints.iter().find(|s| s.0 == id)).map_or_else(|| "Backlog".to_string(), |s| s.1.clone());
                ComboBox::from_id_salt("c-sprint")
                    .selected_text(shown)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(form.sprint.is_none(), "Backlog").clicked() {
                            form.sprint = None;
                        }
                        for (id, name, state) in &sprints {
                            let label = if *state == SprintState::Active { format!("{name} (active)") } else { name.clone() };
                            if ui.selectable_label(form.sprint == Some(*id), label).clicked() {
                                form.sprint = Some(*id);
                            }
                        }
                    })
                    .response
                    .labelled_by(l_c_sprint);
                ui.end_row();
            }
            // UI-9: parent as an issue picker; an epic has none.
            if !form.issue_type.eq_ignore_ascii_case("Epic") {
                let l_c_parent = ui.label("Parent").id;
                let current = IssueKey::parse(form.parent.trim()).ok();
                let selected = match &current {
                    Some(p) => ws.issues.get(p).map_or_else(|| p.to_string(), |o| short(p, &o.summary, 30)),
                    None if subtask => "Choose the parent issue".to_string(),
                    None => "None".to_string(),
                };
                let project = form.project.clone();
                let picked =
                    issue_picker(ui, "c-parent", selected, || parent_candidates(ws, &project, subtask), current.as_ref(), true, l_c_parent, 300.0);
                if let Some(choice) = picked.choice {
                    form.parent = choice.map(|k| k.to_string()).unwrap_or_default();
                }
                ui.end_row();
            }
        });
        if let Some(e) = &form.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(10.0);
        // Mod+Enter creates from anywhere in the dialog (the description too).
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            submit = true;
        }
        ui.horizontal(|ui| {
            let create = ui.add_enabled(can_create, egui::Button::new(RichText::new("Create").color(t.on_accent).strong()).fill(t.accent)); // UI-37
            let create = create.on_disabled_hover_text(crate::widgets::denied("create issues in this project"));
            if create.on_hover_text(format!("Create ({})", crate::key("Mod+Enter"))).clicked() {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
            ui.add_space(12.0);
            ui.checkbox(&mut form.create_another, "Create another").on_hover_text("Keep the dialog open, with the same project and type");
        });
    });
    // Escape or a backdrop click: typed text is kept unless the person confirms (`guard.rs`).
    if modal.should_close() {
        app.push(Action::CloseCreate);
    }
    if submit {
        let subtask = subtask_type(&app.session.ws, &form.project, &form.issue_type);
        let mut p = json!({"project": form.project, "type": form.issue_type, "summary": form.summary, "priority": form.priority});
        if !form.description.trim().is_empty() {
            p["description"] = json!(form.description);
        }
        if let Some(a) = &form.assignee {
            p["assignee"] = json!(a);
        }
        if !form.parent.trim().is_empty() && !form.issue_type.eq_ignore_ascii_case("Epic") {
            p["parent"] = json!(form.parent.trim());
        }
        // A label typed but not yet confirmed with Enter still counts.
        let mut labels = form.labels.clone();
        for w in form.label_draft.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()) {
            if !labels.iter().any(|l| l == w) {
                labels.push(w.to_string());
            }
        }
        if !labels.is_empty() {
            p["labels"] = json!(labels);
        }
        if let Some(s) = form.sprint.filter(|_| !subtask) {
            p["sprint"] = json!(s.0);
        }
        match app.session.run("issue.create", &p) {
            Ok(v) => {
                app.jql_ran = None;
                let key = v["key"].as_str().and_then(|k| IssueKey::parse(k).ok());
                if form.create_another {
                    // Keep project, type, priority, sprint and parent; clear what was typed.
                    if let Some(k) = key {
                        app.notify(format!("Created {k}"));
                    }
                    app.create = Some(CreateForm {
                        project: form.project,
                        issue_type: form.issue_type,
                        priority: form.priority,
                        parent: form.parent,
                        sprint: form.sprint,
                        create_another: true,
                        ..CreateForm::default()
                    });
                    return;
                }
                if let Some(k) = key {
                    app.notify(format!("Created {k}"));
                    app.push(Action::Nav(View::Issue(k)));
                }
                return;
            }
            Err(e) => form.error = Some(e),
        }
    }
    if !close {
        app.create = Some(form);
    }
}
