//! The project board: status columns, drag and drop, keyboard selection. A sprint board
//! (`View::SprintBoard`) is the same board limited to one sprint's issues, under a sprint header.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::{Id, IssueKey, ProjectKey};
use trackcraft_model::{Issue, SprintState, StatusCategory, Timestamp, Workspace, priority_rank};

use crate::theme::Tokens;
use crate::widgets::{pill, who};
use crate::{Action, Then, TrackApp, View, icons};

/// The sprint the current view limits `key`'s board to, if any.
fn board_sprint(app: &TrackApp, key: &ProjectKey) -> Option<Id> {
    match &app.view {
        View::SprintBoard(p, id) if p == key => Some(*id),
        _ => None,
    }
}

/// The board's columns in workflow order: (status, category, card keys in rank order),
/// after the board's filters (and, on a sprint board, only that sprint's issues). Drawing and
/// keyboard navigation share it.
pub(crate) fn board_columns(app: &TrackApp, key: &ProjectKey) -> Vec<(String, StatusCategory, Vec<IssueKey>)> {
    let Ok(project) = app.session.ws.project(key.as_str()) else { return vec![] };
    let me = app.session.user.as_str();
    let needle = app.board_text.to_lowercase();
    let sprint = board_sprint(app, key);
    let ws = &app.session.ws;
    let cards: Vec<&Issue> = ws
        .rank_order(key)
        .iter()
        .filter_map(|k| ws.issues.get(k))
        .filter(|i| sprint.is_none_or(|s| i.sprint == Some(s)))
        .filter(|i| !app.board_mine || i.assignee.as_deref() == Some(me))
        .filter(|i| needle.is_empty() || i.summary.to_lowercase().contains(&needle) || i.key.to_string().to_lowercase().contains(&needle))
        .collect();
    project
        .workflow
        .statuses
        .iter()
        .map(|s| (s.name.clone(), s.category, cards.iter().filter(|i| i.status.eq_ignore_ascii_case(&s.name)).map(|i| i.key.clone()).collect()))
        .collect()
}

/// "Oct 27 – Nov 10" (either end may be missing).
pub(crate) fn date_range(start: Option<Timestamp>, end: Option<Timestamp>) -> String {
    let f = |t: Option<Timestamp>| t.map(|t| t.format("%b %-d").to_string());
    match (f(start), f(end)) {
        (Some(s), Some(e)) => format!("{s} – {e}"),
        (Some(s), None) => format!("from {s}"),
        (None, Some(e)) => format!("until {e}"),
        (None, None) => "No dates".into(),
    }
}

/// "9 days left", "Ends today", "2 days over"; `None` without an end date.
pub(crate) fn days_left(now: Timestamp, end: Option<Timestamp>) -> Option<String> {
    let days = (end? - now).num_days();
    Some(match days {
        d if d < 0 => format!("{} days over", d.saturating_neg()),
        0 => "Ends today".into(),
        1 => "1 day left".into(),
        d => format!("{d} days left"),
    })
}

/// A small upper-case state chip: ACTIVE (accent), PLANNED, CLOSED.
pub(crate) fn state_chip(ui: &mut Ui, t: &Tokens, state: SprintState) -> egui::Response {
    let done = t.status(StatusCategory::Done);
    let (text, fill, fg) = match state {
        SprintState::Active => ("ACTIVE", t.accent, t.on_accent),
        SprintState::Future => ("PLANNED", t.surface_alt, t.muted),
        SprintState::Closed => ("CLOSED", done.0, done.1),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, if state == SprintState::Future { t.border } else { fill }))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(10.5).strong().color(fg)))
        .response
}

/// The sprint board: a sprint header (name, dates, days left, goal), then the board.
pub(crate) fn sprint_board(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &ProjectKey, id: Id) {
    let Ok(sp) = app.session.ws.sprint(id).cloned() else {
        ui.label("This sprint no longer exists.");
        return;
    };
    let now = app.session.now();
    ui.horizontal(|ui| {
        ui.heading(RichText::new(&sp.name).size(22.0));
        state_chip(ui, t, sp.state);
        ui.label(RichText::new(date_range(sp.start, sp.end)).color(t.muted));
        let left = match sp.state {
            SprintState::Closed => sp.completed.map(|c| format!("Completed {}", c.format("%b %-d"))),
            _ => days_left(now, sp.end),
        };
        if let Some(l) = left {
            ui.label(RichText::new(l).strong());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if sp.state == SprintState::Active
                && ui.add(egui::Button::new(RichText::new("Complete sprint").color(t.on_accent).strong()).fill(t.accent)).clicked()
            {
                app.sprint_dialog = crate::sprints::SprintDialog::open(&app.session, "complete", Some(id), "").ok();
            }
            if ui.button("Backlog").on_hover_text("Plan this project's sprints").clicked() {
                app.push(Action::Nav(View::Backlog(key.clone())));
            }
            if ui.button("Whole project").on_hover_text("Show every issue in the project").clicked() {
                app.push(Action::Nav(View::Board(key.clone())));
            }
        });
    });
    if !sp.goal.is_empty() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Goal").color(t.muted).strong().size(12.0));
            ui.label(&sp.goal);
        });
    }
    board_body(app, ui, t, key);
}

