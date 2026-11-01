//! File › Import… (UI-fixes): pick what to import (a Jira CSV or JSON export, or a Confluence
//! space export) and, for Confluence, an optional target space; the host shows the file dialog
//! (`FileRequest::Import`) and hands the path back to [`TrackApp::import_file`], which runs
//! `workspace.import` (one undo step) and shows the report: counts and warnings.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{Action, ComboBox, FileRequest, ImportKind, TrackApp};

/// Longest space key typed into the dialog.
const MAX_KEY: usize = 10;
/// Warnings the report lists (the engine caps them too).
const MAX_SHOWN_WARNINGS: usize = 200;

/// The import dialogs' state.
#[derive(Debug, Default)]
pub(crate) struct ImportUi {
    /// The "what to import" dialog is open.
    pub choose: bool,
    pub kind: Kind,
    /// Confluence: an unpacked export folder rather than a .zip (or one page file).
    pub folder: bool,
    /// Confluence: the target space key (empty: the export's own).
    pub space: String,
    /// The last import's report (its dialog is open while set).
    pub report: Option<Report>,
}

/// The three choices of the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Kind {
    #[default]
    JiraCsv,
    JiraJson,
    Confluence,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::JiraCsv => "Jira CSV export (.csv)",
            Kind::JiraJson => "Jira JSON export (REST v3 search, .json)",
            Kind::Confluence => "Confluence space export (folder, .zip or one page)",
        }
    }
}

/// What an import did, from `workspace.import`'s result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Report {
    pub file: String,
    /// (label, count) for every non-zero count.
    pub counts: Vec<(String, usize)>,
    pub warnings: Vec<String>,
    pub more_warnings: usize,
}

impl Report {
    pub(crate) fn from_json(file: &str, v: &Value) -> Report {
        let n = |k: &str| match &v[k] {
            Value::Array(a) => a.len(),
            other => other.as_u64().and_then(|x| usize::try_from(x).ok()).unwrap_or(0),
        };
        let counts = [
            ("projects", "Projects"),
            ("issues", "Issues"),
            ("users", "People"),
            ("sprints", "Sprints"),
            ("comments", "Comments"),
            ("links", "Issue links"),
            ("spaces", "Spaces"),
            ("pages", "Pages"),
        ]
        .into_iter()
        .map(|(k, label)| (label.to_string(), n(k)))
        .filter(|(_, c)| *c > 0)
        .collect();
        let warnings =
            v["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).take(MAX_SHOWN_WARNINGS).map(str::to_string).collect();
        Report { file: file.chars().take(300).collect(), counts, warnings, more_warnings: n("moreWarnings") }
    }

    /// "Imported 12 issues, 3 people" for the toast.
    pub(crate) fn summary(&self) -> String {
        if self.counts.is_empty() {
            return "Import finished: nothing new".to_string();
        }
        let one = |l: &str| match l {
            "People" => "person".to_string(),
            l => l.strip_suffix('s').unwrap_or(l).to_lowercase(),
        };
        let parts: Vec<String> =
            self.counts.iter().take(3).map(|(l, c)| if *c == 1 { format!("1 {}", one(l)) } else { format!("{c} {}", l.to_lowercase()) }).collect();
        format!("Imported {}", parts.join(", "))
    }
}

/// The engine's `kind` for a file request.
pub(crate) fn kind_param(k: ImportKind) -> &'static str {
    match k {
        ImportKind::JiraCsv => "jiraCsv",
        ImportKind::JiraJson => "jiraJson",
        ImportKind::Confluence { .. } => "confluence",
    }
}

/// `workspace.import`'s params: the kind, the path and (Confluence only) the typed space.
pub(crate) fn params(app: &TrackApp, kind: ImportKind, path: &str) -> Value {
    let mut p = json!({"kind": kind_param(kind), "path": path});
    let space = app.import.space.trim();
    if matches!(kind, ImportKind::Confluence { .. }) && !space.is_empty() {
        p["space"] = json!(space.to_ascii_uppercase());
    }
    p
}

fn request(ui: &ImportUi) -> ImportKind {
    match ui.kind {
        Kind::JiraCsv => ImportKind::JiraCsv,
        Kind::JiraJson => ImportKind::JiraJson,
        Kind::Confluence => ImportKind::Confluence { folder: ui.folder },
    }
}

/// Both import dialogs: what to import, then the report.
pub(crate) fn dialogs(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    if app.import.choose {
        choose(app, ctx, t);
    }
    if app.import.report.is_some() {
        report(app, ctx, t);
    }
}

fn choose(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let mut spaces: Vec<(String, String)> = app.session.ws.spaces.values().map(|s| (s.key.to_string(), s.name.clone())).collect();
    spaces.sort_by_key(|s| s.1.to_lowercase());
    let mut pick = false;
    let mut close = false;
    let st = &mut app.import;
    let modal = egui::Modal::new(egui::Id::unique("import-choose")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.heading("Import");
        ui.label(RichText::new("Bring issues or pages in from an export file. The whole import is one step: Undo takes it back.").color(t.muted));
        ui.add_space(8.0);
        for k in [Kind::JiraCsv, Kind::JiraJson, Kind::Confluence] {
            ui.radio_value(&mut st.kind, k, k.label());
        }
        if st.kind == Kind::Confluence {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.selectable_value(&mut st.folder, false, "A .zip or page file");
                ui.selectable_value(&mut st.folder, true, "An unpacked folder");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                space_picker(ui, &mut st.space, &spaces);
            });
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let label = if st.kind == Kind::Confluence && st.folder { "Choose folder\u{2026}" } else { "Choose file\u{2026}" };
            pick = ui.add(egui::Button::new(RichText::new(label).color(t.on_accent).strong()).fill(t.accent)).clicked();
            close = ui.button("Cancel").clicked();
        });
    });
    if close || modal.should_close() {
        app.import.choose = false;
    }
    if pick {
        app.import.choose = false;
        let req = request(&app.import);
        app.push(Action::File(FileRequest::Import(req)));
    }
}

