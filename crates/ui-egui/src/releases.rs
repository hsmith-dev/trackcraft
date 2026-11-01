//! M2.4: a project's releases — its versions with progress (issues fixed in each, and how many
//! are done), and Release (`version.release`), optionally moving unresolved issues on.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::StatusCategory;

use crate::theme::Tokens;
use crate::widgets::{card, empty_state};
use crate::{Action, ComboBox, Then, TrackApp, View};

/// The release confirmation open on one version.
#[derive(Debug, Clone, Default)]
pub(crate) struct Releasing {
    pub id: Id,
    /// Where unresolved issues go (`None`: they stay).
    pub move_to: Option<Id>,
    /// The release date typed (empty: today).
    pub date: String,
}

pub(crate) fn show(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    let Some(name) = app.session.ws.projects.get(project).map(|p| p.name.clone()) else {
        ui.label("This project no longer exists.");
        return;
    };
    let defs = app.session.ws.project_fields(project).cloned().unwrap_or_default();
    let mut acts: Vec<Action> = Vec::new();
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Releases").size(22.0));
        ui.add(egui::Label::new(RichText::new(&name).color(t.muted)).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Manage versions").on_hover_text("Create, rename, date and archive versions").clicked() {
                acts.push(Action::Nav(View::ProjectFields(project.clone())));
            }
            ui.checkbox(&mut app.m24.show_archived, "Show archived");
        });
    });
    ui.add_space(8.0);
    let shown: Vec<_> = defs.versions.iter().filter(|v| app.m24.show_archived || !v.archived).collect();
    if shown.is_empty() {
        card(ui, t, "", |ui| {
            empty_state(ui, t, "", "No versions to show. Add versions under Manage versions.", None);
        });
    }
    egui::ScrollArea::vertical().id_salt("releases").auto_shrink([false, false]).show(ui, |ui| {
        for v in shown {
            let ws = &app.session.ws;
            let (done, total) = ws.version_progress(v.id);
            let unresolved = total.saturating_sub(done);
            card(ui, t, "", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(&v.name).strong().size(16.0)).truncate());
                    let (text, color) = match (v.released, v.archived) {
                        (_, true) => ("ARCHIVED", t.muted),
                        (true, _) => ("RELEASED", t.ok),
                        _ => ("UNRELEASED", t.accent),
                    };
                    ui.label(RichText::new(text).size(11.0).strong().color(color));
                    if let Some(d) = v.release_date {
                        let what = if v.released { "Released" } else { "Due" };
                        ui.label(RichText::new(format!("{what} {}", d.format("%Y-%m-%d"))).color(t.muted));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !v.released
                            && app.m24.releasing.as_ref().is_none_or(|r| r.id != v.id)
                            && ui.button("Release").on_hover_text(format!("Release {}", v.name)).clicked()
                        {
                            app.m24.releasing = Some(Releasing { id: v.id, ..Releasing::default() });
                        }
                        if ui.link("View issues").on_hover_text(format!("Issues fixed in {}", v.name)).clicked() {
                            acts.push(Action::Search(format!("project = {project} AND fixVersion = \"{}\" ORDER BY rank", v.name.replace('"', ""))));
                        }
                    });
                });
                if !v.description.is_empty() {
                    ui.label(RichText::new(&v.description).color(t.muted));
                }
                progress(ui, t, done, total);
                ui.label(
                    RichText::new(if total == 0 { "No issues yet".to_string() } else { format!("{done} of {total} issues done") })
                        .size(12.0)
                        .color(t.muted),
                );
                // The confirmation: a date and where unresolved issues go.
                if let Some(r) = app.m24.releasing.as_mut().filter(|r| r.id == v.id) {
                    let mut close = false;
                    egui::Frame::new().fill(t.surface_alt).corner_radius(6).inner_margin(8).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let l_date = ui.label("Release date").id;
                            ui.add(egui::TextEdit::singleline(&mut r.date).desired_width(110.0).hint_text("Today")).labelled_by(l_date);
                            let others: Vec<(Id, String)> =
                                defs.versions.iter().filter(|o| o.id != v.id && !o.released && !o.archived).map(|o| (o.id, o.name.clone())).collect();
                            if unresolved > 0 {
                                let l_move = ui.label(format!("{unresolved} unresolved: move to")).id;
                                let cur =
                                    r.move_to.and_then(|id| others.iter().find(|o| o.0 == id)).map_or("Leave them", |o| o.1.as_str()).to_string();
                                ComboBox::from_id_salt(("release-move", v.id.0))
                                    .selected_text(cur)
                                    .width(140.0)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut r.move_to, None, "Leave them");
                                        for (id, n) in &others {
                                            ui.selectable_value(&mut r.move_to, Some(*id), n);
                                        }
                                    })
                                    .response
                                    .labelled_by(l_move);
                            }
                        });
                        let date_ok = crate::fields_admin::check_day(&r.date);
                        if let Err(why) = &date_ok {
                            ui.label(RichText::new(why).size(11.0).color(t.danger));
                        }
                        ui.horizontal(|ui| {
                            let go = ui.add_enabled(
                                date_ok.is_ok(),
                                egui::Button::new(RichText::new("Confirm release").color(t.on_accent).strong()).fill(t.accent),
                            );
                            if go.clicked() {
                                let mut p = json!({"id": v.id.0});
                                if !r.date.trim().is_empty() {
                                    p["date"] = json!(r.date.trim());
                                }
                                if let Some(m) = r.move_to {
                                    p["moveUnresolvedTo"] = json!(m.0);
                                }
                                acts.push(Action::Run {
                                    id: "version.release",
                                    params: p,
                                    then: Then::Toast(format!("Released {}", crate::issues::short_name(&v.name))),
                                });
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
                    if close {
                        app.m24.releasing = None;
                    }
                }
            });
            ui.add_space(8.0);
        }
    });
    for a in acts {
        app.push(a);
    }
}

/// A painted progress bar: done (Done colour) over the rest.
fn progress(ui: &mut Ui, t: &Tokens, done: usize, total: usize) {
    let w = ui.available_width().min(520.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 8.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, t.surface_alt);
    if total > 0 {
        let frac = (done as f32 / total as f32).clamp(0.0, 1.0);
        let mut filled = rect;
        filled.set_width(rect.width() * frac);
        p.rect_filled(filled, 4.0, t.progress(StatusCategory::Done));
    }
    r.on_hover_text(format!("{done} of {total} done"));
}
