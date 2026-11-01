//! The project board: status columns, drag and drop, keyboard selection. A sprint board
//! (`View::SprintBoard`) is the same board limited to one sprint's issues, under a sprint header.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::{Id, IssueKey, ProjectKey};
use trackcraft_model::{Issue, SprintState, StatusCategory, Timestamp, Workspace, priority_rank};

use crate::theme::Tokens;
use crate::widgets::{empty_state, pill, who};
use crate::{Action, Then, TrackApp, View, icons};

/// The sprint the current view limits `key`'s board to, if any.
fn board_sprint(app: &TrackApp, key: &ProjectKey) -> Option<Id> {
    match &app.view {
        View::SprintBoard(p, id) if p == key => Some(*id),
        _ => None,
    }
}

/// The trailing column for issues whose status isn't in the project's workflow (UI-6: a
/// migrated or hand-edited file). Cards can't be dropped into it.
pub(crate) const OTHER_STATUSES: &str = "Other statuses";

/// Cards a column draws before "Show more" (UI-6: a 10 000-issue column stays responsive).
const CARD_PAGE: usize = 100;

/// The board's columns in workflow order: (status, category, card keys in rank order),
/// after the board's filters (and, on a sprint board, only that sprint's issues), then an
/// [`OTHER_STATUSES`] column when any card's status isn't in the workflow. Drawing and keyboard
/// navigation share it.
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
    let statuses = &project.workflow.statuses;
    let mut cols: Vec<(String, StatusCategory, Vec<IssueKey>)> = statuses
        .iter()
        .map(|s| (s.name.clone(), s.category, cards.iter().filter(|i| i.status.eq_ignore_ascii_case(&s.name)).map(|i| i.key.clone()).collect()))
        .collect();
    // UI-6
    let other: Vec<IssueKey> =
        cards.iter().filter(|i| !statuses.iter().any(|s| i.status.eq_ignore_ascii_case(&s.name))).map(|i| i.key.clone()).collect();
    if !other.is_empty() {
        cols.push((OTHER_STATUSES.to_string(), StatusCategory::ToDo, other));
    }
    cols
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
            ui.add(egui::Label::new(&sp.goal).truncate());
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
    // Buttons first (right to left), so a 255-character project name truncates instead of
    // pushing them off screen (UI-6).
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // UI-37: off for people who can't create issues here.
            let allowed = crate::widgets::can(app, "issue.create", &json!({"project": key.to_string()}));
            let create = ui.add_enabled(allowed, egui::Button::new("+ Create issue"));
            let create = if allowed { create } else { create.on_disabled_hover_text(crate::widgets::denied("create issues in this project")) };
            if create.clicked() {
                app.push(Action::OpenCreate(Some(key.to_string())));
            }
            if let Some((id, name)) = active
                && ui.add(egui::Button::new(format!("{name} only")).truncate()).on_hover_text("Show only the active sprint's issues").clicked()
            {
                app.push(Action::Nav(View::SprintBoard(key.clone(), id)));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(format!("{pname} board")).size(22.0).strong()).truncate());
            });
        })
    });
    board_body(app, ui, t, key);
}

