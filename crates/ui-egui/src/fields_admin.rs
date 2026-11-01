//! M2.4: a project's custom fields, components and versions — the editor the project settings
//! "Fields" tab mounts (temporarily its own view, `View::ProjectFields`).
//!
//! Everything runs through `field.*`, `component.*` and `version.*`; the engine refuses what the
//! user may not do (project administrators only), which shows as an error toast.

use egui::{RichText, Ui};
use serde_json::{Value, json};
use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::{FIELD_KINDS, FieldKind};

use crate::theme::Tokens;
use crate::widgets::{card, commit_field, empty_state, name_of, remove_button, user_picker};
use crate::{Action, ComboBox, Then, TrackApp, View};

/// The fields editor's and the releases view's state (M2.4).
#[derive(Debug, Default)]
pub(crate) struct FieldsUi {
    pub new_field: NewField,
    pub new_component: String,
    pub new_version: String,
    pub new_version_date: String,
    /// The field whose options are being edited, and the options text.
    pub editing_options: Option<(Id, String)>,
    /// The releases view: the version being released, where its unresolved issues go, the date.
    pub releasing: Option<crate::releases::Releasing>,
    /// The releases view lists archived versions too.
    pub show_archived: bool,
    /// The field, component or version being renamed (ids are unique across all three).
    pub renaming: Option<Id>,
}

#[derive(Debug, Default)]
pub(crate) struct NewField {
    pub name: String,
    /// An index into [`FIELD_KINDS`].
    pub kind: usize,
    /// Comma-separated options (select kinds).
    pub options: String,
    pub required: bool,
}

/// Human names of [`FIELD_KINDS`], same order.
const KIND_NAMES: [&str; 7] = ["Text", "Number", "Date", "Select list", "Multi-select list", "User", "URL"];

pub(crate) fn kind_label(k: &FieldKind) -> &'static str {
    FIELD_KINDS.iter().position(|n| *n == k.name()).and_then(|i| KIND_NAMES.get(i)).copied().unwrap_or("Text")
}

fn split_options(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|o| !o.is_empty()).map(str::to_string).collect()
}

fn run(id: &'static str, params: Value, toast: &str) -> Action {
    Action::Run { id, params, then: Then::Toast(toast.to_string()) }
}

