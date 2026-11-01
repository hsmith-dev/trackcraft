//! M2.4: custom fields, components and versions on the issue view and in the create dialog,
//! and their cells in the issue list.
//!
//! Every edit is one `issue.edit` (`custom` by `customfield_<id>`, `components`, `fixVersions`,
//! `affectsVersions` by name), so it is one undo step and one history group.

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_model::{FieldDef, FieldKind, FieldValue, Issue, ProjectFields, Workspace, parse_day};

use crate::theme::Tokens;
use crate::widgets::{card, chip_picker, commit_field, field_row, number_field, user_picker};
use crate::{Action, ComboBox, Then, TrackApp, View};

/// A field name as a row label: at most 28 characters.
pub(crate) fn short_name(name: &str) -> String {
    if name.chars().count() > 28 { format!("{}...", name.chars().take(26).collect::<String>()) } else { name.to_string() }
}

/// Text fields: URLs must look like one.
fn check_text(kind: &FieldKind, s: &str) -> Result<(), String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(());
    }
    match kind {
        FieldKind::Date => parse_day(s).map(|_| ()).map_err(|_| "Use YYYY-MM-DD".to_string()),
        FieldKind::Url => {
            let l = s.to_ascii_lowercase();
            if (l.starts_with("http://") || l.starts_with("https://")) && !s.contains(char::is_whitespace) {
                Ok(())
            } else {
                Err("Starts with http:// or https://".into())
            }
        }
        _ => Ok(()),
    }
}

fn names(ids: &[Id], name: impl Fn(Id) -> Option<String>) -> Vec<String> {
    ids.iter().filter_map(|id| name(*id)).collect()
}

/// The "More fields" card under Details: components, versions, then each custom field.
pub(crate) fn section(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str) {
    let defs = app.session.ws.project_fields(&issue.key.project).cloned().unwrap_or_default();
    let project = issue.key.project.clone();
    let mut acts: Vec<Action> = Vec::new();
    ui.add_space(10.0);
    card(ui, t, "More fields", |ui| {
        if defs.is_empty() {
            ui.label(RichText::new("This project has no custom fields, components or versions.").color(t.muted));
        }
        let ws = &app.session.ws;
        let me = app.session.user.clone();
        egui::Grid::new("more-fields").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            if !defs.components.is_empty() {
                field_row(ui, t, "Components", |ui, l| {
                    let all: Vec<String> = defs.components.iter().map(|c| c.name.clone()).collect();
                    let sel = names(&issue.components, |id| defs.component(id).map(|c| c.name.clone()));
                    if let Some(next) = chip_picker(ui, t, "issue-components", "component", &sel, &all, l, 150.0) {
                        acts.push(edit(k, json!({"components": next})));
                    }
                });
            }
            if !defs.versions.is_empty() {
                for (label, salt, key, ids) in [
                    ("Fix versions", "issue-fix-versions", "fixVersions", &issue.fix_versions),
                    ("Affects versions", "issue-affects-versions", "affectsVersions", &issue.affects_versions),
                ] {
                    field_row(ui, t, label, |ui, l| {
                        let sel = names(ids, |id| defs.version(id).map(|v| v.name.clone()));
                        // Archived versions aren't offered, but stay when already set.
                        let all: Vec<String> = defs.versions.iter().filter(|v| !v.archived).map(|v| v.name.clone()).collect();
                        if let Some(next) = chip_picker(ui, t, salt, "version", &sel, &all, l, 150.0) {
                            acts.push(edit(k, json!({ key: next })));
                        }
                    });
                }
            }
            for f in &defs.fields {
                let label = if f.required { format!("{} *", row_name(&f.name)) } else { row_name(&f.name) };
                field_row(ui, t, &label, |ui, l| {
                    if let Some(v) = editor(ui, t, ws, &me, f, issue.custom.get(&f.id), l) {
                        acts.push(edit(k, json!({"custom": {format!("customfield_{}", f.id): v}})));
                    }
                });
            }
        });
        ui.add_space(4.0);
        if ui.link(RichText::new("Manage fields").size(12.0)).on_hover_text("Custom fields, components and versions of this project").clicked() {
            acts.push(Action::Nav(View::ProjectFields(project.clone())));
        }
    });
    for a in acts {
        app.push(a);
    }
}

fn edit(k: &str, mut p: Value) -> Action {
    p["key"] = json!(k);
    Action::Run { id: "issue.edit", params: p, then: Then::Nothing }
}

