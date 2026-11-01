//! Settings (UI-7): a left tab list and a form on the right.
//!
//! - **Appearance**: theme (System / Light / Dark, `TrackApp::dark`) and density.
//! - **Workspace**: rename (`workspace.rename`), the file and Save as…, autosave on/off and its
//!   interval (the desktop host reads [`TrackApp::autosave_secs`]).
//! - **People**: every user with inline edit (`user.edit`), Add person (`user.add`) and who you
//!   are acting as (`user.switch`).
//! - **Shortcuts**: the keymap (`keys::table`), read-only for now.
//!
//! Theme, density and autosave are machine-local preferences kept in egui's persisted memory
//! (eframe saves it with the window state), not in the workspace.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::theme::{Density, ThemeChoice, Tokens};
use crate::{Action, ComboBox, FileRequest, TrackApp, View, keys};

/// A Settings tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Appearance,
    Workspace,
    People,
    Shortcuts,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Appearance, Tab::Workspace, Tab::People, Tab::Shortcuts];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Appearance => "Appearance",
            Tab::Workspace => "Workspace",
            Tab::People => "People",
            Tab::Shortcuts => "Shortcuts",
        }
    }

    /// `ui.navigate`'s `"tab"`: a tab name (any case), `""` for the first tab.
    pub fn parse(s: &str) -> Option<Tab> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "appearance" | "preferences" => Some(Tab::Appearance),
            "workspace" => Some(Tab::Workspace),
            "people" | "users" => Some(Tab::People),
            "shortcuts" | "keys" => Some(Tab::Shortcuts),
            _ => None,
        }
    }
}

/// Autosave intervals offered, in seconds.
pub(crate) const INTERVALS: [u64; 5] = [10, 30, 60, 120, 300];
/// The autosave interval until the user picks another.
pub(crate) const DEFAULT_AUTOSAVE_SECS: u64 = 30;
/// Saved intervals outside this range (a hand-edited or corrupt memory file) are clamped.
const SECS_RANGE: (u64, u64) = (5, 3600);

/// What persisted memory holds: (theme, compact, autosave on, autosave seconds).
type Saved = (Option<bool>, bool, bool, u64);

fn prefs_id() -> egui::Id {
    egui::Id::unique("trackcraft.ui7.prefs")
}

/// Machine-local preferences (the theme itself lives in `TrackApp::dark`).
#[derive(Debug, Clone)]
pub(crate) struct Prefs {
    pub density: Density,
    pub autosave: bool,
    pub autosave_secs: u64,
    loaded: bool,
    written: Option<Saved>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { density: Density::Comfortable, autosave: true, autosave_secs: DEFAULT_AUTOSAVE_SECS, loaded: false, written: None }
    }
}

/// An inline user edit on the People tab.
#[derive(Debug, Clone, Default)]
pub(crate) struct UserEdit {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// The Settings view's state.
#[derive(Debug, Default)]
pub(crate) struct SettingsUi {
    pub prefs: Prefs,
    /// The workspace-name field; `None` shows the current name.
    pub name: Option<String>,
    pub name_error: Option<String>,
    pub edit: Option<UserEdit>,
    pub edit_error: Option<String>,
    pub add_id: String,
    pub add_name: String,
    pub add_error: Option<String>,
}

impl TrackApp {
    /// How long after the last change the desktop host autosaves, or `None` when autosave is off.
    /// (A workspace without a file is never autosaved; the host checks that.)
    pub fn autosave_secs(&self) -> Option<u64> {
        let p = &self.settings.prefs;
        p.autosave.then_some(p.autosave_secs.clamp(SECS_RANGE.0, SECS_RANGE.1))
    }

