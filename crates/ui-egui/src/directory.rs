//! Projects and Spaces directories (searchable tables), the sidebar's own state (filter text,
//! stars, collapsed sections) and the small Create project / Create space dialog (UI-8).
//!
//! Stars are a per-machine UI preference kept in egui's persisted memory, not in the
//! workspace: the plan's `user.favourite` command (per user, synced in M4) does not exist yet.
//! When it lands, [`SideUi::set_star`] becomes a command run and the memory copy goes away.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Color32, Rect, Response, RichText, Sense, Shape, Ui, vec2};
use serde_json::json;
use trackcraft_core::ProjectKey;

use crate::theme::Tokens;
use crate::{Action, TrackApp, View};

/// How many recent destinations the sidebar and palette remember.
pub(crate) const RECENT: usize = 8;

/// Where the stars live in egui's persisted memory.
fn stars_id() -> egui::Id {
    egui::Id::unique("trackcraft.ui8.stars")
}

/// Something that can be starred.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Star {
    Project(String),
    Space(String),
    Page(u64),
    /// A sidebar filter, by its label.
    Filter(String),
}

impl Star {
    /// `project:LUME`, `space:ENG`, `page:12`, `filter:All issues`.
    pub(crate) fn parse(s: &str) -> Option<Star> {
        let (kind, id) = s.split_once(':')?;
        let id = id.trim();
        if id.is_empty() || id.len() > 200 {
            return None;
        }
        Some(match kind.trim() {
            "project" => Star::Project(id.to_ascii_uppercase()),
            "space" => Star::Space(id.to_ascii_uppercase()),
            "page" => Star::Page(id.parse().ok()?),
            "filter" => Star::Filter(id.to_string()),
            _ => return None,
        })
    }

    fn encode(&self) -> String {
        match self {
            Star::Project(k) => format!("project:{k}"),
            Star::Space(k) => format!("space:{k}"),
            Star::Page(id) => format!("page:{id}"),
            Star::Filter(l) => format!("filter:{l}"),
        }
    }
}

/// Which create dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Project,
    Space,
}

#[derive(Debug)]
pub(crate) struct CreateDialog {
    pub kind: Kind,
    pub name: String,
    pub key: String,
    /// The key was typed by hand (stop suggesting one from the name).
    pub key_typed: bool,
    pub error: Option<String>,
}

impl CreateDialog {
    pub(crate) fn new(kind: Kind) -> Self {
        CreateDialog { kind, name: String::new(), key: String::new(), key_typed: false, error: None }
    }
}

/// Sidebar and directory state.
#[derive(Debug, Default)]
pub(crate) struct SideUi {
    /// The sidebar filter box.
    pub filter: String,
    stars: BTreeSet<Star>,
    /// Loaded from persisted memory yet.
    loaded: bool,
    /// Changed since the last write to persisted memory.
    dirty: bool,
    /// Collapsed sidebar sections, by title.
    pub collapsed: BTreeSet<&'static str>,
    /// Projects and spaces opened with their chevron (besides the current one).
    pub open_projects: BTreeSet<String>,
    pub open_spaces: BTreeSet<String>,
    /// The directory search box (shared by both directories; cleared when switching).
    pub query: String,
    query_for: Option<Kind>,
    pub dialog: Option<CreateDialog>,
}

impl SideUi {
    pub(crate) fn starred(&self, s: &Star) -> bool {
        self.stars.contains(s)
    }

    pub(crate) fn stars(&self) -> impl Iterator<Item = &Star> {
        self.stars.iter()
    }

    pub(crate) fn set_star(&mut self, s: Star, on: bool) {
        let changed = if on { self.stars.insert(s) } else { self.stars.remove(&s) };
        self.dirty |= changed;
    }

    pub(crate) fn toggle(&mut self, s: Star) {
        let on = !self.starred(&s);
        self.set_star(s, on);
    }

    /// Loads the stars from persisted memory once, and writes them back after a change.
    pub(crate) fn sync(&mut self, ctx: &egui::Context) {
        if !self.loaded {
            self.loaded = true;
            let saved: Vec<String> = ctx.data_mut(|d| d.get_persisted(stars_id())).unwrap_or_default();
            let before = self.stars.len();
            self.stars.extend(saved.iter().filter_map(|s| Star::parse(s)));
            self.dirty |= self.stars.len() != saved.len() || before > 0;
        }
        if std::mem::take(&mut self.dirty) {
            let list: Vec<String> = self.stars.iter().map(Star::encode).collect();
            ctx.data_mut(|d| d.insert_persisted(stars_id(), list));
        }
    }
}