/// The editor for one custom field's value. Returns the new value (JSON for `issue.edit`'s
/// `custom`, `null` to clear) when an edit was committed.
fn editor(ui: &mut Ui, t: &Tokens, ws: &Workspace, me: &str, f: &FieldDef, v: Option<&FieldValue>, l: egui::Id) -> Option<Value> {
    let salt = format!("cf-{}", f.id);
    match &f.kind {
        FieldKind::Number => {
            let cur = match v {
                Some(FieldValue::Number(n)) => Some(*n),
                _ => None,
            };
            number_field(ui, &salt, cur, l, 100.0).map(|n| json!(n))
        }
        FieldKind::Select { options } => {
            let cur = v.map(FieldValue::display);
            let mut out = None;
            ComboBox::from_id_salt(salt)
                .selected_text(cur.clone().unwrap_or_else(|| "None".into()))
                .width(140.0)
                .show_ui(ui, |ui| {
                    if !f.required && ui.selectable_label(cur.is_none(), "None").clicked() && cur.is_some() {
                        out = Some(Value::Null);
                    }
                    for o in options {
                        if ui.selectable_label(cur.as_deref() == Some(o.as_str()), o).clicked() && cur.as_deref() != Some(o.as_str()) {
                            out = Some(json!(o));
                        }
                    }
                })
                .response
                .labelled_by(l);
            out
        }
        FieldKind::MultiSelect { options } => {
            let sel = match v {
                Some(FieldValue::MultiSelect(s)) => s.clone(),
                _ => vec![],
            };
            chip_picker(ui, t, &salt, "option", &sel, options, l, 150.0).map(|next| if next.is_empty() { Value::Null } else { json!(next) })
        }
        FieldKind::User => {
            let cur = match v {
                Some(FieldValue::User(u)) => Some(u.as_str()),
                _ => None,
            };
            ui.horizontal(|ui| {
                crate::widgets::who(ui, t, ws, cur, 18.0);
                user_picker(ui, ws, me, &salt, cur, l, 120.0).choice.map(|c| c.map_or(Value::Null, |u| json!(u)))
            })
            .inner
        }
        kind @ (FieldKind::Text | FieldKind::Date | FieldKind::Url) => {
            let cur = v.map(FieldValue::display).unwrap_or_default();
            let hint = if matches!(kind, FieldKind::Date) { "YYYY-MM-DD" } else { "None" };
            // A URL shows as a link above its field (stacked: the fields column is narrow).
            ui.vertical(|ui| {
                if matches!(kind, FieldKind::Url) && !cur.is_empty() {
                    ui.hyperlink_to(RichText::new(row_name(&cur)).size(12.0), &cur);
                }
                commit_field(ui, t, &salt, &cur, hint, l, 145.0, |s| check_text(kind, s))
            })
            .inner
            .map(|s| if s.is_empty() { Value::Null } else { json!(s) })
        }
    }
}

// ---- the create dialog ----

/// Components and required custom fields typed in the create dialog (M2.4).
#[derive(Debug, Default, Clone)]
pub(crate) struct CreateExtra {
    /// The project these belong to (a project change clears them).
    pub project: String,
    pub components: Vec<String>,
    /// Typed values of required fields, by field id (as text; the engine parses per kind).
    pub custom: BTreeMap<u64, String>,
}

fn defs_of(ws: &Workspace, project: &str) -> ProjectFields {
    ws.project(project).ok().and_then(|p| ws.project_fields(&p.key)).cloned().unwrap_or_default()
}

