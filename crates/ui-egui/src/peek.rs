//! The peek pane (UI-fixes, ui-plan §2.1 / §3.1): a right-hand, read-only look at an issue from
//! the issue list, the board and the backlog, without leaving them. `Space` on the selected row
//! or card (or a row's "Peek" hover button) opens it; moving the selection with the arrow keys
//! moves the pane along; `Escape` or "Close" shuts it; "Open" shows the full issue.

use egui::{RichText, Ui};
use trackcraft_core::IssueKey;

use crate::theme::Tokens;
use crate::widgets::{name_of, pill, snippet, who};
use crate::{Action, TrackApp, View, markup_view};

/// The pane's width.
const WIDTH: f32 = 380.0;
/// Comments the pane shows (the newest).
const COMMENTS: usize = 3;
/// Characters of each comment shown.
const COMMENT_CHARS: usize = 280;

/// The view shows issues the pane can peek at.
pub(crate) fn peekable(v: &View) -> bool {
    matches!(v, View::Issues | View::Board(_) | View::SprintBoard(..) | View::Backlog(_))
}

/// The keyboard-selected issue of the current view.
fn selected(app: &TrackApp) -> Option<IssueKey> {
    match &app.view {
        View::Issues => app.list_sel.clone(),
        View::Board(_) | View::SprintBoard(..) => app.board_sel.clone(),
        View::Backlog(_) => app.backlog.sel.clone(),
        _ => None,
    }
}

/// Peeks at `key`, selecting it in the current view so the arrow keys continue from there.
pub(crate) fn open(app: &mut TrackApp, key: IssueKey) {
    match &app.view {
        View::Issues => app.list_sel = Some(key.clone()),
        View::Board(_) | View::SprintBoard(..) => app.board_sel = Some(key.clone()),
        View::Backlog(_) => app.backlog.sel = Some(key.clone()),
        _ => {}
    }
    app.peek = Some(key);
}

/// `Space`: peek at the selected issue, or close the pane when it already shows it.
pub(crate) fn toggle(app: &mut TrackApp) {
    match selected(app) {
        Some(k) if app.peek.as_ref() == Some(&k) => app.peek = None,
        Some(k) => app.peek = Some(k),
        None => app.notify("Select an issue first (J / K or the arrow keys), then press Space to peek"),
    }
}

/// Per-frame upkeep: the pane follows the selection, and closes on `Escape` (when no dialog or
/// text field has it), when the issue is gone, or when the view can't peek.
pub(crate) fn logic(app: &mut TrackApp, ctx: &egui::Context, modal: bool) {
    if app.peek.is_none() {
        return;
    }
    if !peekable(&app.view) {
        app.peek = None;
        return;
    }
    if let Some(s) = selected(app)
        && app.peek.as_ref() != Some(&s)
    {
        app.peek = Some(s);
    }
    if app.peek.as_ref().is_some_and(|k| !app.session.ws.issues.contains_key(k)) {
        app.peek = None;
        return;
    }
    if !modal && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.peek = None;
    }
}

/// A "Peek" button over the right end of `row` while the pointer is on it (rows and cards draw
/// it after themselves). Returns true when it was clicked.
pub(crate) fn hover_button(ui: &Ui, row: egui::Rect, key: &IssueKey) -> bool {
    let Some(p) = ui.ctx().pointer_latest_pos() else { return false };
    if !row.contains(p) || !ui.clip_rect().contains(p) || egui::DragAndDrop::has_any_payload(ui.ctx()) {
        return false;
    }
    let size = egui::vec2(48.0, 20.0);
    let at = egui::pos2(row.right() - size.x - 6.0, row.center().y - size.y / 2.0);
    let r = egui::Area::new(egui::Id::unique("peek-hover-button"))
        .fixed_pos(at)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            let b = ui.add(egui::Button::new(RichText::new("Peek").size(11.5)).small().min_size(size));
            b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Peek at {key}")));
            b
        })
        .inner;
    r.on_hover_text(format!("Peek at {key} (Space)")).clicked()
}

/// Draws the pane at the right of the window when one is open (before the central view).
pub(crate) fn pane(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let Some(key) = app.peek.clone() else { return };
    if !peekable(&app.view) {
        return;
    }
    let mut close = false;
    let mut open_full = false;
    egui::Panel::right("peek")
        .resizable(true)
        .default_size(WIDTH)
        .frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(16, 12)).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            let ws = &app.session.ws;
            let Some(issue) = ws.issues.get(&key) else { return };
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} \u{00b7} {}", issue.key, issue.issue_type)).color(t.muted).size(12.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.button("Close").on_hover_text("Close the peek pane (Escape)").clicked();
                    let o = egui::Button::new(RichText::new("Open").color(t.on_accent).strong()).fill(t.accent);
                    open_full = ui.add(o).on_hover_text(format!("Open {} (Enter)", issue.key)).clicked();
                });
            });
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&issue.summary).size(18.0).strong()).wrap());
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                pill(ui, t, ws.status_category(issue), &issue.status);
                ui.label(RichText::new(&issue.priority).color(t.muted).size(12.5));
                if let Some(p) = issue.story_points {
                    ui.label(RichText::new(if p == 1.0 { "1 point".to_string() } else { format!("{p} points") }).color(t.muted).size(12.5));
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Assignee").color(t.muted).size(12.5));
                who(ui, t, ws, issue.assignee.as_deref(), 20.0);
                ui.label(name_of(ws, issue.assignee.as_deref()));
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("peek-body").auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Description").strong());
                ui.add_space(2.0);
                markup_view::show(ui, t, &issue.description);
                ui.add_space(10.0);
                let n = issue.comments.len();
                ui.label(RichText::new(if n == 0 { "No comments".to_string() } else { format!("Latest comments ({n})") }).strong());
                let now = app.session.now();
                for c in issue.comments.iter().rev().take(COMMENTS) {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        who(ui, t, ws, Some(&c.author), 18.0);
                        ui.label(RichText::new(name_of(ws, Some(&c.author))).strong().size(12.5));
                        ui.label(RichText::new(crate::ago(now, c.created)).color(t.muted).size(12.0));
                    });
                    ui.add(egui::Label::new(snippet(&c.body.plain_text(), COMMENT_CHARS)).wrap());
                }
                if n > COMMENTS {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("{} older comments: open the issue to read them.", n - COMMENTS)).color(t.muted).size(12.0));
                }
            });
        });
    if close {
        app.peek = None;
    }
    if open_full {
        app.push(Action::Nav(View::Issue(key)));
    }
}
