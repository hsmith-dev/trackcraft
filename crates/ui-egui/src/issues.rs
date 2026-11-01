//! Tracker views: the JQL issue list, the board, issue detail and the create dialog.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::{IssueKey, ProjectKey};
use trackcraft_model::{Issue, PRIORITIES, StatusCategory, Workspace, priority_rank};

use crate::shell::card;
use crate::theme::Tokens;
use crate::{Action, ComboBox, CreateForm, Then, TrackApp, View, icons, markup_view};

fn pill(ui: &mut Ui, t: &Tokens, cat: StatusCategory, text: &str) -> egui::Response {
    let (fill, fg) = t.status(cat);
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| ui.label(RichText::new(text.to_uppercase()).size(11.0).strong().color(fg)))
        .response
}

fn who(ui: &mut Ui, t: &Tokens, ws: &Workspace, id: Option<&str>, size: f32) {
    match id.and_then(|u| ws.users.get(u)) {
        Some(u) => {
            icons::avatar(ui, t, &u.id, &u.display_name, size);
        }
        None => {
            icons::unassigned(ui, t, size);
        }
    }
}

fn name_of(ws: &Workspace, id: Option<&str>) -> String {
    id.and_then(|u| ws.users.get(u)).map_or_else(|| "Unassigned".to_string(), |u| u.display_name.clone())
}

pub(crate) fn list(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    app.refresh_search();
    ui.heading(RichText::new("Issues").size(22.0));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mut q = app.jql.clone();
        let r = ui.add(
            egui::TextEdit::singleline(&mut q)
                .font(egui::TextStyle::Monospace)
                .desired_width(ui.available_width() - 90.0)
                .hint_text("project = LUME AND statusCategory != Done"),
        );
        if r.changed() {
            app.jql = q;
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Search").clicked() || enter {
            app.jql_ran = None;
        }
    });
    let keys = match &app.jql_result {
        Ok(k) => k.clone(),
        Err(e) => {
            egui::Frame::new().fill(t.danger_soft).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.label(RichText::new(e).color(t.danger));
            });
            return;
        }
    };
    ui.label(RichText::new(format!("{} issue{}", keys.len(), if keys.len() == 1 { "" } else { "s" })).color(t.muted));
    ui.add_space(6.0);
    let ws = &app.session.ws;
    let now = app.session.now();
    let mut open = None;
    card(ui, t, "", |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("issue-grid").num_columns(7).spacing([14.0, 8.0]).striped(true).show(ui, |ui| {
                for h in ["", "Key", "Summary", "Status", "Priority", "Assignee", "Updated"] {
                    ui.label(RichText::new(h).size(12.0).color(t.muted).strong());
                }
                ui.end_row();
                for k in &keys {
                    let Some(i) = ws.issues.get(k) else { continue };
                    icons::issue_type(ui, &i.issue_type, 16.0);
                    if ui.link(RichText::new(i.key.to_string()).monospace()).clicked() {
                        open = Some(i.key.clone());
                    }
                    let summary: String = if i.summary.chars().count() > 70 {
                        format!("{}…", i.summary.chars().take(69).collect::<String>())
                    } else {
                        i.summary.clone()
                    };
                    if ui.add(egui::Label::new(summary).sense(egui::Sense::click())).on_hover_text(&i.summary).clicked() {
                        open = Some(i.key.clone());
                    }
                    pill(ui, t, ws.status_category(i), &i.status);
                    ui.horizontal(|ui| {
                        icons::priority(ui, t, &i.priority, priority_rank(&i.priority).unwrap_or(3), 14.0);
                        ui.label(&i.priority);
                    });
                    ui.horizontal(|ui| {
                        who(ui, t, ws, i.assignee.as_deref(), 18.0);
                        ui.label(name_of(ws, i.assignee.as_deref()));
                    });
                    ui.label(RichText::new(crate::ago(now, i.updated)).color(t.muted));
                    ui.end_row();
                }
            });
        });
    });
    if let Some(k) = open {
        app.push(Action::Nav(View::Issue(k)));
    }
}