/// The stars saved in `ctx`'s persisted memory.
#[cfg(test)]
pub(crate) fn saved_stars(ctx: &egui::Context) -> Vec<String> {
    ctx.data_mut(|d| d.get_persisted(stars_id())).unwrap_or_default()
}

/// Words of `q` (lower-cased) all appear in one of `fields`. An empty query matches everything.
pub(crate) fn hit_all(q: &str, fields: &[&str]) -> bool {
    let q = q.trim().to_lowercase();
    q.split_whitespace().all(|w| fields.iter().any(|f| f.to_lowercase().contains(w)))
}

/// A five-pointed star, filled when `on`. Clicking toggles; the accessible name says what.
pub(crate) fn star_toggle(ui: &mut Ui, t: &Tokens, on: bool, name: &str, size: f32) -> Response {
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let label = if on { format!("Unstar {name}") } else { format!("Star {name}") };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    let col = if on {
        t.accent
    } else if resp.hovered() {
        t.text
    } else {
        t.muted
    };
    paint_star(ui.painter(), r, col, on);
    resp.on_hover_text(if on { "Remove from Starred" } else { "Add to Starred" })
}

fn paint_star(p: &egui::Painter, r: Rect, col: Color32, filled: bool) {
    let c = r.center();
    let outer = r.width() * 0.46;
    let inner = outer * 0.42;
    let pt = |i: usize, rad: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        c + vec2(a.cos(), a.sin()) * rad
    };
    let pts: Vec<egui::Pos2> = (0..10).map(|i| pt(i, if i % 2 == 0 { outer } else { inner })).collect();
    if filled {
        // A concave star: the inner pentagon plus one triangle per point, all convex.
        let pent: Vec<egui::Pos2> = (0..5).map(|i| pt(i * 2 + 1, inner)).collect();
        p.add(Shape::convex_polygon(pent, col, egui::Stroke::NONE));
        for i in 0..5 {
            let tip = pt(i * 2, outer);
            let a = pt((i * 2 + 9) % 10, inner);
            let b = pt(i * 2 + 1, inner);
            p.add(Shape::convex_polygon(vec![a, tip, b], col, egui::Stroke::NONE));
        }
    } else {
        p.add(Shape::closed_line(pts, egui::Stroke::new(1.2, col)));
    }
}

/// A short name for a destination (sidebar Recent, palette): `None` when it no longer exists.
pub(crate) fn view_label(app: &TrackApp, v: &View) -> Option<String> {
    let ws = &app.session.ws;
    Some(match v {
        View::Issue(k) => format!("{k} {}", ws.issues.get(k)?.summary),
        View::Page(id) | View::PageHistory(id) => ws.pages.get(id)?.title.clone(),
        View::Board(p) | View::SprintBoard(p, _) => format!("{} board", ws.projects.get(p)?.name),
        View::Backlog(p) => format!("{} backlog", ws.projects.get(p)?.name),
        View::Home => "Home".into(),
        View::Issues => "Issues".into(),
        View::Projects => "Projects".into(),
        View::Spaces => "Spaces".into(),
    })
}

/// The board for a project: its active sprint's when one is running (as the sidebar does).
pub(crate) fn board_view(app: &TrackApp, k: &ProjectKey) -> View {
    match app.session.ws.active_sprint(k) {
        Some(sp) => View::SprintBoard(k.clone(), sp.id),
        None => View::Board(k.clone()),
    }
}

/// One row of the Projects directory.
pub(crate) struct ProjectRow {
    pub key: ProjectKey,
    pub name: String,
    pub lead: String,
    pub issues: usize,
}

/// One row of the Spaces directory.
pub(crate) struct SpaceRow {
    pub key: ProjectKey,
    pub name: String,
    pub owner: String,
    pub pages: usize,
    pub home: trackcraft_core::Id,
}

fn display(app: &TrackApp, id: Option<&str>) -> String {
    match id {
        Some(id) => app.session.ws.users.get(id).map_or_else(|| id.to_string(), |u| u.display_name.clone()),
        None => String::new(),
    }
}