pub(crate) fn board(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &ProjectKey) {
    let Ok(project) = app.session.ws.project(key.as_str()) else {
        ui.label("This project no longer exists.");
        return;
    };
    let pname = project.name.clone();
    let active = app.session.ws.active_sprint(key).map(|s| (s.id, s.name.clone()));
    ui.horizontal(|ui| {
        ui.heading(RichText::new(format!("{pname} board")).size(22.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+ Create issue").clicked() {
                app.push(Action::OpenCreate(Some(key.to_string())));
            }
            if let Some((id, name)) = active
                && ui.button(format!("{name} only")).on_hover_text("Show only the active sprint's issues").clicked()
            {
                app.push(Action::Nav(View::SprintBoard(key.clone(), id)));
            }
        });
    });
    board_body(app, ui, t, key);
}

/// Filters, then the columns.
fn board_body(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &ProjectKey) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.board_text).hint_text("Filter cards").desired_width(200.0));
        ui.toggle_value(&mut app.board_mine, "Only my issues");
        let hint = "Drag a card to another column, or select it with the arrow keys and press Shift+Right. Press ? for all shortcuts.";
        ui.label(RichText::new(hint).color(t.muted).size(12.0));
    });
    ui.add_space(8.0);

    let columns = board_columns(app, key);
    let scroll = std::mem::take(&mut app.scroll_sel);
    let selected = app.board_sel.clone();
    let ws = &app.session.ws;

    let mut moved: Option<(IssueKey, String)> = None;
    let mut open: Option<IssueKey> = None;
    let col_w = ((ui.available_width() - 12.0 * columns.len().saturating_sub(1) as f32) / columns.len().max(1) as f32).clamp(220.0, 320.0);
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (status, cat, col_keys) in &columns {
                let in_col: Vec<&Issue> = col_keys.iter().filter_map(|k| ws.issues.get(k)).collect();
                let frame = egui::Frame::new().fill(t.surface_alt).corner_radius(10).inner_margin(10);
                let (resp, dropped) = ui.dnd_drop_zone::<IssueKey, ()>(frame, |ui| {
                    // The row of columns is horizontal; each column stacks its cards vertically.
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        ui.set_min_height((ui.available_height() - 20.0).max(300.0));
                        ui.horizontal(|ui| {
                            pill(ui, t, *cat, status);
                            ui.label(RichText::new(in_col.len().to_string()).color(t.muted));
                        });
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().id_salt(("col", status)).auto_shrink([false, false]).show(ui, |ui| {
                            for i in in_col {
                                let id = egui::Id::unique(("card", i.key.to_string()));
                                let sel = selected.as_ref() == Some(&i.key);
                                let r = ui.dnd_drag_source(id, i.key.clone(), |ui| card_body(ui, t, ws, i, col_w, sel)).response;
                                if r.clicked() {
                                    open = Some(i.key.clone());
                                }
                                if sel && scroll {
                                    r.scroll_to_me(None);
                                }
                                ui.add_space(6.0);
                            }
                        });
                    });
                });
                if resp.response.dnd_hover_payload::<IssueKey>().is_some() {
                    ui.painter().rect_stroke(resp.response.rect, 10.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
                if let Some(k) = dropped {
                    moved = Some(((*k).clone(), status.clone()));
                }
            }
        });
    });
    if let Some((k, to)) = moved
        && app.session.ws.issues.get(&k).is_some_and(|i| !i.status.eq_ignore_ascii_case(&to))
    {
        app.push(Action::Run {
            id: "issue.transition",
            params: json!({"key": k.to_string(), "to": to}),
            then: Then::Toast(format!("{k} moved to {to}")),
        });
    }
    if let Some(k) = open {
        app.board_sel = Some(k.clone());
        app.push(Action::Nav(View::Issue(k)));
    }
}

fn card_body(ui: &mut Ui, t: &Tokens, ws: &Workspace, i: &Issue, w: f32, selected: bool) {
    let (fill, stroke) = if selected { (t.accent_soft, egui::Stroke::new(2.0, t.accent)) } else { (t.surface, egui::Stroke::new(1.0, t.border)) };
    egui::Frame::new().fill(fill).stroke(stroke).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(w - 20.0);
        ui.add(egui::Label::new(&i.summary).wrap());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.set_width(w - 20.0);
            icons::issue_type(ui, &i.issue_type, 15.0);
            ui.label(RichText::new(i.key.to_string()).monospace().size(12.0).color(t.muted));
            if let Some(p) = i.story_points {
                ui.label(RichText::new(format!("{p}")).size(11.0).background_color(t.surface_alt));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                who(ui, t, ws, i.assignee.as_deref(), 20.0);
                icons::priority(ui, t, &i.priority, priority_rank(&i.priority).unwrap_or(3), 13.0);
            });
        });
    });
}
