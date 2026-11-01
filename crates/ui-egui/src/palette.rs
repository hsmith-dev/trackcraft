//! The command palette (⌘K): jump to an issue, project board or page, or run any command.
//! Commands get the current context filled in (`key` on an issue, `id` on a page, `project` on
//! a board); a command that still lacks params reports what it needs.

use egui::RichText;
use serde_json::{Value, json};
use trackcraft_core::IssueKey;

use crate::theme::Tokens;
use crate::{Action, Then, TrackApp, View};

#[derive(Debug, Default)]
pub(crate) struct Palette {
    pub query: String,
    pub selected: usize,
}

enum Entry {
    Go(View, String),
    Cmd(&'static str, String),
    /// The shortcut overlay (`?`).
    Help(String),
}

fn entries(app: &TrackApp, q: &str) -> Vec<Entry> {
    let q = q.trim().to_lowercase();
    let hit = |s: &str| q.split_whitespace().all(|w| s.to_lowercase().contains(w));
    let mut out = Vec::new();
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
    for p in app.session.ws.projects.values() {
        let label = format!("Board: {}", p.name);
        if hit(&label) {
            out.push(Entry::Go(View::Board(p.key.clone()), label));
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

fn context_params(app: &TrackApp) -> Value {
    match &app.view {
        View::Issue(k) => json!({"key": k.to_string(), "project": k.project.to_string()}),
        View::Page(id) | View::PageHistory(id) => {
            let space = app.session.ws.pages.get(id).map(|p| p.space.to_string());
            json!({"id": id.0, "space": space})
        }
        View::Board(p) => json!({"project": p.to_string(), "key": p.to_string()}),
        _ => json!({}),
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
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (n, e) in list.iter().enumerate().take(60) {
                let (label, kind) = match e {
                    Entry::Go(_, l) => (l.as_str(), "go"),
                    Entry::Cmd(id, l) => (l.as_str(), *id),
                    Entry::Help(l) => (l.as_str(), "?"),
                };
                let sel = n == pal.selected;
                let r = ui.horizontal(|ui| {
                    let r = ui.selectable_label(sel, label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(kind).monospace().size(11.0).color(t.muted))
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
            Some(Entry::Go(v, _)) => app.push(Action::Nav(v)),
            Some(Entry::Help(_)) => app.push(Action::ShowHelp),
            Some(Entry::Cmd(id, label)) => {
                let then = if id == "issue.create" { Then::OpenIssue } else { Then::Toast(format!("Done: {label}")) };
                if id == "issue.create" {
                    app.push(Action::OpenCreate(None));
                } else {
                    app.push(Action::Run { id, params: context_params(app), then });
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