    /// The display density.
    pub fn density(&self) -> Density {
        self.settings.prefs.density
    }
}

/// Loads the preferences from persisted memory once, applies the density, and writes the
/// preferences back after a change. Called every frame from `TrackApp::logic`.
pub(crate) fn sync(app: &mut TrackApp, ctx: &egui::Context) {
    let p = &mut app.settings.prefs;
    if !p.loaded {
        p.loaded = true;
        let saved: Option<Saved> = ctx.data_mut(|d| d.get_persisted(prefs_id()));
        if let Some((dark, compact, autosave, secs)) = saved {
            app.dark = dark;
            p.density = if compact { Density::Compact } else { Density::Comfortable };
            p.autosave = autosave;
            p.autosave_secs = secs.clamp(SECS_RANGE.0, SECS_RANGE.1);
            p.written = Some((dark, compact, autosave, p.autosave_secs));
        }
    }
    let now: Saved = (app.dark, p.density == Density::Compact, p.autosave, p.autosave_secs);
    if p.written != Some(now) {
        p.written = Some(now);
        ctx.data_mut(|d| d.insert_persisted(prefs_id(), now));
    }
    let density = p.density;
    if !density.applied(&ctx.global_style()) {
        ctx.global_style_mut(|s| density.apply(s));
    }
}

/// The preferences saved in `ctx`'s persisted memory.
#[cfg(test)]
pub(crate) fn saved(ctx: &egui::Context) -> Option<(Option<bool>, bool, bool, u64)> {
    ctx.data_mut(|d| d.get_persisted(prefs_id()))
}

/// "30 seconds", "2 minutes".
fn interval_label(secs: u64) -> String {
    match secs {
        60 => "1 minute".into(),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

/// The Settings view.
pub(crate) fn show(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, tab: Tab) {
    ui.heading(RichText::new("Settings").strong());
    ui.add_space(12.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(160.0, 0.0), egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            ui.set_width(160.0);
            ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
            for tb in Tab::ALL {
                let r = ui.add(egui::Button::selectable(tb == tab, tb.label()));
                if r.clicked() && tb != tab {
                    // Switching tabs is not a navigation: it leaves no Back step.
                    app.view = View::Settings(tb);
                }
            }
        });
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(12.0);
        egui::ScrollArea::vertical().id_salt("settings-form").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(720.0);
            ui.vertical(|ui| match tab {
                Tab::Appearance => appearance(app, ui, t),
                Tab::Workspace => workspace(app, ui, t),
                Tab::People => people(app, ui, t),
                Tab::Shortcuts => shortcuts(ui, t),
            });
        });
    });
}

fn section_title(ui: &mut Ui, text: &str) -> egui::Response {
    ui.label(RichText::new(text).strong().size(15.0))
}

fn note(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(text).color(t.muted).size(12.5));
}

fn error(ui: &mut Ui, t: &Tokens, e: &Option<String>) {
    if let Some(e) = e {
        ui.label(RichText::new(e).color(t.danger));
    }
}

fn appearance(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    section_title(ui, "Theme");
    let current = ThemeChoice::from_dark(app.dark);
    ui.horizontal(|ui| {
        for c in ThemeChoice::ALL {
            if ui.add(egui::Button::selectable(c == current, c.label())).clicked() {
                app.dark = c.dark();
            }
        }
    });
    note(ui, t, "System follows your computer's light or dark setting.");
    ui.add_space(16.0);
    section_title(ui, "Density");
    let density = app.settings.prefs.density;
    ui.horizontal(|ui| {
        for d in Density::ALL {
            if ui.add(egui::Button::selectable(d == density, d.label())).clicked() {
                app.settings.prefs.density = d;
            }
        }
    });
    note(ui, t, "Compact fits more rows on screen with tighter spacing.");
    ui.add_space(16.0);
    note(ui, t, "Appearance is saved on this computer, not in the workspace.");
}

/// Runs a settings command now (the view needs its error inline, not as a toast).
fn run_now(app: &mut TrackApp, id: &str, params: &Value) -> Result<Value, String> {
    let r = app.session.run(id, params);
    if r.is_ok() {
        app.jql_ran = None;
    }
    r
}