pub(crate) fn board(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &ProjectKey) {
    let Ok(project) = app.session.ws.project(key.as_str()) else {
        ui.label("This project no longer exists.");
        return;
    };
    let statuses: Vec<(String, StatusCategory)> = project.workflow.statuses.iter().map(|s| (s.name.clone(), s.category)).collect();
    let pname = project.name.clone();
    ui.horizontal(|ui| {
        ui.heading(RichText::new(format!("{pname} board")).size(22.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+ Create issue").clicked() {
                app.push(Action::OpenCreate(Some(key.to_string())));
            }
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.board_text).hint_text("Filter cards").desired_width(200.0));
        ui.toggle_value(&mut app.board_mine, "Only my issues");
        ui.label(RichText::new("Drag a card to another column to move it through the workflow.").color(t.muted).size(12.0));
    });
    ui.add_space(8.0);

    let ws = &app.session.ws;
    let me = app.session.user.clone();
    let needle = app.board_text.to_lowercase();
    let cards: Vec<&Issue> = ws
        .issues
        .values()
        .filter(|i| &i.key.project == key)
        .filter(|i| !app.board_mine || i.assignee.as_deref() == Some(me.as_str()))
        .filter(|i| needle.is_empty() || i.summary.to_lowercase().contains(&needle) || i.key.to_string().to_lowercase().contains(&needle))
        .collect();
    let mut sorted = cards;
    sorted.sort_by_key(|i| (priority_rank(&i.priority).unwrap_or(9), i.key.number));

    let mut moved: Option<(IssueKey, String)> = None;
    let mut open: Option<IssueKey> = None;
    let col_w = ((ui.available_width() - 12.0 * statuses.len().saturating_sub(1) as f32) / statuses.len().max(1) as f32).clamp(220.0, 320.0);
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (status, cat) in &statuses {
                let in_col: Vec<&&Issue> = sorted.iter().filter(|i| i.status.eq_ignore_ascii_case(status)).collect();
                let frame = egui::Frame::new().fill(t.surface_alt).corner_radius(10).inner_margin(10);
                let (resp, dropped) = ui.dnd_drop_zone::<IssueKey, ()>(frame, |ui| {
                    // The row of columns is horizontal; each column stacks its cards vertically.
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        ui.set_min_height((ui.available_height() - 20.0).max(300.0));
                        ui.horizontal(|ui| {
                            pill(ui, t, *cat, status);
                            ui.label(RichText::new(in_col.len().to_string()).color(t.muted));
                        });
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().id_salt(("col", status)).auto_shrink([false, false]).show(ui, |ui| {
                            for i in in_col {
                                let id = egui::Id::unique(("card", i.key.to_string()));
                                let r = ui.dnd_drag_source(id, i.key.clone(), |ui| card_body(ui, t, ws, i, col_w)).response;
                                if r.clicked() {
                                    open = Some(i.key.clone());
                                }
                                ui.add_space(6.0);
                            }
                        });
                    });
                });
                if resp.response.dnd_hover_payload::<IssueKey>().is_some() {
                    ui.painter().rect_stroke(resp.response.rect, 10.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
                if let Some(k) = dropped {
                    moved = Some(((*k).clone(), status.clone()));
                }
            }
        });
    });
    if let Some((k, to)) = moved
        && app.session.ws.issues.get(&k).is_some_and(|i| !i.status.eq_ignore_ascii_case(&to))
    {
        app.push(Action::Run { id: "issue.transition", params: json!({"key": k.to_string(), "to": to}), then: Then::Toast(format!("{k} → {to}")) });
    }
    if let Some(k) = open {
        app.push(Action::Nav(View::Issue(k)));
    }
}