/// The projects matching `q` (name, key or lead), by name.
pub(crate) fn project_rows(app: &TrackApp, q: &str) -> Vec<ProjectRow> {
    let ws = &app.session.ws;
    let mut counts: BTreeMap<&ProjectKey, usize> = BTreeMap::new();
    for i in ws.issues.values() {
        *counts.entry(&i.key.project).or_default() += 1;
    }
    let mut rows: Vec<ProjectRow> = ws
        .projects
        .values()
        .map(|p| ProjectRow {
            key: p.key.clone(),
            name: p.name.clone(),
            lead: display(app, p.lead.as_deref()),
            issues: counts.get(&p.key).copied().unwrap_or(0),
        })
        .filter(|r| hit_all(q, &[&r.name, r.key.as_str(), &r.lead]))
        .collect();
    rows.sort_by_key(|r| r.name.to_lowercase());
    rows
}

/// The spaces matching `q` (name, key or owner), by name. The owner is whoever wrote the home page.
pub(crate) fn space_rows(app: &TrackApp, q: &str) -> Vec<SpaceRow> {
    let ws = &app.session.ws;
    let mut counts: BTreeMap<&ProjectKey, usize> = BTreeMap::new();
    for p in ws.pages.values() {
        *counts.entry(&p.space).or_default() += 1;
    }
    let mut rows: Vec<SpaceRow> = ws
        .spaces
        .values()
        .map(|s| SpaceRow {
            key: s.key.clone(),
            name: s.name.clone(),
            owner: display(app, ws.pages.get(&s.homepage).map(|p| p.author.as_str())),
            pages: counts.get(&s.key).copied().unwrap_or(0),
            home: s.homepage,
        })
        .filter(|r| hit_all(q, &[&r.name, r.key.as_str(), &r.owner]))
        .collect();
    rows.sort_by_key(|r| r.name.to_lowercase());
    rows
}

/// Heading, search box and create button shared by both directories.
fn header(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, kind: Kind, total: usize, shown: usize) {
    if app.side.query_for != Some(kind) {
        app.side.query_for = Some(kind);
        app.side.query.clear();
    }
    let (title, what, create) = match kind {
        Kind::Project => ("Projects", "projects", "Create project"),
        Kind::Space => ("Spaces", "spaces", "Create space"),
    };
    ui.horizontal(|ui| {
        ui.heading(RichText::new(title).size(22.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let b = egui::Button::new(RichText::new(format!("+ {create}")).color(t.on_accent).strong()).fill(t.accent);
            if ui.add(b).clicked() {
                app.side.dialog = Some(CreateDialog::new(kind));
            }
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let label = ui.label(RichText::new("Search").color(t.muted));
        let hint = format!("Search {what} by name, key or {}", if kind == Kind::Project { "lead" } else { "owner" });
        ui.add(egui::TextEdit::singleline(&mut app.side.query).hint_text(hint).desired_width(360.0)).labelled_by(label.id);
        let count = if shown == total { format!("{total} {what}") } else { format!("{shown} of {total} {what}") };
        ui.label(RichText::new(count).color(t.muted).size(12.0));
    });
    ui.add_space(10.0);
}

fn col_head(ui: &mut Ui, t: &Tokens, s: &str) {
    ui.label(RichText::new(s).size(12.0).color(t.muted).strong());
}

/// `View::Projects`: every project, searchable; a name opens its board.
pub(crate) fn projects(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let total = app.session.ws.projects.len();
    let q = app.side.query.clone();
    let rows = project_rows(app, if app.side.query_for == Some(Kind::Project) { &q } else { "" });
    header(app, ui, t, Kind::Project, total, rows.len());
    if rows.is_empty() {
        let (title, hint) = if total == 0 {
            ("No projects yet", "Create one with the button above.")
        } else {
            ("No project matches the search", "Try fewer or different words.")
        };
        crate::widgets::empty_state(ui, t, title, hint, None);
        return;
    }
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("projects-dir").striped(true).num_columns(5).spacing(vec2(24.0, 8.0)).min_col_width(16.0).show(ui, |ui| {
            col_head(ui, t, "");
            col_head(ui, t, "Name");
            col_head(ui, t, "Key");
            col_head(ui, t, "Lead");
            col_head(ui, t, "Issues");
            ui.end_row();
            for r in &rows {
                let star = Star::Project(r.key.to_string());
                if star_toggle(ui, t, app.side.starred(&star), &r.name, 16.0).clicked() {
                    app.side.toggle(star);
                }
                if ui.link(RichText::new(&r.name).strong()).clicked() {
                    actions.push(Action::Nav(board_view(app, &r.key)));
                }
                ui.label(RichText::new(r.key.as_str()).monospace().color(t.muted));
                ui.label(if r.lead.is_empty() { RichText::new("None").color(t.muted) } else { RichText::new(&r.lead) });
                ui.label(r.issues.to_string());
                ui.end_row();
            }
        });
    });
    for a in actions {
        app.push(a);
    }
}