fn workspace(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    section_title(ui, "Name");
    let current = app.session.ws.name.clone();
    let mut name = app.settings.name.clone().unwrap_or_else(|| current.clone());
    let mut submit = false;
    ui.horizontal(|ui| {
        let label = ui.label("Workspace name");
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(320.0)).labelled_by(label.id);
        submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.add_enabled(name != current, egui::Button::new("Rename")).clicked() {
            submit = true;
        }
    });
    if name != current || app.settings.name.is_some() {
        app.settings.name = Some(name.clone());
    }
    error(ui, t, &app.settings.name_error);
    if submit && name != current {
        match run_now(app, "workspace.rename", &json!({"name": name})) {
            Ok(_) => {
                app.settings.name = None;
                app.settings.name_error = None;
                let shown = app.session.ws.name.clone();
                app.notify_done(format!("Renamed the workspace to {shown}"), true);
            }
            Err(e) => app.settings.name_error = Some(e),
        }
    }

    ui.add_space(16.0);
    section_title(ui, "File");
    let path = app.session.path.as_ref().map(|p| p.display().to_string());
    ui.horizontal(|ui| {
        match &path {
            Some(p) => ui.add(egui::Label::new(RichText::new(p).monospace()).truncate()),
            None => ui.label(RichText::new("Not saved yet").color(t.muted)),
        };
    });
    if ui.button("Save as…").on_hover_text(format!("Save the workspace to a new file ({})", crate::key("Mod+Shift+S"))).clicked() {
        app.push(Action::File(FileRequest::SaveAs));
    }

    ui.add_space(16.0);
    section_title(ui, "Autosave");
    let p = &mut app.settings.prefs;
    ui.checkbox(&mut p.autosave, "Save automatically");
    ui.add_enabled_ui(p.autosave, |ui| {
        ui.horizontal(|ui| {
            let label = ui.label("Save");
            let secs = p.autosave_secs;
            let combo = ComboBox::from_id_salt("autosave-interval")
                .selected_text(interval_label(secs))
                .width(130.0)
                .show_ui(ui, |ui| {
                    for s in INTERVALS {
                        if ui.selectable_label(s == secs, interval_label(s)).clicked() {
                            p.autosave_secs = s;
                        }
                    }
                })
                .response;
            let _ = combo.labelled_by(label.id);
            ui.label("after the last change");
        });
    });
    if path.is_none() {
        note(ui, t, "A workspace without a file is never autosaved: use Save as… first.");
    } else {
        note(ui, t, "Autosave is a preference on this computer.");
    }
}