/// Filters, then the columns (or an empty state when there is nothing to put in them).
fn board_body(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &ProjectKey) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.board_text).hint_text("Filter cards").desired_width(200.0));
        ui.toggle_value(&mut app.board_mine, "Only my issues");
        let hint = "Drag a card to another column, or select it with the arrow keys and press Shift+Right. Press ? for all shortcuts.";
        ui.add(egui::Label::new(RichText::new(hint).color(t.muted).size(12.0)).truncate());
    });
    ui.add_space(8.0);

    let columns = board_columns(app, key);
    // UI-6: say why the board is empty instead of drawing bare columns.
    if columns.iter().all(|c| c.2.is_empty()) {
        let sprint = board_sprint(app, key);
        let ws = &app.session.ws;
        let any = ws.issues.values().any(|i| &i.key.project == key && sprint.is_none_or(|s| i.sprint == Some(s)));
        let filtered = !app.board_text.is_empty() || app.board_mine;
        if any && filtered {
            if empty_state(ui, t, "No cards match these filters", "Clear the filter text or turn off Only my issues.", Some("Clear filters")) {
                app.board_text.clear();
                app.board_mine = false;
            }
        } else if sprint.is_some() {
            if empty_state(ui, t, "This sprint has no issues", "Plan issues into it from the backlog.", Some("Open backlog")) {
                app.push(Action::Nav(View::Backlog(key.clone())));
            }
        } else if empty_state(ui, t, "This project has no issues yet", "Create the first one and it appears here as a card.", Some("Create issue")) {
            app.push(Action::OpenCreate(Some(key.to_string())));
        }
        return;
    }
    let scroll = std::mem::take(&mut app.scroll_sel);
    let selected = app.board_sel.clone();
    let ws = &app.session.ws;

    let mut moved: Option<(IssueKey, String)> = None;
    let mut open: Option<IssueKey> = None;
    let mut peek: Option<IssueKey> = None; // fixes
    let mut more: Vec<(egui::Id, usize)> = Vec::new();
    let col_w = ((ui.available_width() - 12.0 * columns.len().saturating_sub(1) as f32) / columns.len().max(1) as f32).clamp(220.0, 320.0);
    egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (status, cat, col_keys) in &columns {
                let other = status == OTHER_STATUSES;
                // Only the first `shown` cards are laid out; the selection is always among them.
                let shown_id = egui::Id::unique(("board-shown", key.as_str(), status.as_str()));
                let sel_at = selected.as_ref().and_then(|s| col_keys.iter().position(|k| k == s)).map_or(0, |p| p.saturating_add(1));
                let shown = ui.data(|d| d.get_temp::<usize>(shown_id)).unwrap_or(CARD_PAGE).max(sel_at).min(col_keys.len());
                let in_col: Vec<&Issue> = col_keys.iter().take(shown).filter_map(|k| ws.issues.get(k)).collect();
                let frame = egui::Frame::new().fill(t.surface_alt).corner_radius(10).inner_margin(10);
                let (resp, dropped) = ui.dnd_drop_zone::<IssueKey, ()>(frame, |ui| {
                    // The row of columns is horizontal; each column stacks its cards vertically.
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        ui.set_min_height((ui.available_height() - 20.0).max(300.0));
                        ui.horizontal(|ui| {
                            if other {
                                ui.label(RichText::new(OTHER_STATUSES.to_uppercase()).size(11.0).strong().color(t.muted))
                                    .on_hover_text("Statuses that aren't in this project's workflow");
                            } else {
                                pill(ui, t, *cat, status);
                            }
                            ui.label(RichText::new(col_keys.len().to_string()).color(t.muted));
                        });
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().id_salt(("col", status)).auto_shrink([false, false]).show(ui, |ui| {
                            if in_col.is_empty() {
                                empty_state(ui, t, "", "Drop issues here", None);
                            }
                            for i in in_col {
                                let id = egui::Id::unique(("card", i.key.to_string()));
                                let sel = selected.as_ref() == Some(&i.key);
                                let r = ui.dnd_drag_source(id, i.key.clone(), |ui| card_body(ui, t, ws, i, col_w, sel, other)).response;
                                if r.clicked() {
                                    open = Some(i.key.clone());
                                }
                                if crate::peek::hover_button(ui, r.rect, &i.key) {
                                    peek = Some(i.key.clone()); // fixes
                                }
                                if sel && scroll {
                                    r.scroll_to_me(None);
                                }
                                ui.add_space(6.0);
                            }
                            let rest = col_keys.len().saturating_sub(shown);
                            if rest > 0 && ui.button(format!("Show {} more of {rest}", rest.min(CARD_PAGE))).clicked() {
                                more.push((shown_id, shown.saturating_add(CARD_PAGE)));
                            }
                        });
                    });
                });
                if !other && resp.response.dnd_hover_payload::<IssueKey>().is_some() {
                    ui.painter().rect_stroke(resp.response.rect, 10.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
                if let Some(k) = dropped
                    && !other
                {
                    moved = Some(((*k).clone(), status.clone()));
                }
            }
        });
    });
    for (id, n) in more {
        ui.data_mut(|d| d.insert_temp(id, n));
    }
    if let Some((k, to)) = moved
        && app.session.ws.issues.get(&k).is_some_and(|i| !i.status.eq_ignore_ascii_case(&to))
    {
        app.push(Action::Run {
            id: "issue.transition",
            params: json!({"key": k.to_string(), "to": to}),
            then: Then::Toast(format!("{k} moved to {to}")),
        });
    }
    if let Some(k) = peek {
        crate::peek::open(app, k); // fixes
    }
    if let Some(k) = open {
        app.board_sel = Some(k.clone());
        app.push(Action::Nav(View::Issue(k)));
    }
}

/// `text` wrapped to `rows` lines at most, the last ending in an ellipsis (UI-6: 255-character
/// titles and unbreakable words). The label shows the full text on hover when it was cut.
fn clamped(text: &str, rows: usize, color: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(egui::FontId::proportional(14.0), color));
    job.wrap = egui::text::TextWrapping { max_rows: rows, break_anywhere: false, overflow_character: Some('…'), ..Default::default() };
    job
}

#[allow(clippy::too_many_arguments)]
fn card_body(ui: &mut Ui, t: &Tokens, ws: &Workspace, i: &Issue, w: f32, selected: bool, other: bool) {
    let (fill, stroke) = if selected { (t.accent_soft, egui::Stroke::new(2.0, t.accent)) } else { (t.surface, egui::Stroke::new(1.0, t.border)) };
    egui::Frame::new().fill(fill).stroke(stroke).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(w - 20.0);
        ui.add(egui::Label::new(clamped(&i.summary, 3, t.text)).wrap());
        if other {
            // The orphan column mixes statuses: name each card's own.
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(i.status.to_uppercase()).size(10.5).strong().color(t.muted)).truncate());
            });
        }
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
