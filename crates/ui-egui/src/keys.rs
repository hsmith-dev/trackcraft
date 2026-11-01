//! Keyboard-first navigation: the one shortcut table (the help overlay renders it; the handlers
//! below implement it), the per-view key handlers, and the `?` help overlay.
//!
//! Single-letter keys never fire while a text field has focus or a dialog is open; `logic`
//! checks that before calling [`view_keys`].

use egui::{InputState, Key, Modifiers, RichText};
use serde_json::json;
use trackcraft_core::IssueKey;

use crate::theme::Tokens;
use crate::{Action, Then, TrackApp, View, issues};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Area {
    Global,
    List,
    Board,
    Issue,
    Page,
}

impl Area {
    pub(crate) const ALL: [Area; 5] = [Area::Global, Area::List, Area::Board, Area::Issue, Area::Page];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Area::Global => "Everywhere",
            Area::List => "Issue list",
            Area::Board => "Board",
            Area::Issue => "Issue",
            Area::Page => "Page",
        }
    }
}

/// (area, keys as text — `Mod` becomes Cmd or Ctrl —, what it does).
pub(crate) const SHORTCUTS: &[(Area, &str, &str)] = &[
    (Area::Global, "Mod+K", "Command palette"),
    (Area::Global, "?", "Keyboard shortcuts"),
    (Area::Global, "C", "Create an issue"),
    (Area::Global, "/", "Search with JQL"),
    (Area::Global, "Alt+Left", "Back"),
    (Area::Global, "Mod+Z", "Undo"),
    (Area::Global, "Mod+Shift+Z", "Redo"),
    (Area::Global, "Mod+S", "Save"),
    (Area::Global, "Mod+Shift+S", "Save as"),
    (Area::Global, "Mod+O", "Open a workspace"),
    (Area::Global, "Escape", "Close a dialog, cancel an edit"),
    (Area::List, "J / Down", "Next issue"),
    (Area::List, "K / Up", "Previous issue"),
    (Area::List, "Enter", "Open the selected issue"),
    (Area::List, "Click a header", "Sort by that column"),
    (Area::Board, "H J K L / Arrows", "Move between cards"),
    (Area::Board, "Enter", "Open the selected card"),
    (Area::Board, "Shift+Right / ]", "Move the card to the next status"),
    (Area::Board, "Shift+Left / [", "Move the card to the previous status"),
    (Area::Issue, "A", "Assign to me"),
    (Area::Issue, "M", "Write a comment"),
    (Area::Issue, "E", "Edit the description"),
    (Area::Issue, "Escape", "Cancel editing"),
    (Area::Page, "E", "Edit the page"),
];

/// `?` on any layout: the logical key, or Shift+/ from backends that report the physical key.
pub(crate) fn consume_help(i: &mut InputState) -> bool {
    i.consume_key(Modifiers::NONE, Key::Questionmark) || i.consume_key(Modifiers::SHIFT, Key::Slash)
}

/// The current view's single-key shortcuts. Only called when nothing is typing and no dialog is open.
pub(crate) fn view_keys(app: &mut TrackApp, i: &mut InputState) {
    match app.view.clone() {
        View::Issues => list_keys(app, i),
        View::Board(p) => board_keys(app, i, &p),
        View::Issue(k) => issue_keys(app, i, &k),
        View::Page(id) => {
            if app.page_edit.is_none() && i.consume_key(Modifiers::NONE, Key::E) {
                app.push(Action::EditPage(id));
            }
        }
        View::Home | View::PageHistory(_) => {}
    }
}

fn list_keys(app: &mut TrackApp, i: &mut InputState) {
    let Ok(keys) = &app.jql_result else { return };
    let down = i.consume_key(Modifiers::NONE, Key::J) || i.consume_key(Modifiers::NONE, Key::ArrowDown);
    let up = i.consume_key(Modifiers::NONE, Key::K) || i.consume_key(Modifiers::NONE, Key::ArrowUp);
    let enter = i.consume_key(Modifiers::NONE, Key::Enter);
    let pos = app.list_sel.as_ref().and_then(|s| keys.iter().position(|k| k == s));
    let last = keys.len().saturating_sub(1);
    let next = match (pos, down, up) {
        (Some(p), true, false) => Some(p.saturating_add(1).min(last)),
        (Some(p), false, true) => Some(p.saturating_sub(1)),
        (None, true, false) => Some(0),
        (None, false, true) => Some(last),
        _ => pos,
    };
    let sel = next.and_then(|n| keys.get(n)).cloned();
    if (down || up) && sel.is_some() {
        app.list_sel = sel.clone();
        app.scroll_sel = true;
    }
    if enter && let Some(k) = sel {
        app.push(Action::Nav(View::Issue(k)));
    }
}