fn people(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let me = app.session.user.clone();
    let me_name = crate::widgets::name_of(&app.session.ws, Some(&me));
    section_title(ui, "Acting as");
    note(ui, t, &format!("You are acting as {me_name}. Changes you make are recorded under this person."));
    ui.add_space(12.0);
    section_title(ui, "People");
    let users: Vec<(String, String, String)> =
        app.session.ws.users.values().map(|u| (u.id.clone(), u.display_name.clone(), u.email.clone().unwrap_or_default())).collect();
    let mut save = false;
    let mut cancel = false;
    let mut start: Option<UserEdit> = None;
    let mut switch: Option<(String, String)> = None;
    egui::Grid::new("settings-people").num_columns(4).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
        ui.label(RichText::new("ID").color(t.muted));
        let h_name = ui.label(RichText::new("Display name").color(t.muted));
        let h_email = ui.label(RichText::new("Email").color(t.muted));
        ui.label("");
        ui.end_row();
        for (id, name, email) in &users {
            cell(ui, 120.0, egui::Label::new(RichText::new(id).monospace()).truncate());
            let editing = app.settings.edit.as_ref().is_some_and(|e| e.id == *id);
            if editing && let Some(e) = &mut app.settings.edit {
                let n = ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(180.0)).labelled_by(h_name.id);
                let m = ui.add(egui::TextEdit::singleline(&mut e.email).desired_width(220.0).hint_text("name@example.com")).labelled_by(h_email.id);
                save |= (n.lost_focus() || m.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    save |= ui.button("Save").clicked();
                    cancel |= ui.button("Cancel").clicked();
                });
            } else {
                let you = if *id == me { " (you)" } else { "" };
                cell(ui, 180.0, egui::Label::new(format!("{name}{you}")).truncate());
                if email.is_empty() {
                    cell(ui, 220.0, egui::Label::new(RichText::new("No email").color(t.muted)));
                } else {
                    cell(ui, 220.0, egui::Label::new(email).truncate());
                }
                ui.horizontal(|ui| {
                    if ui.button("Edit").on_hover_text(format!("Edit {name}")).clicked() {
                        start = Some(UserEdit { id: id.clone(), name: name.clone(), email: email.clone() });
                    }
                    if ui.add_enabled(*id != me, egui::Button::new("Act as")).on_hover_text(format!("Act as {name}")).clicked() {
                        switch = Some((id.clone(), name.clone()));
                    }
                });
            }
            ui.end_row();
        }
    });
    error(ui, t, &app.settings.edit_error);
    if let Some(e) = start {
        app.settings.edit = Some(e);
        app.settings.edit_error = None;
    }
    if cancel {
        app.settings.edit = None;
        app.settings.edit_error = None;
    } else if save && let Some(e) = app.settings.edit.clone() {
        let email = if e.email.trim().is_empty() { Value::Null } else { json!(e.email.trim()) };
        match run_now(app, "user.edit", &json!({"id": e.id, "name": e.name, "email": email})) {
            Ok(_) => {
                app.settings.edit = None;
                app.settings.edit_error = None;
                app.notify_done(format!("Updated {}", e.id), true);
            }
            Err(err) => app.settings.edit_error = Some(err),
        }
    }
    if let Some((id, name)) = switch {
        match run_now(app, "user.switch", &json!({"id": id})) {
            Ok(_) => app.notify(format!("Acting as {name}")),
            Err(e) => app.notify_error(e),
        }
    }

    ui.add_space(16.0);
    section_title(ui, "New person");
    let mut add = false;
    ui.horizontal(|ui| {
        let l = ui.label("ID");
        let r = ui.add(egui::TextEdit::singleline(&mut app.settings.add_id).desired_width(120.0).hint_text("ann")).labelled_by(l.id);
        add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let l = ui.label("Name");
        let r = ui.add(egui::TextEdit::singleline(&mut app.settings.add_name).desired_width(200.0).hint_text("Ann Lee")).labelled_by(l.id);
        add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        add |= ui.add_enabled(!app.settings.add_id.trim().is_empty(), egui::Button::new("Add person")).clicked();
    });
    note(ui, t, "The ID is how JQL and mentions refer to someone; it never changes.");
    error(ui, t, &app.settings.add_error);
    if add {
        let id = app.settings.add_id.trim().to_string();
        let name = app.settings.add_name.trim().to_string();
        match run_now(app, "user.add", &json!({"id": id, "name": name})) {
            Ok(_) => {
                app.settings.add_id.clear();
                app.settings.add_name.clear();
                app.settings.add_error = None;
                app.notify_done(format!("Added {id}"), true);
            }
            Err(e) => app.settings.add_error = Some(e),
        }
    }
}

/// A grid cell `w` points wide: long text truncates instead of widening the table.
fn cell(ui: &mut Ui, w: f32, label: egui::Label) {
    let h = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(egui::vec2(w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(w);
        ui.add(label);
    });
}

fn shortcuts(ui: &mut Ui, t: &Tokens) {
    note(ui, t, "Every keyboard shortcut, by where it works. Changing them comes in a later release.");
    ui.add_space(8.0);
    for scope in keys::Scope::ALL {
        let rows: Vec<&keys::Binding> = keys::table().iter().filter(|b| b.scope == scope).collect();
        if rows.is_empty() {
            continue;
        }
        ui.add_space(8.0);
        section_title(ui, scope.title());
        egui::Grid::new(("settings-keys", scope.title())).num_columns(2).spacing([24.0, 4.0]).min_col_width(200.0).show(ui, |ui| {
            for b in rows {
                ui.label(RichText::new(crate::key(&b.keys)).monospace().color(t.accent));
                ui.label(&b.label);
                ui.end_row();
            }
        });
    }
}
