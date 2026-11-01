//! Project settings and space settings (UI-37): a heading, a tab bar and one form.
//!
//! - **Access** (projects and spaces): the access level for everyone without a role
//!   (`access.setProjectAccess` / `access.setSpaceAccess`) and the per-person roles
//!   (`access.setProjectRole` / `access.setSpaceRole`). Rules: `docs/permissions.md`.
//!
//! Reached from the gear on a project or space row in the sidebar, the page "More" menu
//! (Space settings) and `ui.navigate {"view": "projectSettings" | "spaceSettings"}`.

use egui::{RichText, Ui};
use serde_json::{Value, json};
use trackcraft_core::ProjectKey;
use trackcraft_model::{AccessLevel, Grants, ProjectRole, SpaceRole};

use crate::theme::Tokens;
use crate::widgets::{can, denied, name_of, person_adder, remove_button};
use crate::{Action, ComboBox, TrackApp, View};

/// A project settings tab (`pub` like `settings::Tab`, for `View`; the module is crate-private).
///
/// Another work item adds a "Fields" tab: append `Fields` after `Access` here, add it to
/// [`ProjectTab::ALL`], [`ProjectTab::label`] and [`ProjectTab::parse`], and draw it in the
/// `match` in [`project`]. The tab bar needs nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectTab {
    #[default]
    Access,
    /// Custom fields, components and versions (M2.4's editor, mounted here when merging).
    Fields,
}

impl ProjectTab {
    pub(crate) const ALL: &'static [ProjectTab] = &[ProjectTab::Access, ProjectTab::Fields];

    pub(crate) fn label(self) -> &'static str {
        match self {
            ProjectTab::Access => "Access",
            ProjectTab::Fields => "Fields",
        }
    }

    /// `ui.navigate`'s `"tab"`: a tab name (any case), `""` for the first tab.
    pub(crate) fn parse(s: &str) -> Option<ProjectTab> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "access" | "permissions" => Some(ProjectTab::Access),
            "fields" | "components" | "versions" => Some(ProjectTab::Fields),
            _ => None,
        }
    }
}

/// The settings forms' state: the person picked to add, their role, and the last error.
#[derive(Debug, Default)]
pub(crate) struct AccessUi {
    /// The project or space the fields below belong to (they reset on another one).
    pub target: String,
    pub add_user: Option<String>,
    pub add_role: usize,
    pub error: Option<String>,
}

/// Project or space: the words and commands that differ.
struct Kind {
    /// "project" or "space": the command parameter and the noun in sentences.
    noun: &'static str,
    access_cmd: &'static str,
    role_cmd: &'static str,
    /// (command name, label, what it lets someone do), strongest first.
    roles: [(&'static str, &'static str, &'static str); 3],
    /// What open, limited and private mean for people without a role.
    levels: [(AccessLevel, &'static str, &'static str); 3],
}

const PROJECT: Kind = Kind {
    noun: "project",
    access_cmd: "access.setProjectAccess",
    role_cmd: "access.setProjectRole",
    roles: [
        ("administrator", "Administrator", "Everything a member can, plus changing who has access and removing other people's comments."),
        ("member", "Member", "Creates, edits, assigns, moves and deletes issues, and plans sprints."),
        ("viewer", "Viewer", "Reads everything and comments; changes only issues they reported or are assigned."),
    ],
    levels: [
        (AccessLevel::Open, "Open", "Everyone in the workspace can find this project and work on its issues."),
        (AccessLevel::Limited, "Limited", "Everyone can find and read it and comment; only the people below change issues."),
        (
            AccessLevel::Private,
            "Private",
            "Only the people below can see it. For everyone else it doesn't exist: not in the sidebar, search or boards.",
        ),
    ],
};

const SPACE: Kind = Kind {
    noun: "space",
    access_cmd: "access.setSpaceAccess",
    role_cmd: "access.setSpaceRole",
    roles: [
        ("admin", "Admin", "Everything an editor can, plus changing who has access, restricting any page and managing templates."),
        ("editor", "Editor", "Creates, edits, moves and deletes pages."),
        ("viewer", "Viewer", "Reads pages and comments on them."),
    ],
    levels: [
        (AccessLevel::Open, "Open", "Everyone in the workspace can read and edit pages here."),
        (AccessLevel::Limited, "Limited", "Everyone can read pages and comment; only the people below edit."),
        (AccessLevel::Private, "Private", "Only the people below can see this space or any of its pages."),
    ],
};

/// The heading and the tab bar (selectable buttons in a row).
fn header<T: Copy + PartialEq>(ui: &mut Ui, title: &str, tabs: &[T], current: T, label: fn(T) -> &'static str) -> Option<T> {
    ui.add(egui::Label::new(RichText::new(title).size(22.0).strong()).truncate());
    ui.add_space(8.0);
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
        for tb in tabs {
            if ui.add(egui::Button::selectable(*tb == current, label(*tb))).clicked() && *tb != current {
                pick = Some(*tb);
            }
        }
    });
    ui.separator();
    ui.add_space(8.0);
    pick
}