/// `View::Spaces`: every space, searchable; a name opens its home page.
pub(crate) fn spaces(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let total = app.session.ws.spaces.len();
    let q = app.side.query.clone();
    let rows = space_rows(app, if app.side.query_for == Some(Kind::Space) { &q } else { "" });
    header(app, ui, t, Kind::Space, total, rows.len());
    if rows.is_empty() {
        let (title, hint) = if total == 0 {
            ("No spaces yet", "Create one with the button above.")
        } else {
            ("No space matches the search", "Try fewer or different words.")
        };
        crate::widgets::empty_state(ui, t, title, hint, None);
        return;
    }
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("spaces-dir").striped(true).num_columns(5).spacing(vec2(24.0, 8.0)).min_col_width(16.0).show(ui, |ui| {
            col_head(ui, t, "");
            col_head(ui, t, "Name");
            col_head(ui, t, "Key");
            col_head(ui, t, "Owner");
            col_head(ui, t, "Pages");
            ui.end_row();
            for r in &rows {
                let star = Star::Space(r.key.to_string());
                if star_toggle(ui, t, app.side.starred(&star), &r.name, 16.0).clicked() {
                    app.side.toggle(star);
                }
                if ui.link(RichText::new(&r.name).strong()).clicked() {
                    actions.push(Action::Nav(View::Page(r.home)));
                }
                ui.label(RichText::new(r.key.as_str()).monospace().color(t.muted));
                ui.label(if r.owner.is_empty() { RichText::new("None").color(t.muted) } else { RichText::new(&r.owner) });
                ui.label(r.pages.to_string());
                ui.end_row();
            }
        });
    });
    for a in actions {
        app.push(a);
    }
}

/// A key suggested from a name: its first letters and digits, upper-cased, starting with a letter.
fn suggest_key(name: &str) -> String {
    let mut out: String =
        name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).skip_while(char::is_ascii_digit).take(6).collect();
    if out.len() > 4 {
        // Prefer initials for several words ("Mobile App Team" → MAT).
        let initials: String =
            name.split_whitespace().filter_map(|w| w.chars().next()).filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_uppercase()).collect();
        if initials.len() >= 2 {
            out = initials.chars().take(6).collect();
        }
    }
    out
}

/// The Create project / Create space dialog: runs `project.create` or `space.create`.
pub(crate) fn dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut d) = app.side.dialog.take() else { return };
    let (title, what, cmd) = match d.kind {
        Kind::Project => ("Create project", "project", "project.create"),
        Kind::Space => ("Create space", "space", "space.create"),
    };
    let mut submit = false;
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("ui8-create-dialog")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.heading(title);
        ui.add_space(8.0);
        let name_label = ui.label("Name");
        let r = ui
            .add(egui::TextEdit::singleline(&mut d.name).hint_text(format!("The {what}'s name")).desired_width(f32::INFINITY))
            .labelled_by(name_label.id);
        if d.name.is_empty() && ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.changed() && !d.key_typed {
            d.key = suggest_key(&d.name);
        }
        submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(4.0);
        let key_label = ui.label("Key");
        let k = ui.add(egui::TextEdit::singleline(&mut d.key).hint_text("Capital letters, e.g. OPS").desired_width(160.0)).labelled_by(key_label.id);
        if k.changed() {
            d.key_typed = true;
            d.key = d.key.to_ascii_uppercase();
        }
        submit |= k.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some(e) = &d.error {
            ui.add_space(4.0);
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let ok = !d.name.trim().is_empty() && !d.key.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(RichText::new(title).color(t.on_accent).strong()).fill(t.accent)).clicked() {
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
    if submit && !close {
        let key = d.key.trim().to_ascii_uppercase();
        let name = d.name.trim().to_string();
        let mut params = json!({"key": key, "name": name});
        if d.kind == Kind::Project {
            params["lead"] = json!(app.session.user);
        }
        match app.session.run(cmd, &params) {
            Ok(_) => {
                app.jql_ran = None;
                app.notify(format!("Created {what} {name}"));
                let to = match d.kind {
                    Kind::Project => ProjectKey::parse(&key).ok().map(View::Board),
                    Kind::Space => app.session.ws.space(&key).ok().map(|s| View::Page(s.homepage)),
                };
                if let Some(v) = to {
                    app.push(Action::Nav(v));
                }
                return;
            }
            Err(e) => d.error = Some(e),
        }
    }
    if !close {
        app.side.dialog = Some(d);
    }
}
