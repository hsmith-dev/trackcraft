//! The issue view: summary, description, sub-tasks, links, activity and the fields column.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::IssueKey;
use trackcraft_model::{Issue, PRIORITIES, StatusCategory, priority_rank};

use crate::theme::Tokens;
use crate::widgets::{card, field_row, name_of, pill, who};
use crate::{Action, ComboBox, Then, TrackApp, View, icons, markup_view};

pub(crate) fn detail(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &IssueKey) {
    let Some(issue) = app.session.ws.issues.get(key).cloned() else {
        ui.label("This issue no longer exists.");
        return;
    };
    let k = key.to_string();
    let ws_now = app.session.now();
    let project_name = app.session.ws.projects.get(&key.project).map(|p| p.name.clone()).unwrap_or_default();
    let cat = app.session.ws.status_category(&issue);
    let transitions: Vec<String> = app
        .session
        .ws
        .projects
        .get(&key.project)
        .map(|p| p.workflow.available(&issue.status).iter().map(|t| t.to.clone()).collect())
        .unwrap_or_default();

    ui.horizontal(|ui| {
        if ui.link(RichText::new(&project_name).color(t.muted)).clicked() {
            app.push(Action::Nav(View::Board(key.project.clone())));
        }
        ui.label(RichText::new("/").color(t.muted));
        if let Some(p) = &issue.parent {
            if ui.link(RichText::new(p.to_string()).color(t.muted)).clicked() {
                app.push(Action::Nav(View::Issue(p.clone())));
            }
            ui.label(RichText::new("/").color(t.muted));
        }
        icons::issue_type(ui, &issue.issue_type, 16.0);
        ui.label(RichText::new(&k).monospace().color(t.muted));
    });
    ui.add_space(4.0);

    // Summary: click to edit.
    match app.issue_ui.editing_summary.as_mut() {
        Some(s) => {
            let r = ui.add(egui::TextEdit::singleline(s).font(egui::FontId::proportional(22.0)).desired_width(f32::INFINITY));
            r.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let v = s.clone();
                app.issue_ui.editing_summary = None;
                app.push(Action::Run { id: "issue.edit", params: json!({"key": k, "summary": v}), then: Then::Nothing });
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.issue_ui.editing_summary = None;
            }
        }
        None => {
            let r = ui
                .add(egui::Label::new(RichText::new(&issue.summary).size(22.0).strong()).sense(egui::Sense::click()))
                .on_hover_text("Click to edit");
            if r.clicked() {
                app.issue_ui.editing_summary = Some(issue.summary.clone());
            }
        }
    }
    ui.add_space(10.0);

    let avail = ui.available_width();
    let right_w = 300.0f32.min(avail * 0.4);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(avail - right_w - 20.0);
            main_column(app, ui, t, &issue, &k, ws_now);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(right_w);
            fields(app, ui, t, &issue, &k, cat, &transitions);
        });
    });
}