/// A project's settings.
pub(crate) fn project(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &ProjectKey, tab: ProjectTab) {
    let Ok(p) = app.session.ws.project(key.as_str()) else {
        ui.label("This project no longer exists.");
        return;
    };
    let title = format!("{} settings", p.name);
    if let Some(tb) = header(ui, &title, ProjectTab::ALL, tab, ProjectTab::label) {
        // Switching tabs is not a navigation: it leaves no Back step.
        app.view = View::ProjectSettings(key.clone(), tb);
    }
    // The fields editor scrolls on its own (no nested scroll areas).
    if tab == ProjectTab::Fields {
        crate::fields_admin::show(app, ui, t, key);
        return;
    }
    egui::ScrollArea::vertical().id_salt("project-settings").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(760.0);
        match tab {
            ProjectTab::Access => {
                let g = app.session.ws.access.projects.get(key).cloned().unwrap_or_default();
                let (level, roles) = flatten(&g, ProjectRole::name);
                access(app, ui, t, &PROJECT, key, level, roles);
            }
            ProjectTab::Fields => {}
        }
    });
}

/// A space's settings (one tab today, drawn the same way as a project's).
pub(crate) fn space(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &ProjectKey) {
    let Ok(s) = app.session.ws.space(key.as_str()) else {
        ui.label("This space no longer exists.");
        return;
    };
    let title = format!("{} settings", s.name);
    let _ = header(ui, &title, &[ProjectTab::Access], ProjectTab::Access, ProjectTab::label);
    egui::ScrollArea::vertical().id_salt("space-settings").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(760.0);
        let g = app.session.ws.access.spaces.get(key).cloned().unwrap_or_default();
        let (level, roles) = flatten(&g, SpaceRole::name);
        access(app, ui, t, &SPACE, key, level, roles);
    });
}

fn flatten<R: Copy>(g: &Grants<R>, name: fn(R) -> &'static str) -> (AccessLevel, Vec<(String, &'static str)>) {
    (g.access, g.roles.iter().map(|(u, r)| (u.clone(), name(*r))).collect())
}

fn note(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(text).color(t.muted).size(12.5));
}

/// Runs an access command now; its error shows inline under the form.
fn run(app: &mut TrackApp, id: &str, params: &Value, done: String) {
    match app.session.run(id, params) {
        Ok(_) => {
            app.jql_ran = None;
            app.access.error = None;
            app.notify_done(done, true);
        }
        Err(e) => app.access.error = Some(e),
    }
}

