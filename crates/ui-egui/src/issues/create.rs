//! The create-issue dialog.

use egui::RichText;
use serde_json::json;
use trackcraft_core::IssueKey;
use trackcraft_model::PRIORITIES;

use crate::theme::Tokens;
use crate::widgets::name_of;
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

pub(crate) fn create_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut form) = app.create.take() else { return };
    let mut close = false;
    let mut submit = false;
    let ws = &app.session.ws;
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
                        if ui.selectable_label(form.project == k, format!("{} ({k})", p.name)).clicked() {
                            form.project = k;
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
            let l_c_assignee = ui.label("Assignee").id;
            ComboBox::from_id_salt("c-assignee")
                .selected_text(name_of(ws, form.assignee.as_deref()))
                .width(200.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(form.assignee.is_none(), "Unassigned").clicked() {
                        form.assignee = None;
                    }
                    for u in ws.users.values() {
                        if ui.selectable_label(form.assignee.as_deref() == Some(u.id.as_str()), &u.display_name).clicked() {
                            form.assignee = Some(u.id.clone());
                        }
                    }
                })
                .response
                .labelled_by(l_c_assignee);
            ui.end_row();
            ui.label("Parent");
            ui.add(egui::TextEdit::singleline(&mut form.parent).desired_width(160.0).hint_text("e.g. LUME-1 (optional)"));
            ui.end_row();
        });
        if let Some(e) = &form.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Create").color(t.on_accent).strong()).fill(t.accent)).clicked() {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    // Escape or a backdrop click: typed text is kept unless the person confirms (`guard.rs`).
    if modal.should_close() {
        app.push(Action::CloseCreate);
    }
    if submit {
        let mut p = json!({"project": form.project, "type": form.issue_type, "summary": form.summary, "priority": form.priority});
        if !form.description.trim().is_empty() {
            p["description"] = json!(form.description);
        }
        if let Some(a) = &form.assignee {
            p["assignee"] = json!(a);
        }
        if !form.parent.trim().is_empty() {
            p["parent"] = json!(form.parent.trim());
        }
        match app.session.run("issue.create", &p) {
            Ok(v) => {
                app.jql_ran = None;
                if let Some(k) = v["key"].as_str().and_then(|k| IssueKey::parse(k).ok()) {
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