pub(crate) fn show(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    let Some(name) = app.session.ws.projects.get(project).map(|p| p.name.clone()) else {
        ui.label("This project no longer exists.");
        return;
    };
    let defs = app.session.ws.project_fields(project).cloned().unwrap_or_default();
    let p = project.to_string();
    let mut acts: Vec<Action> = Vec::new();
    egui::ScrollArea::vertical().id_salt("fields-admin").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Fields").size(22.0));
            ui.add(egui::Label::new(RichText::new(&name).color(t.muted)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Releases").on_hover_text("Versions with their progress").clicked() {
                    acts.push(Action::Nav(View::Releases(project.clone())));
                }
            });
        });
        ui.label(RichText::new("Custom fields, components and versions of this project. Only project administrators can change them.").color(t.muted));
        ui.add_space(10.0);

        // ---- custom fields ----
        card(ui, t, "Custom fields", |ui| {
            if defs.fields.is_empty() {
                empty_state(ui, t, "", "No custom fields yet. Add one below.", None);
            }
            egui::Grid::new("admin-fields").num_columns(5).spacing([14.0, 8.0]).striped(true).show(ui, |ui| {
                if !defs.fields.is_empty() {
                    for h in ["Name", "Type", "Options", "Required", ""] {
                        ui.label(RichText::new(h).size(12.0).color(t.muted).strong());
                    }
                    ui.end_row();
                }
                for f in &defs.fields {
                    let l = name_cell(&mut app.m24.renaming, ui, t, f.id, &f.name, "field.edit", &mut acts);
                    ui.label(kind_label(&f.kind));
                    let opts = f.kind.options();
                    if matches!(f.kind, FieldKind::Select { .. } | FieldKind::MultiSelect { .. }) {
                        let editing = app.m24.editing_options.as_ref().is_some_and(|(id, _)| *id == f.id);
                        if editing {
                            ui.horizontal(|ui| {
                                let mut save = false;
                                let mut cancel = false;
                                if let Some((_, text)) = app.m24.editing_options.as_mut() {
                                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(200.0).hint_text("a, b, c")).labelled_by(l);
                                    save = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                }
                                save |= ui.small_button("Save").clicked();
                                cancel |= ui.small_button("Cancel").clicked();
                                if save && let Some((_, text)) = app.m24.editing_options.take() {
                                    acts.push(run("field.edit", json!({"id": f.id.0, "options": split_options(&text)}), "Options saved"));
                                }
                                if cancel {
                                    app.m24.editing_options = None;
                                }
                            });
                        } else {
                            ui.horizontal(|ui| {
                                let shown = opts.join(", ");
                                ui.add(egui::Label::new(crate::issues::short_name(&shown)).truncate()).on_hover_text(&shown);
                                if ui.small_button("Edit").on_hover_text(format!("Edit the options of {}", f.name)).clicked() {
                                    app.m24.editing_options = Some((f.id, opts.join(", ")));
                                }
                            });
                        }
                    } else {
                        ui.label(RichText::new("None").color(t.muted));
                    }
                    let mut req = f.required;
                    if ui.checkbox(&mut req, "").on_hover_text("New issues must set it").labelled_by(l).changed() {
                        acts.push(run("field.edit", json!({"id": f.id.0, "required": req}), if req { "Field is required" } else { "Field is optional" }));
                    }
                    if remove_button(ui, t, &format!("Delete field {}", f.name), 18.0).clicked() {
                        acts.push(run("field.delete", json!({"id": f.id.0}), &format!("Deleted field {}", short(&f.name))));
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("Add a field").strong());
            let nf = &mut app.m24.new_field;
            ui.horizontal_wrapped(|ui| {
                let l_name = ui.label("Name").id;
                ui.add(egui::TextEdit::singleline(&mut nf.name).desired_width(160.0).hint_text("Customer")).labelled_by(l_name);
                let l_type = ui.label("Type").id;
                ComboBox::from_id_salt("fa-new-kind")
                    .selected_text(KIND_NAMES.get(nf.kind).copied().unwrap_or("Text"))
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (i, n) in KIND_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut nf.kind, i, *n);
                        }
                    })
                    .response
                    .labelled_by(l_type);
                let select = matches!(FIELD_KINDS.get(nf.kind), Some(&"select") | Some(&"multiSelect"));
                if select {
                    let l_opts = ui.label("Options").id;
                    ui.add(egui::TextEdit::singleline(&mut nf.options).desired_width(180.0).hint_text("Low, Medium, High")).labelled_by(l_opts);
                }
                ui.checkbox(&mut nf.required, "Required");
                let ready = !nf.name.trim().is_empty() && (!select || !split_options(&nf.options).is_empty());
                if ui.add_enabled(ready, egui::Button::new("Add field")).clicked() {
                    let mut params = json!({"project": p, "name": nf.name.trim(), "type": FIELD_KINDS.get(nf.kind).copied().unwrap_or("text"), "required": nf.required});
                    if select {
                        params["options"] = json!(split_options(&nf.options));
                    }
                    acts.push(run("field.create", params, "Field added"));
                    *nf = NewField::default();
                }
            });
        });
        ui.add_space(12.0);

        // ---- components ----
        card(ui, t, "Components", |ui| {
            if defs.components.is_empty() {
                empty_state(ui, t, "", "No components yet. Components group issues by part of the product.", None);
            }
            let ws = &app.session.ws;
            let me = app.session.user.clone();
            egui::Grid::new("admin-components").num_columns(4).spacing([14.0, 8.0]).striped(true).show(ui, |ui| {
                if !defs.components.is_empty() {
                    for h in ["Name", "Lead", "Issues", ""] {
                        ui.label(RichText::new(h).size(12.0).color(t.muted).strong());
                    }
                    ui.end_row();
                }
                for c in &defs.components {
                    let l = name_cell(&mut app.m24.renaming, ui, t, c.id, &c.name, "component.edit", &mut acts);
                    ui.horizontal(|ui| {
                        crate::widgets::who(ui, t, ws, c.lead.as_deref(), 18.0);
                        let picked = user_picker(ui, ws, &me, &format!("fa-lead-{}", c.id), c.lead.as_deref(), l, 140.0);
                        if let Some(choice) = picked.choice {
                            acts.push(run("component.edit", json!({"id": c.id.0, "lead": choice}), "Lead changed"));
                        }
                    })
                    .response
                    .on_hover_text(name_of(ws, c.lead.as_deref()));
                    let n = ws.issues.values().filter(|i| &i.key.project == project && i.components.contains(&c.id)).count();
                    if ui.link(n.to_string()).on_hover_text("Show these issues").clicked() {
                        acts.push(Action::Search(format!("project = {p} AND component = \"{}\"", c.name.replace('"', ""))));
                    }
                    if remove_button(ui, t, &format!("Delete component {}", c.name), 18.0).clicked() {
                        acts.push(run("component.delete", json!({"id": c.id.0}), &format!("Deleted component {}", short(&c.name))));
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let l = ui.label("New component").id;
                let r = ui.add(egui::TextEdit::singleline(&mut app.m24.new_component).desired_width(200.0).hint_text("Sync")).labelled_by(l);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ready = !app.m24.new_component.trim().is_empty();
                if (ui.add_enabled(ready, egui::Button::new("Add component")).clicked() || enter) && ready {
                    acts.push(run("component.create", json!({"project": p, "name": app.m24.new_component.trim()}), "Component added"));
                    app.m24.new_component.clear();
                }
            });
        });
        ui.add_space(12.0);

        // ---- versions ----
        card(ui, t, "Versions", |ui| {
            if defs.versions.is_empty() {
                empty_state(ui, t, "", "No versions yet. Versions are the releases issues are fixed in.", None);
            }
            egui::Grid::new("admin-versions").num_columns(5).spacing([14.0, 8.0]).striped(true).show(ui, |ui| {
                if !defs.versions.is_empty() {
                    for h in ["Name", "Release date", "Status", "Archived", ""] {
                        ui.label(RichText::new(h).size(12.0).color(t.muted).strong());
                    }
                    ui.end_row();
                }
                for v in &defs.versions {
                    let l = name_cell(&mut app.m24.renaming, ui, t, v.id, &v.name, "version.edit", &mut acts);
                    let date = v.release_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
                    if let Some(d) = commit_field(ui, t, &format!("fa-verdate-{}", v.id), &date, "YYYY-MM-DD", l, 130.0, check_day) {
                        let d = if d.is_empty() { Value::Null } else { json!(d) };
                        acts.push(run("version.edit", json!({"id": v.id.0, "releaseDate": d}), "Release date changed"));
                    }
                    ui.label(if v.released { RichText::new("Released").color(t.ok) } else { RichText::new("Unreleased").color(t.muted) });
                    let mut arch = v.archived;
                    if ui.checkbox(&mut arch, "").on_hover_text("Archived versions aren't offered on issues").labelled_by(l).changed() {
                        acts.push(run("version.edit", json!({"id": v.id.0, "archived": arch}), if arch { "Version archived" } else { "Version restored" }));
                    }
                    if remove_button(ui, t, &format!("Delete version {}", v.name), 18.0).clicked() {
                        acts.push(run("version.delete", json!({"id": v.id.0}), &format!("Deleted version {}", short(&v.name))));
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let l = ui.label("New version").id;
                ui.add(egui::TextEdit::singleline(&mut app.m24.new_version).desired_width(120.0).hint_text("1.4")).labelled_by(l);
                let l_d = ui.label("Release date").id;
                ui.add(egui::TextEdit::singleline(&mut app.m24.new_version_date).desired_width(110.0).hint_text("YYYY-MM-DD")).labelled_by(l_d);
                let date_ok = check_day(&app.m24.new_version_date).is_ok();
                if !date_ok {
                    ui.label(RichText::new("Use YYYY-MM-DD").size(11.0).color(t.danger));
                }
                let ready = !app.m24.new_version.trim().is_empty() && date_ok;
                if ui.add_enabled(ready, egui::Button::new("Add version")).clicked() {
                    let mut params = json!({"project": p, "name": app.m24.new_version.trim()});
                    if !app.m24.new_version_date.trim().is_empty() {
                        params["releaseDate"] = json!(app.m24.new_version_date.trim());
                    }
                    acts.push(run("version.create", params, "Version added"));
                    app.m24.new_version.clear();
                    app.m24.new_version_date.clear();
                }
            });
        });
    });
    for a in acts {
        app.push(a);
    }
}

/// The name column: the name (its label names the row's other controls) and Rename, or the
/// rename field.
fn name_cell(renaming: &mut Option<Id>, ui: &mut Ui, t: &Tokens, id: Id, name: &str, cmd: &'static str, acts: &mut Vec<Action>) -> egui::Id {
    ui.horizontal(|ui| {
        if *renaming == Some(id) {
            let l = ui.label(RichText::new("Name").color(t.muted)).id;
            let next = commit_field(ui, t, &format!("fa-rename-{id}"), name, "Name", l, 160.0, |s| {
                if s.trim().is_empty() { Err("Give it a name".into()) } else { Ok(()) }
            });
            if let Some(n) = next {
                acts.push(run(cmd, json!({"id": id.0, "name": n}), "Renamed"));
                *renaming = None;
            }
            if ui.small_button("Done").clicked() {
                *renaming = None;
            }
            l
        } else {
            let l = ui.add(egui::Label::new(RichText::new(crate::issues::short_name(name)).strong()).truncate()).on_hover_text(name).id;
            if ui.small_button("Rename").on_hover_text(format!("Rename {name}")).clicked() {
                *renaming = Some(id);
            }
            l
        }
    })
    .inner
}

pub(crate) fn check_day(s: &str) -> Result<(), String> {
    if s.trim().is_empty() || trackcraft_model::parse_day(s).is_ok() { Ok(()) } else { Err("Use YYYY-MM-DD".into()) }
}

fn short(s: &str) -> String {
    crate::issues::short_name(s)
}