fn access(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, kind: &Kind, key: &ProjectKey, level: AccessLevel, roles: Vec<(String, &'static str)>) {
    let k = key.to_string();
    let target = format!("{}:{k}", kind.noun);
    if app.access.target != target {
        // New people start with the middle role (member / editor).
        app.access = AccessUi { target, add_role: 1, ..Default::default() };
    }
    let allowed = can(app, kind.access_cmd, &json!({(kind.noun): k, "access": "open"}));
    let why = denied(&format!("change who can use this {}", kind.noun));

    ui.label(RichText::new("Who can see it").strong().size(15.0));
    note(ui, t, &format!("For people without a role below. Workspace admins always see and manage every {}.", kind.noun));
    ui.add_space(4.0);
    let mut set_level = None;
    for (l, name, says) in kind.levels {
        ui.horizontal(|ui| {
            let r = ui.add_enabled(allowed, egui::RadioButton::new(level == l, name));
            let r = r.on_disabled_hover_text(&why);
            if r.clicked() && level != l {
                set_level = Some(l);
            }
            ui.add(egui::Label::new(RichText::new(says).color(t.muted).size(12.5)).wrap());
        });
    }
    if let Some(l) = set_level {
        run(app, kind.access_cmd, &json!({(kind.noun): k, "access": l.name()}), format!("{k} is now {}", l.name()));
    }

    ui.add_space(16.0);
    ui.label(RichText::new("People").strong().size(15.0));
    egui::Grid::new(("access-roles", kind.noun)).num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
        for (_, label, says) in kind.roles {
            ui.label(RichText::new(label).strong().size(12.5));
            note(ui, t, says);
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    let mut change: Option<(String, Value)> = None;
    if roles.is_empty() {
        note(ui, t, &format!("No one has a role in this {} yet.", kind.noun));
    }
    let ws = &app.session.ws;
    egui::Grid::new(("access-people", kind.noun)).num_columns(3).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
        for (user, role) in &roles {
            let name = name_of(ws, Some(user));
            let h = ui.spacing().interact_size.y;
            let l = ui
                .allocate_ui_with_layout(egui::vec2(220.0, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(220.0);
                    ui.add(egui::Label::new(&name).truncate())
                })
                .inner;
            let current = kind.roles.iter().find(|r| r.0 == *role).map_or(*role, |r| r.1);
            ui.add_enabled_ui(allowed, |ui| {
                ComboBox::from_id_salt(("access-role", kind.noun, user.as_str()))
                    .selected_text(current)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (id, label, _) in kind.roles {
                            if ui.selectable_label(id == *role, label).clicked() && id != *role {
                                change = Some((user.clone(), json!(id)));
                            }
                        }
                    })
                    .response
                    .labelled_by(l.id)
                    .on_disabled_hover_text(&why);
            });
            let rm = ui.add_enabled_ui(allowed, |ui| remove_button(ui, t, &format!("Remove {name}"), 18.0)).inner;
            if rm.clicked() {
                change = Some((user.clone(), Value::Null));
            }
            ui.end_row();
        }
    });
    if let Some((user, role)) = change {
        let done = match role.as_str() {
            Some(r) => format!("{} is now {r}", name_of(&app.session.ws, Some(&user))),
            None => format!("Removed {}'s role", name_of(&app.session.ws, Some(&user))),
        };
        run(app, kind.role_cmd, &json!({(kind.noun): k, "user": user, "role": role}), done);
    }

    ui.add_space(10.0);
    let mut add = false;
    ui.add_enabled_ui(allowed, |ui| {
        ui.horizontal(|ui| {
            let ws = &app.session.ws;
            let l = ui.label("Add person");
            let shown = app.access.add_user.as_deref().map_or_else(|| "Choose someone".to_string(), |u| name_of(ws, Some(u)));
            let picked = person_adder(ui, ws, "access-add-user", &shown, l.id, 170.0);
            if let Some(c) = picked.choice {
                app.access.add_user = c;
            }
            let l = ui.label("as");
            let role = kind.roles.get(app.access.add_role).map_or("", |r| r.1);
            ComboBox::from_id_salt(("access-add-role", kind.noun))
                .selected_text(role)
                .width(130.0)
                .show_ui(ui, |ui| {
                    for (i, (_, label, _)) in kind.roles.iter().enumerate() {
                        if ui.selectable_label(app.access.add_role == i, *label).clicked() {
                            app.access.add_role = i;
                        }
                    }
                })
                .response
                .labelled_by(l.id);
            add = ui.add_enabled(app.access.add_user.is_some(), egui::Button::new("Add")).clicked();
        });
    });
    if !allowed {
        note(ui, t, &format!("{why}. Ask an administrator of this {} or of the workspace.", kind.noun));
    }
    if add
        && let Some(user) = app.access.add_user.clone()
        && let Some((role, label, _)) = kind.roles.get(app.access.add_role)
    {
        let done = format!("Added {} as {}", name_of(&app.session.ws, Some(&user)), label.to_lowercase());
        run(app, kind.role_cmd, &json!({(kind.noun): k, "user": user, "role": role}), done);
        if app.access.error.is_none() {
            app.access.add_user = None;
        }
    }
    if let Some(e) = &app.access.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    ui.add_space(16.0);
    let back = if kind.noun == "project" { "Open the board" } else { "Open the space" };
    if ui.link(back).clicked() {
        let to = if kind.noun == "project" {
            crate::directory::board_view(app, key)
        } else {
            match app.session.ws.spaces.get(key) {
                Some(s) => View::Page(s.homepage),
                None => View::Spaces,
            }
        };
        app.push(Action::Nav(to));
    }
}