/// The target space: an existing one from the list, or a new key typed in (empty: the export's).
fn space_picker(ui: &mut Ui, space: &mut String, spaces: &[(String, String)]) {
    ui.vertical(|ui| {
        ui.label("Target space (optional)");
        let shown = match spaces.iter().find(|s| s.0.eq_ignore_ascii_case(space.trim())) {
            Some(s) => format!("{} ({})", s.1, s.0),
            None if space.trim().is_empty() => "The space named in the export".to_string(),
            None => format!("New space {}", space.trim().to_ascii_uppercase()),
        };
        ComboBox::from_id_salt("import-space").selected_text(shown).width(320.0).show_ui(ui, |ui| {
            if ui.selectable_label(space.trim().is_empty(), "The space named in the export").clicked() {
                space.clear();
            }
            for (key, name) in spaces {
                if ui.selectable_label(space.trim().eq_ignore_ascii_case(key), format!("{name} ({key})")).clicked() {
                    *space = key.clone();
                }
            }
        });
        ui.horizontal(|ui| {
            let k = ui.label(RichText::new("or a new space key").size(12.5));
            ui.add(egui::TextEdit::singleline(space).desired_width(110.0).hint_text("e.g. DOCS").char_limit(MAX_KEY)).labelled_by(k.id);
        });
    });
}

fn report(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(rep) = app.import.report.clone() else { return };
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("import-report")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.heading("Import finished");
        ui.add(egui::Label::new(RichText::new(&rep.file).color(t.muted).size(12.0)).truncate());
        ui.add_space(8.0);
        if rep.counts.is_empty() {
            ui.label("Nothing new was found in this export.");
        }
        egui::Grid::new("import-counts").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
            for (label, n) in &rep.counts {
                ui.label(label);
                ui.label(RichText::new(n.to_string()).strong());
                ui.end_row();
            }
        });
        let nw = rep.warnings.len().saturating_add(rep.more_warnings);
        ui.add_space(8.0);
        if nw == 0 {
            ui.label(RichText::new("No warnings.").color(t.ok));
        } else {
            ui.label(RichText::new(format!("{nw} warning{}", if nw == 1 { "" } else { "s" })).strong().color(t.danger));
            egui::ScrollArea::vertical().id_salt("import-warnings").max_height(220.0).show(ui, |ui| {
                for w in &rep.warnings {
                    ui.add(egui::Label::new(RichText::new(w).size(12.5)).wrap());
                }
                if rep.more_warnings > 0 {
                    ui.label(RichText::new(format!("and {} more", rep.more_warnings)).color(t.muted).size(12.5));
                }
            });
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            close = ui.add(egui::Button::new(RichText::new("Done").color(t.on_accent).strong()).fill(t.accent)).clicked();
            if ui.button("Undo import").on_hover_text("Take the whole import back").clicked() {
                app.push(Action::Run { id: "edit.undo", params: json!({}), then: crate::Then::Toast("Import undone".into()) });
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        app.import.report = None;
    }
}

impl TrackApp {
    /// Runs `workspace.import` on `path` (as picked for `kind`; Confluence uses the dialog's
    /// target space) and shows the report. The host calls this after its file dialog.
    pub fn import_file(&mut self, kind: ImportKind, path: &std::path::Path) {
        let shown = path.display().to_string();
        let p = params(self, kind, &shown);
        self.import_with(&p, &shown);
    }

    /// Runs one import and opens its report (or shows the error).
    pub(crate) fn import_with(&mut self, p: &Value, file: &str) {
        let before = self.session.revision();
        match self.session.run("workspace.import", p) {
            Ok(v) => {
                self.jql_ran = None;
                let rep = Report::from_json(file, &v);
                let changed = self.session.revision() != before;
                self.notify_done(rep.summary(), changed);
                self.import.report = Some(rep);
            }
            Err(e) => self.notify_error(format!("Import failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_counts_and_summary() {
        let v = json!({"projects": ["IMP"], "issues": 12, "users": ["a", "b"], "sprints": 0, "comments": 3, "links": 0,
            "spaces": [], "pages": 0, "warnings": ["row 3: no summary"], "moreWarnings": 4});
        let r = Report::from_json("/tmp/x.csv", &v);
        assert_eq!(r.counts, vec![("Projects".into(), 1), ("Issues".into(), 12), ("People".into(), 2), ("Comments".into(), 3)]);
        assert_eq!(r.summary(), "Imported 1 project, 12 issues, 2 people");
        assert_eq!((r.warnings.len(), r.more_warnings), (1, 4));
        let empty = Report::from_json("f", &json!(null));
        assert!(empty.counts.is_empty() && empty.summary().contains("nothing"));
    }
}