fn main_column(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str, now: trackcraft_model::Timestamp) {
    let mut acts: Vec<Action> = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Description").strong());
        if app.issue_ui.editing_description.is_none() && ui.small_button("Edit").on_hover_text("Edit the description (E)").clicked() {
            app.issue_ui.editing_description = Some(issue.description.to_markdown());
        }
    });
    let mut end_edit = false;
    match app.issue_ui.editing_description.as_mut() {
        Some(text) => {
            let r = ui.add(
                egui::TextEdit::multiline(text)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Markdown: **bold**, *italic*, - lists, ## headings"),
            );
            if std::mem::take(&mut app.issue_ui.focus_description) {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    end_edit = true;
                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "description": text.clone()}), then: Then::Nothing });
                }
                if ui.button("Cancel").clicked() {
                    end_edit = true;
                }
            });
        }
        None => card(ui, t, "", |ui| markup_view::show(ui, t, &issue.description)),
    }
    if end_edit {
        app.issue_ui.editing_description = None;
    }
    ui.add_space(14.0);

    let ws = &app.session.ws;
    let children: Vec<&Issue> = ws.issues.values().filter(|i| i.parent.as_ref() == Some(&issue.key)).collect();
    if !children.is_empty() || issue.issue_type != "Sub-task" {
        ui.horizontal(|ui| {
            ui.label(RichText::new(if issue.issue_type == "Epic" { "Child issues" } else { "Sub-tasks" }).strong());
            // The create dialog takes this issue as the parent (see `new_form`).
            if issue.issue_type != "Sub-task" && ui.small_button("+ Add").clicked() {
                acts.push(Action::OpenCreate(Some(issue.key.project.to_string())));
            }
        });
        for c in &children {
            let cat = ws.status_category(c);
            ui.horizontal(|ui| {
                icons::issue_type(ui, &c.issue_type, 15.0);
                if ui.link(c.key.to_string()).clicked() {
                    acts.push(Action::Nav(View::Issue(c.key.clone())));
                }
                ui.label(&c.summary);
                pill(ui, t, cat, &c.status);
            });
        }
        ui.add_space(10.0);
    }

    // Links, both directions.
    let inward: Vec<(String, IssueKey)> = ws
        .issues
        .values()
        .flat_map(|o| o.links.iter().filter(|l| l.target == issue.key).map(move |l| (inverse(l.kind.outward()), o.key.clone())))
        .collect();
    if !issue.links.is_empty() || !inward.is_empty() {
        ui.label(RichText::new("Linked issues").strong());
        let rows: Vec<(String, IssueKey)> = issue.links.iter().map(|l| (l.kind.outward().to_string(), l.target.clone())).chain(inward).collect();
        for (rel, target) in rows {
            ui.horizontal(|ui| {
                ui.label(RichText::new(rel).color(t.muted));
                if ui.link(target.to_string()).clicked() {
                    acts.push(Action::Nav(View::Issue(target.clone())));
                }
                if let Some(o) = ws.issues.get(&target) {
                    ui.label(&o.summary);
                }
            });
        }
        ui.add_space(10.0);
    }

    ui.label(RichText::new("Activity").strong());
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        let me = app.session.user.clone();
        let name = name_of(ws, Some(&me));
        icons::avatar(ui, t, &me, &name, 26.0);
        ui.vertical(|ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut app.issue_ui.comment).desired_rows(2).desired_width(f32::INFINITY).hint_text("Add a comment… (M)"),
            );
            if std::mem::take(&mut app.issue_ui.focus_comment) {
                r.request_focus();
                r.scroll_to_me(None);
            }
            if !app.issue_ui.comment.trim().is_empty() && ui.button("Save comment").clicked() {
                let body = std::mem::take(&mut app.issue_ui.comment);
                acts.push(Action::Run { id: "issue.comment", params: json!({"key": k, "body": body}), then: Then::Nothing });
            }
        });
    });
    ui.add_space(6.0);
    for c in issue.comments.iter().rev() {
        ui.horizontal_top(|ui| {
            let name = name_of(ws, Some(&c.author));
            icons::avatar(ui, t, &c.author, &name, 26.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).strong());
                    ui.label(RichText::new(crate::ago(now, c.created)).color(t.muted).size(12.0));
                    if c.author == app.session.user && ui.small_button("Delete").clicked() {
                        acts.push(Action::Run {
                            id: "issue.deleteComment",
                            params: json!({"key": k, "id": c.id.0}),
                            then: Then::Toast("Comment deleted".into()),
                        });
                    }
                });
                markup_view::show(ui, t, &c.body);
            });
        });
        ui.add_space(8.0);
    }
    for a in acts {
        app.push(a);
    }
}

fn inverse(outward: &str) -> String {
    match outward {
        "blocks" => "is blocked by",
        "duplicates" => "is duplicated by",
        "clones" => "is cloned by",
        other => other,
    }
    .to_string()
}