/// The dialog's grid rows: Components (when the project has any) and each required field.
pub(crate) fn create_rows(ui: &mut Ui, t: &Tokens, ws: &Workspace, me: &str, project: &str, x: &mut CreateExtra) {
    if x.project != project {
        *x = CreateExtra { project: project.to_string(), ..CreateExtra::default() };
    }
    let defs = defs_of(ws, project);
    if !defs.components.is_empty() {
        let l = ui.label("Components").id;
        let all: Vec<String> = defs.components.iter().map(|c| c.name.clone()).collect();
        if let Some(next) = chip_picker(ui, t, "c-components", "component", &x.components, &all, l, 360.0) {
            x.components = next;
        }
        ui.end_row();
    }
    for f in defs.fields.iter().filter(|f| f.required) {
        let l = ui.label(format!("{} *", short_name(&f.name))).id;
        let text = x.custom.entry(f.id.0).or_default();
        match &f.kind {
            FieldKind::Select { options } => {
                ComboBox::from_id_salt(format!("c-cf-{}", f.id))
                    .selected_text(if text.is_empty() { "Choose".to_string() } else { text.clone() })
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for o in options {
                            if ui.selectable_label(text == o, o).clicked() {
                                *text = o.clone();
                            }
                        }
                    })
                    .response
                    .labelled_by(l);
            }
            FieldKind::MultiSelect { options } => {
                let sel: Vec<String> = text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
                if let Some(next) = chip_picker(ui, t, &format!("c-cf-{}", f.id), "option", &sel, options, l, 360.0) {
                    *text = next.join(", ");
                }
            }
            FieldKind::User => {
                let cur = Some(text.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
                if let Some(c) = user_picker(ui, ws, me, &format!("c-cf-{}", f.id), cur.as_deref(), l, 200.0).choice {
                    *text = c.unwrap_or_default();
                }
            }
            kind => {
                let hint = match kind {
                    FieldKind::Date => "YYYY-MM-DD",
                    FieldKind::Number => "A number",
                    FieldKind::Url => "https://",
                    _ => "Required",
                };
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(text).desired_width(200.0).hint_text(hint)).labelled_by(l);
                    if let Err(why) = check_text(kind, text) {
                        ui.label(RichText::new(why).size(11.0).color(t.danger));
                    }
                });
            }
        }
        ui.end_row();
    }
}

/// Adds the dialog's components and required values to `issue.create`'s params.
pub(crate) fn create_params(p: &mut Value, x: &CreateExtra) {
    if !x.components.is_empty() {
        p["components"] = json!(x.components);
    }
    let custom: serde_json::Map<String, Value> =
        x.custom.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(id, v)| (format!("customfield_{id}"), json!(v.trim()))).collect();
    if !custom.is_empty() {
        p["custom"] = Value::Object(custom);
    }
}

// ---- issue list columns ----

/// Extra list columns (M2.4): components, fix versions, or a custom field by name (`cf:Name`).
pub(crate) fn column_header(col: &str) -> String {
    match col {
        "component" => "Components".into(),
        "fixVersion" => "Fix versions".into(),
        "affectedVersion" => "Affects versions".into(),
        c => short_name(c.strip_prefix("cf:").unwrap_or(c)),
    }
}

/// The JQL field a column header sorts by.
pub(crate) fn column_field(col: &str) -> String {
    match col.strip_prefix("cf:") {
        Some(name) => format!("\"{}\"", name.replace('"', "")),
        None => col.to_string(),
    }
}

/// The columns the chooser offers: the built-in three, then every custom field name in the
/// workspace (de-duplicated, at most 100).
pub(crate) fn column_choices(ws: &Workspace) -> Vec<String> {
    let mut v: Vec<String> = ["component", "fixVersion", "affectedVersion"].map(str::to_string).to_vec();
    let mut seen = std::collections::BTreeSet::new();
    for d in ws.fields.values() {
        for f in &d.fields {
            if seen.len() < 100 && seen.insert(f.name.to_lowercase()) {
                v.push(format!("cf:{}", f.name));
            }
        }
    }
    v
}

/// A cell's text for `issue` (empty when the issue's project lacks the field).
pub(crate) fn cell_text(ws: &Workspace, issue: &Issue, col: &str) -> String {
    let Some(d) = ws.project_fields(&issue.key.project) else { return String::new() };
    match col {
        "component" => names(&issue.components, |id| d.component(id).map(|c| c.name.clone())).join(", "),
        "fixVersion" => names(&issue.fix_versions, |id| d.version(id).map(|v| v.name.clone())).join(", "),
        "affectedVersion" => names(&issue.affects_versions, |id| d.version(id).map(|v| v.name.clone())).join(", "),
        c => {
            let name = c.strip_prefix("cf:").unwrap_or(c);
            d.fields
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
                .and_then(|f| issue.custom.get(&f.id))
                .map(|v| match v {
                    FieldValue::User(u) => crate::widgets::name_of(ws, Some(u)),
                    other => other.display(),
                })
                .unwrap_or_default()
        }
    }
}

/// A field name as a row label in the narrow fields column: at most 18 characters.
fn row_name(name: &str) -> String {
    if name.chars().count() > 18 { format!("{}...", name.chars().take(16).collect::<String>()) } else { name.to_string() }
}