fn card_body(ui: &mut Ui, t: &Tokens, ws: &Workspace, i: &Issue, w: f32) {
    egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(w - 20.0);
        ui.add(egui::Label::new(&i.summary).wrap());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.set_width(w - 20.0);
            icons::issue_type(ui, &i.issue_type, 15.0);
            ui.label(RichText::new(i.key.to_string()).monospace().size(12.0).color(t.muted));
            if let Some(p) = i.story_points {
                ui.label(RichText::new(format!("{p}")).size(11.0).background_color(t.surface_alt));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                who(ui, t, ws, i.assignee.as_deref(), 20.0);
                icons::priority(ui, t, &i.priority, priority_rank(&i.priority).unwrap_or(3), 13.0);
            });
        });
    });
}

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
        if app.issue_ui.editing_description.is_none() && ui.small_button("Edit").clicked() {
            app.issue_ui.editing_description = Some(issue.description.to_markdown());
        }
    });
    let mut end_edit = false;
    match app.issue_ui.editing_description.as_mut() {
        Some(text) => {
            ui.add(
                egui::TextEdit::multiline(text)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Markdown: **bold**, *italic*, - lists, ## headings"),
            );
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
            ui.add(egui::TextEdit::multiline(&mut app.issue_ui.comment).desired_rows(2).desired_width(f32::INFINITY).hint_text("Add a comment…"));
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
    ComboBox::from_id_salt("status")
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
    let _ = fill;
    ui.add_space(10.0);
    card(ui, t, "Details", |ui| {
        let ws = &app.session.ws;
        let mut acts = Vec::new();
        egui::Grid::new("fields").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            let l_assignee = ui.label(RichText::new("Assignee").color(t.muted)).id;
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    who(ui, t, ws, issue.assignee.as_deref(), 20.0);
                    ComboBox::from_id_salt("assignee")
                        .selected_text(name_of(ws, issue.assignee.as_deref()))
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(issue.assignee.is_none(), "Unassigned").clicked() {
                                acts.push(Action::Run { id: "issue.assign", params: json!({"key": k, "assignee": null}), then: Then::Nothing });
                            }
                            for u in ws.users.values() {
                                if ui.selectable_label(issue.assignee.as_deref() == Some(u.id.as_str()), &u.display_name).clicked() {
                                    acts.push(Action::Run { id: "issue.assign", params: json!({"key": k, "assignee": u.id}), then: Then::Nothing });
                                }
                            }
                        })
                        .response
                        .labelled_by(l_assignee);
                });
                if issue.assignee.as_deref() != Some(app.session.user.as_str()) && ui.link(RichText::new("Assign to me").size(12.0)).clicked() {
                    acts.push(Action::Run { id: "issue.assignToMe", params: json!({"key": k}), then: Then::Nothing });
                }
            });
            ui.end_row();

            ui.label(RichText::new("Reporter").color(t.muted));
            ui.horizontal(|ui| {
                who(ui, t, ws, Some(&issue.reporter), 20.0);
                ui.label(name_of(ws, Some(&issue.reporter)));
            });
            ui.end_row();

            let l_priority = ui.label(RichText::new("Priority").color(t.muted)).id;
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
            ui.end_row();

            ui.label(RichText::new("Labels").color(t.muted));
            match app.issue_ui.labels.as_mut() {
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
            }
            ui.end_row();

            ui.label(RichText::new("Story points").color(t.muted));
            let mut pts = issue.story_points.unwrap_or(0.0);
            let r = ui.add(egui::DragValue::new(&mut pts).range(0.0..=1000.0).speed(0.25));
            if r.drag_stopped() || (r.lost_focus() && r.changed()) || (r.changed() && !r.dragged()) {
                acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "storyPoints": pts}), then: Then::Nothing });
            }
            ui.end_row();

            let now = app.session.now();
            ui.label(RichText::new("Created").color(t.muted));
            ui.label(crate::ago(now, issue.created));
            ui.end_row();
            ui.label(RichText::new("Updated").color(t.muted));
            ui.label(crate::ago(now, issue.updated));
            ui.end_row();
            if let Some(r) = issue.resolved {
                ui.label(RichText::new("Resolved").color(t.muted));
                ui.label(crate::ago(now, r));
                ui.end_row();
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
    if modal.should_close() {
        close = true;
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
                    app.navigate(View::Issue(k));
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