#[allow(clippy::too_many_arguments)]
fn fields(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str, cat: StatusCategory, transitions: &[String]) {
    let (fill, fg) = t.status(cat);
    let l_status = ui.label(RichText::new("Status").color(t.muted).size(12.0)).id;
    let status = ComboBox::from_id_salt("status")
        .selected_text(RichText::new(issue.status.to_uppercase()).strong().color(fg))
        .width(180.0)
        .show_ui(ui, |ui| {
            if transitions.is_empty() {
                ui.label("No transitions available");
            }
            for to in transitions {
                if ui.selectable_label(false, to).clicked() {
                    app.push(Action::Run { id: "issue.transition", params: json!({"key": k, "to": to}), then: Then::Nothing });
                }
            }
        })
        .response
        .labelled_by(l_status);
    // `t` (see `keys.rs`) opens the menu.
    if std::mem::take(&mut app.issue_ui.open_status) {
        egui::Popup::open_id(ui.ctx(), status.id.with("popup"));
    }
    let _ = fill;
    ui.add_space(10.0);
    card(ui, t, "Details", |ui| {
        let ws = &app.session.ws;
        let mut acts = Vec::new();
        egui::Grid::new("fields").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            field_row(ui, t, "Assignee", |ui, l_assignee| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        who(ui, t, ws, issue.assignee.as_deref(), 20.0);
                        let picker = ComboBox::from_id_salt("assignee")
                            .selected_text(name_of(ws, issue.assignee.as_deref()))
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(issue.assignee.is_none(), "Unassigned").clicked() {
                                    acts.push(Action::Run { id: "issue.assign", params: json!({"key": k, "assignee": null}), then: Then::Nothing });
                                }
                                for u in ws.users.values() {
                                    if ui.selectable_label(issue.assignee.as_deref() == Some(u.id.as_str()), &u.display_name).clicked() {
                                        acts.push(Action::Run {
                                            id: "issue.assign",
                                            params: json!({"key": k, "assignee": u.id}),
                                            then: Then::Nothing,
                                        });
                                    }
                                }
                            })
                            .response
                            .labelled_by(l_assignee);
                        // `a` (see `keys.rs`) opens the picker.
                        if std::mem::take(&mut app.issue_ui.open_assignee) {
                            egui::Popup::open_id(ui.ctx(), picker.id.with("popup"));
                        }
                    });
                    if issue.assignee.as_deref() != Some(app.session.user.as_str())
                        && ui.link(RichText::new("Assign to me").size(12.0)).on_hover_text("Assign to me (I)").clicked()
                    {
                        acts.push(Action::Run { id: "issue.assignToMe", params: json!({"key": k}), then: Then::Nothing });
                    }
                });
            });

            field_row(ui, t, "Reporter", |ui, _| {
                ui.horizontal(|ui| {
                    who(ui, t, ws, Some(&issue.reporter), 20.0);
                    ui.label(name_of(ws, Some(&issue.reporter)));
                });
            });

            field_row(ui, t, "Priority", |ui, l_priority| {
                ui.horizontal(|ui| {
                    icons::priority(ui, t, &issue.priority, priority_rank(&issue.priority).unwrap_or(3), 14.0);
                    ComboBox::from_id_salt("priority")
                        .selected_text(&issue.priority)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            for p in PRIORITIES {
                                if ui.selectable_label(issue.priority == p, p).clicked() {
                                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "priority": p}), then: Then::Nothing });
                                }
                            }
                        })
                        .response
                        .labelled_by(l_priority);
                });
            });

            field_row(ui, t, "Labels", |ui, _| match app.issue_ui.labels.as_mut() {
                Some(text) => {
                    let r = ui.add(egui::TextEdit::singleline(text).hint_text("space separated").desired_width(160.0));
                    if r.lost_focus() {
                        let v = text.clone();
                        app.issue_ui.labels = None;
                        acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "labels": v}), then: Then::Nothing });
                    }
                }
                None => {
                    let shown = if issue.labels.is_empty() { "None".to_string() } else { issue.labels.join(", ") };
                    if ui.add(egui::Label::new(shown).sense(egui::Sense::click())).on_hover_text("Click to edit").clicked() {
                        app.issue_ui.labels = Some(issue.labels.join(" "));
                    }
                }
            });

            field_row(ui, t, "Story points", |ui, _| {
                let mut pts = issue.story_points.unwrap_or(0.0);
                let r = ui.add(egui::DragValue::new(&mut pts).range(0.0..=1000.0).speed(0.25));
                if r.drag_stopped() || (r.lost_focus() && r.changed()) || (r.changed() && !r.dragged()) {
                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "storyPoints": pts}), then: Then::Nothing });
                }
            });

            let now = app.session.now();
            field_row(ui, t, "Created", |ui, _| ui.label(crate::ago(now, issue.created)));
            field_row(ui, t, "Updated", |ui, _| ui.label(crate::ago(now, issue.updated)));
            if let Some(r) = issue.resolved {
                field_row(ui, t, "Resolved", |ui, _| ui.label(crate::ago(now, r)));
            }
        });
        for a in acts {
            app.push(a);
        }
    });
    ui.add_space(10.0);
    if app.issue_ui.confirm_delete {
        ui.label(RichText::new(format!("Delete {k} and its sub-tasks?")).color(t.danger));
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Delete").color(t.danger)).clicked() {
                app.issue_ui.confirm_delete = false;
                app.push(Action::Run { id: "issue.delete", params: json!({"key": k}), then: Then::Nav(View::Board(issue.key.project.clone())) });
                app.push(Action::Toast(format!("Deleted {k} ({} to undo)", crate::key("Mod+Z"))));
            }
            if ui.button("Cancel").clicked() {
                app.issue_ui.confirm_delete = false;
            }
        });
    } else if ui.small_button("Delete issue…").clicked() {
        app.issue_ui.confirm_delete = true;
    }
}