fn board_keys(app: &mut TrackApp, i: &mut InputState, project: &trackcraft_core::ProjectKey) {
    let cols = issues::board_columns(app, project);
    let pos = app.board_sel.as_ref().and_then(|s| cols.iter().enumerate().find_map(|(c, col)| col.2.iter().position(|k| k == s).map(|r| (c, r))));
    // Shifted arrows first: a plain-arrow pattern also matches Shift+arrow.
    let to_next = i.consume_key(Modifiers::SHIFT, Key::ArrowRight) || i.consume_key(Modifiers::NONE, Key::CloseBracket);
    let to_prev = i.consume_key(Modifiers::SHIFT, Key::ArrowLeft) || i.consume_key(Modifiers::NONE, Key::OpenBracket);
    if let Some((c, r)) = pos
        && to_next != to_prev
    {
        let target = if to_next { c.checked_add(1) } else { c.checked_sub(1) };
        if let Some((status, _, _)) = target.and_then(|t| cols.get(t))
            && let Some(k) = cols.get(c).and_then(|col| col.2.get(r))
        {
            app.push(Action::Run {
                id: "issue.transition",
                params: json!({"key": k.to_string(), "to": status}),
                then: Then::Toast(format!("{k} moved to {status}")),
            });
            app.scroll_sel = true;
        }
        return;
    }
    let left = i.consume_key(Modifiers::NONE, Key::ArrowLeft) || i.consume_key(Modifiers::NONE, Key::H);
    let right = i.consume_key(Modifiers::NONE, Key::ArrowRight) || i.consume_key(Modifiers::NONE, Key::L);
    let down = i.consume_key(Modifiers::NONE, Key::ArrowDown) || i.consume_key(Modifiers::NONE, Key::J);
    let up = i.consume_key(Modifiers::NONE, Key::ArrowUp) || i.consume_key(Modifiers::NONE, Key::K);
    let enter = i.consume_key(Modifiers::NONE, Key::Enter);
    let at = |c: usize, r: usize| -> Option<IssueKey> { cols.get(c).and_then(|col| col.2.get(r.min(col.2.len().saturating_sub(1)))).cloned() };
    let moved = match pos {
        None if left || right || up || down => cols.iter().position(|col| !col.2.is_empty()).and_then(|c| at(c, 0)),
        None => None,
        Some((c, r)) if left || right => {
            // The nearest column in that direction that has cards.
            let mut found = None;
            let mut n = c;
            while found.is_none() {
                n = match if left { n.checked_sub(1) } else { n.checked_add(1) } {
                    Some(n) if n < cols.len() => n,
                    _ => break,
                };
                if cols.get(n).is_some_and(|col| !col.2.is_empty()) {
                    found = at(n, r);
                }
            }
            found
        }
        Some((c, r)) if down => at(c, r.saturating_add(1)),
        Some((c, r)) if up => at(c, r.saturating_sub(1)),
        Some(_) => None,
    };
    if let Some(k) = moved {
        app.board_sel = Some(k);
        app.scroll_sel = true;
    }
    if enter && let Some(k) = pos.and_then(|(c, r)| at(c, r)) {
        app.push(Action::Nav(View::Issue(k)));
    }
}

fn issue_keys(app: &mut TrackApp, i: &mut InputState, key: &IssueKey) {
    if i.consume_key(Modifiers::NONE, Key::A) {
        app.push(Action::Run {
            id: "issue.assignToMe",
            params: json!({"key": key.to_string()}),
            then: Then::Toast(format!("Assigned {key} to you")),
        });
    }
    if i.consume_key(Modifiers::NONE, Key::M) {
        app.issue_ui.focus_comment = true;
    }
    if i.consume_key(Modifiers::NONE, Key::E)
        && let Some(issue) = app.session.ws.issues.get(key)
    {
        if app.issue_ui.editing_description.is_none() {
            app.issue_ui.editing_description = Some(issue.description.to_markdown());
        }
        app.issue_ui.focus_description = true;
    }
}

/// Escape outside dialogs on an issue: drop every unsaved edit.
pub(crate) fn cancel_issue_edits(app: &mut TrackApp) {
    let ui = &mut app.issue_ui;
    ui.editing_summary = None;
    ui.editing_description = None;
    ui.labels = None;
    ui.confirm_delete = false;
}

/// The `?` overlay, built from [`SHORTCUTS`].
pub(crate) fn help(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    if !app.help {
        return;
    }
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("shortcut-help")).show(ctx, |ui| {
        ui.set_width(680.0);
        ui.horizontal(|ui| {
            ui.heading("Keyboard shortcuts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close (Escape)").clicked() {
                    close = true;
                }
            });
        });
        ui.label(RichText::new("Single keys work when no text field has focus.").color(t.muted).size(12.0));
        ui.add_space(8.0);
        let (left, right): (Vec<Area>, Vec<Area>) = Area::ALL.into_iter().partition(|a| matches!(a, Area::Global | Area::List));
        ui.columns(2, |cols| {
            for (n, (col, areas)) in cols.iter_mut().zip([left, right]).enumerate() {
                // One grid per column, so descriptions line up across areas.
                egui::Grid::new(("help-col", n)).num_columns(2).spacing([14.0, 6.0]).show(col, |ui| {
                    for (a, area) in areas.into_iter().enumerate() {
                        if a > 0 {
                            // A blank row between areas (grids don't take `add_space`).
                            ui.label("");
                            ui.end_row();
                        }
                        ui.label(RichText::new(area.title().to_uppercase()).size(11.0).strong().color(t.muted));
                        ui.end_row();
                        for (_, keys, what) in SHORTCUTS.iter().filter(|s| s.0 == area) {
                            key_chip(ui, t, &crate::key(keys));
                            ui.label(*what);
                            ui.end_row();
                        }
                    }
                });
            }
        });
    });
    if close || modal.should_close() {
        app.help = false;
    }
}

fn key_chip(ui: &mut egui::Ui, t: &Tokens, keys: &str) {
    egui::Frame::new()
        .fill(t.surface_alt)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(keys).monospace().size(12.0).color(t.text)));
}
