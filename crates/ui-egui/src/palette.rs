//! The command palette (`Mod+K`): jump to an issue, project board or page, or run any command.
//!
//! With an empty query it lists recent destinations first. A command that has a UI form opens
//! it ([`ui_route`]: Create Issue → the create dialog, Create Page → the new-page dialog, Create
//! Project / Space → their dialog, Search → the JQL box…). Commands that take no params run
//! directly; the rest run with the current context (`key` on an issue, `id` on a page, `project`
//! on a board), and when that is not enough the palette stays open and shows the error and the
//! params the command takes (UI-8).

use egui::RichText;
use serde_json::{Value, json};
use trackcraft_core::{IssueKey, ProjectKey};

use crate::directory::{self, CreateDialog, Kind};
use crate::theme::Tokens;
use crate::{Action, FileRequest, TrackApp, View, keys, shell};

#[derive(Debug, Default)]
pub(crate) struct Palette {
    pub query: String,
    pub selected: usize,
    /// A command the context could not fill: (label, error, params doc).
    pub note: Option<(String, String, &'static str)>,
}

enum Entry {
    Go(View, String),
    /// A recently visited destination (empty query only).
    Recent(View, String),
    Cmd(&'static str, String),
    /// The shortcut overlay (`?`).
    Help(String),
}

fn entries(app: &TrackApp, q: &str) -> Vec<Entry> {
    let q = q.trim().to_lowercase();
    let hit = |s: &str| q.split_whitespace().all(|w| s.to_lowercase().contains(w));
    let mut out = Vec::new();
    if q.is_empty() {
        for v in app.recent.iter().filter(|v| **v != app.view) {
            if let Some(l) = directory::view_label(app, v) {
                out.push(Entry::Recent(v.clone(), l));
            }
        }
    }
    if let Ok(k) = IssueKey::parse(&q)
        && let Some(i) = app.session.ws.issues.get(&k)
    {
        out.push(Entry::Go(View::Issue(k.clone()), format!("Open {k}: {}", i.summary)));
    }
    if !q.is_empty() {
        for i in app.session.ws.issues.values().filter(|i| hit(&i.summary)).take(6) {
            out.push(Entry::Go(View::Issue(i.key.clone()), format!("{}  {}", i.key, i.summary)));
        }
        for p in app.session.ws.pages.values().filter(|p| hit(&p.title)).take(6) {
            out.push(Entry::Go(View::Page(p.id), format!("Page: {}", p.title)));
        }
    }
    for p in app.session.ws.projects.values().take(if q.is_empty() { 8 } else { usize::MAX }) {
        let label = format!("Board: {}", p.name);
        if hit(&label) {
            out.push(Entry::Go(View::Board(p.key.clone()), label));
        }
    }
    // M2.4: each project's fields editor and releases.
    for p in app.session.ws.projects.values().take(if q.is_empty() { 0 } else { usize::MAX }) {
        for (label, v) in
            [(format!("Fields: {}", p.name), View::ProjectFields(p.key.clone())), (format!("Releases: {}", p.name), View::Releases(p.key.clone()))]
        {
            if hit(&label) {
                out.push(Entry::Go(v, label));
            }
        }
    }
    for (label, v) in [("All projects", View::Projects), ("All spaces", View::Spaces), ("Settings", View::Settings(crate::settings::Tab::Appearance))]
    {
        if hit(label) {
            out.push(Entry::Go(v, label.into()));
        }
    }
    let help = "Keyboard shortcuts";
    if hit(help) || hit("help keys") {
        out.push(Entry::Help(help.into()));
    }
    for c in trackcraft_engine::commands() {
        let label = format!("{} — {}", c.label, c.location);
        if hit(&label) || hit(c.id) {
            out.push(Entry::Cmd(c.id, label));
        }
    }
    out
}

/// The current view as command params (also the fallback for shortcuts in `keys.rs`).
pub(crate) fn context_params(app: &TrackApp) -> Value {
    match &app.view {
        View::Issue(k) => json!({"key": k.to_string(), "project": k.project.to_string()}),
        View::Page(id) | View::PageHistory(id) => {
            let space = app.session.ws.pages.get(id).map(|p| p.space.to_string());
            json!({"id": id.0, "space": space})
        }
        View::Board(p) | View::Backlog(p) | View::SprintBoard(p, _) => json!({"project": p.to_string(), "key": p.to_string()}),
        View::ProjectFields(p) | View::Releases(p) => json!({"project": p.to_string()}), // M2.4
        _ => json!({}),
    }
}

/// Opens the UI form for a command whose params the context cannot fill. `false`: no route,
/// run the command instead.
pub(crate) fn ui_route(app: &mut TrackApp, id: &str) -> bool {
    let project = shell::current_project(app).and_then(|p| ProjectKey::parse(&p).ok());
    match (id, app.view.clone()) {
        ("issue.create", _) => app.push(Action::OpenCreate(project.map(|p| p.to_string()))),
        ("page.create", view) => {
            let ws = &app.session.ws;
            let parent = match view {
                View::Page(p) | View::PageHistory(p) => ws.pages.get(&p),
                _ => None,
            };
            let space = parent.map(|p| p.space.to_string()).or_else(|| ws.spaces.keys().next().map(ToString::to_string));
            match space {
                Some(space) => {
                    let parent = parent.map(|p| p.id);
                    app.push(Action::OpenNewPage { space, parent });
                }
                // No space to put a page in: create one first.
                None => app.side.dialog = Some(CreateDialog::new(Kind::Space)),
            }
        }
        ("project.create", _) => app.side.dialog = Some(CreateDialog::new(Kind::Project)),
        ("space.create", _) => app.side.dialog = Some(CreateDialog::new(Kind::Space)),
        ("search.jql", _) => app.focus_search = true,
        ("project.list", _) => app.push(Action::Nav(View::Projects)),
        ("space.list", _) => app.push(Action::Nav(View::Spaces)),
        ("board.view", _) => app.push(Action::Nav(match &project {
            Some(p) => directory::board_view(app, p),
            None => View::Projects,
        })),
        ("board.backlog", _) => app.push(Action::Nav(project.map_or(View::Projects, View::Backlog))),
        ("app.commands", _) => {}
        ("file.open", _) => app.push(Action::File(FileRequest::Open)),
        ("file.save", _) => app.push(Action::File(FileRequest::Save)),
        ("workspace.new", _) => app.push(Action::File(FileRequest::New)),
        ("workspace.import", _) => app.import.choose = true, // fixes
        ("sprint.create" | "sprint.start" | "sprint.complete", _) if project.is_some() => {
            let kind = id.trim_start_matches("sprint.");
            let p = project.map(|p| p.to_string()).unwrap_or_default();
            match crate::sprints::SprintDialog::open(&app.session, kind, None, &p) {
                Ok(d) => app.sprint_dialog = Some(d),
                Err(e) => app.notify(e),
            }
        }
        ("issue.assign", View::Issue(_)) => app.issue_ui.open_assignee = true,
        ("issue.comment", View::Issue(_)) => app.issue_ui.focus_comment = true,
        ("issue.transition", View::Issue(_)) => app.issue_ui.open_status = true,
        ("issue.edit", View::Issue(k)) => {
            if let Some(i) = app.session.ws.issues.get(&k)
                && app.issue_ui.editing_description.is_none()
            {
                app.issue_ui.editing_description = Some(i.description.to_markdown());
            }
            app.issue_ui.focus_description = true;
        }
        ("page.edit", View::Page(pid)) => app.push(Action::EditPage(pid)),
        // M2.4: the definitions' forms live in the fields editor, releases in their view.
        (c, _)
            if project.is_some()
                && (c.starts_with("field.") || c.starts_with("component.") || matches!(c, "version.create" | "version.edit" | "version.delete")) =>
        {
            app.push(Action::Nav(project.map_or(View::Projects, View::ProjectFields)))
        }
        ("version.list" | "version.release", _) if project.is_some() => app.push(Action::Nav(project.map_or(View::Projects, View::Releases))),
        ("page.labels", View::Page(pid)) => {
            if let Some(p) = app.session.ws.pages.get(&pid) {
                app.page_labels = Some((pid, p.labels.join(" ")));
            }
        }
        _ => return false,
    }
    true
}

/// Runs a command from the palette with the view's context. `Err` keeps the palette open with
/// the error and the params the command takes.
fn run_command(app: &mut TrackApp, id: &'static str, label: &str) -> Result<(), (String, String, &'static str)> {
    let doc = trackcraft_engine::command(id).map_or("{}", |c| c.params);
    match app.session.run(id, &context_params(app)) {
        Ok(_) => {
            app.jql_ran = None;
            app.fix_view();
            app.notify(format!("Done: {label}"));
            Ok(())
        }
        Err(e) => Err((label.to_string(), e, doc)),
    }
}

pub(crate) fn show(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut pal) = app.palette.take() else { return };
    let list = entries(app, &pal.query);
    pal.selected = pal.selected.min(list.len().saturating_sub(1));
    let mut chosen: Option<usize> = None;
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("palette")).show(ctx, |ui| {
        ui.set_width(560.0);
        let r =
            ui.add(egui::TextEdit::singleline(&mut pal.query).hint_text("Search issues, pages, boards and commands").desired_width(f32::INFINITY));
        r.request_focus();
        if r.changed() {
            pal.selected = 0;
            pal.note = None;
        }
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowDown) {
                pal.selected = pal.selected.saturating_add(1).min(list.len().saturating_sub(1));
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                pal.selected = pal.selected.saturating_sub(1);
            }
            if i.key_pressed(egui::Key::Enter) && !list.is_empty() {
                chosen = Some(pal.selected);
            }
        });
        if let Some((label, err, doc)) = &pal.note {
            ui.add_space(6.0);
            egui::Frame::new().fill(t.danger_soft).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("{label}: {err}")).color(t.danger));
                ui.label(RichText::new(format!("Params: {doc}")).monospace().size(11.0).color(t.text));
            });
        }
        ui.add_space(6.0);
        let mut recent_head = false;
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (n, e) in list.iter().enumerate().take(60) {
                let (label, kind, shortcut) = match e {
                    Entry::Go(_, l) => (l.as_str(), "go", None),
                    Entry::Recent(_, l) => (l.as_str(), "recent", None),
                    Entry::Cmd(id, l) => (l.as_str(), *id, keys::shortcut_for(app, keys::Act::Cmd(id))),
                    Entry::Help(l) => (l.as_str(), "", keys::shortcut_for(app, keys::Act::Help)),
                };
                if matches!(e, Entry::Recent(..)) && !std::mem::replace(&mut recent_head, true) {
                    ui.label(RichText::new("RECENT").size(11.0).color(t.muted).strong());
                }
                let sel = n == pal.selected;
                let r = ui.horizontal(|ui| {
                    let r = ui.selectable_label(sel, label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // The shortcut in the current view, as text, right-aligned.
                        if let Some(s) = &shortcut {
                            keys::key_chip(ui, t, s);
                        }
                        ui.label(RichText::new(kind).monospace().size(11.0).color(t.muted));
                    });
                    r
                });
                if sel {
                    r.inner.scroll_to_me(None);
                }
                if r.inner.clicked() {
                    chosen = Some(n);
                }
            }
        });
    });
    if modal.should_close() {
        close = true;
    }
    if let Some(n) = chosen {
        match list.into_iter().nth(n) {
            Some(Entry::Go(v, _) | Entry::Recent(v, _)) => app.push(Action::Nav(v)),
            Some(Entry::Help(_)) => app.push(Action::ShowHelp),
            Some(Entry::Cmd(id, label)) => {
                if let Some(why) = trackcraft_engine::command(id).and_then(|c| (c.enabled)(&app.session)) {
                    pal.note = Some((label, why.to_string(), trackcraft_engine::command(id).map_or("{}", |c| c.params)));
                    app.palette = Some(pal);
                } else if !ui_route(app, id)
                    && let Err(note) = run_command(app, id, &label)
                {
                    pal.note = Some(note);
                    app.palette = Some(pal);
                }
            }
            None => {}
        }
        return;
    }
    if !close {
        app.palette = Some(pal);
    }
}
