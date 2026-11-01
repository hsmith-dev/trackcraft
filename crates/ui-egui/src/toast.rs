//! Toasts (UI-5): a stack of short messages at the bottom centre, newest at the bottom.
//!
//! Kinds: `Info`, `Success` and `Error`. Info and success fade after 4 s, errors after 10 s
//! (or when dismissed); hovering a toast pauses its timer. A toast for a change that was just
//! made carries an **Undo** button, shown only while that change is still the newest one (see
//! [`Stamp`]).

use egui::{Align, Align2, Color32, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};
use serde_json::json;
use trackcraft_core::IdGen;
use trackcraft_engine::Session;

use crate::theme::Tokens;
use crate::{Action, Then, TrackApp};

/// At most this many toasts are on screen; a new one pushes the oldest out.
pub(crate) const MAX_VISIBLE: usize = 3;
/// Longest message kept (engine errors can quote hostile input).
const MAX_CHARS: usize = 400;
/// Content width of every toast.
const WIDTH: f32 = 460.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Info,
    Success,
    Error,
}

impl Kind {
    /// Seconds on screen (not counting time hovered).
    pub(crate) fn life(self) -> f64 {
        match self {
            Kind::Error => 10.0,
            Kind::Info | Kind::Success => 4.0,
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Kind> {
        match s {
            "" | "info" => Some(Kind::Info),
            "success" | "done" => Some(Kind::Success),
            "error" => Some(Kind::Error),
            _ => None,
        }
    }
}

/// Where the history stood right after a change: the UI's count of history-changing commands
/// plus the workspace id counter (which also moves when a view calls `Session::run` directly).
/// Undo is offered only while both still match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stamp {
    changes: u64,
    ids: IdGen,
}

#[derive(Debug, Clone)]
pub(crate) struct Toast {
    pub id: u64,
    pub kind: Kind,
    pub text: String,
    pub undo: Option<Stamp>,
    /// Seconds shown so far (paused while hovered).
    shown: f64,
    /// The frame time of the last tick (`None` before the first).
    last: Option<f64>,
    hovered: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Toasts {
    pub items: Vec<Toast>,
    next_id: u64,
    /// History-changing commands run through the UI (bumped by `TrackApp::run` and the
    /// control channel).
    pub changes: u64,
}

/// Commands that move the undo history (a later Undo would no longer revert an earlier toast's change).
pub(crate) fn moves_history(id: &str) -> bool {
    trackcraft_engine::command(id).is_some_and(|c| c.mutates) || matches!(id, "edit.undo" | "edit.redo" | "file.open" | "workspace.new")
}

impl Toasts {
    pub(crate) fn push(&mut self, kind: Kind, text: impl Into<String>, undo: Option<Stamp>) {
        let mut text: String = text.into();
        if text.chars().count() > MAX_CHARS {
            text = text.chars().take(MAX_CHARS).collect::<String>() + "...";
        }
        self.next_id = self.next_id.wrapping_add(1);
        self.items.push(Toast { id: self.next_id, kind, text, undo, shown: 0.0, last: None, hovered: false });
        while self.items.len() > MAX_VISIBLE {
            self.items.remove(0);
        }
    }

    /// The history as it stands now.
    pub(crate) fn stamp(&self, s: &Session) -> Stamp {
        Stamp { changes: self.changes, ids: s.ws.ids.clone() }
    }

    /// The toast's Undo would revert its own change (nothing ran since).
    pub(crate) fn can_undo(&self, toast: &Toast, s: &Session) -> bool {
        s.can_undo() && toast.undo.as_ref().is_some_and(|st| *st == self.stamp(s))
    }

    pub(crate) fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    /// Advances the timers to `now` (seconds) and drops expired toasts. True while any remain.
    pub(crate) fn tick(&mut self, now: f64) -> bool {
        for t in &mut self.items {
            let dt = t.last.map_or(0.0, |l| (now - l).clamp(0.0, 1.0));
            t.last = Some(now);
            if !t.hovered {
                t.shown += dt;
            }
        }
        self.items.retain(|t| t.shown < t.kind.life());
        !self.items.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn set_hovered(&mut self, id: u64, hovered: bool) {
        for t in &mut self.items {
            if t.id == id {
                t.hovered = hovered;
            }
        }
    }
}

/// What a click on a toast asks for.
enum Click {
    Undo(u64),
    Dismiss(u64),
}

/// Draws the stack above the status bar.
pub(crate) fn show(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    if app.toasts.items.is_empty() {
        return;
    }
    let mut click = None;
    let mut hovered = vec![];
    let offset = -(crate::status_bar::HEIGHT + 14.0);
    egui::Area::new(egui::Id::unique("toasts")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, offset)).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for toast in &app.toasts.items {
                let undo = app.toasts.can_undo(toast, &app.session);
                let r = one(ui, t, toast, undo, &mut click);
                hovered.push((toast.id, r));
            }
        });
    });
    for (id, h) in hovered {
        if let Some(x) = app.toasts.items.iter_mut().find(|x| x.id == id) {
            x.hovered = h;
        }
    }
    match click {
        Some(Click::Undo(id)) => {
            let ok = app.toasts.items.iter().find(|x| x.id == id).is_some_and(|x| app.toasts.can_undo(x, &app.session));
            app.toasts.dismiss(id);
            if ok {
                app.push(Action::Run { id: "edit.undo", params: json!({}), then: Then::Toast("Undone".into()) });
            }
        }
        Some(Click::Dismiss(id)) => app.toasts.dismiss(id),
        None => {}
    }
}

/// One toast; returns whether the pointer is over it.
fn one(ui: &mut Ui, t: &Tokens, toast: &Toast, undo: bool, click: &mut Option<Click>) -> bool {
    let (fill, fg) = match toast.kind {
        Kind::Info | Kind::Success => (t.text, t.bg),
        Kind::Error => (t.danger, t.bg),
    };
    // Every toast is the same width, so the stack reads as one column; buttons sit at the right.
    let frame = egui::Frame::new().fill(fill).corner_radius(8).inner_margin(egui::Margin { left: 14, right: 8, top: 8, bottom: 8 }).show(ui, |ui| {
        ui.set_width(WIDTH);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match toast.kind {
                Kind::Success => paint_check(ui, t),
                Kind::Error => paint_alert(ui, t, fg, fill),
                Kind::Info => {}
            }
            ui.scope(|ui| {
                ui.set_max_width(WIDTH - 150.0);
                ui.add(egui::Label::new(RichText::new(&toast.text).color(fg)).wrap());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if close(ui, fg).clicked() {
                    *click = Some(Click::Dismiss(toast.id));
                }
                if undo && link(ui, fg, "Undo", "Undo this change").on_hover_text(format!("Undo this change ({})", crate::key("Mod+Z"))).clicked() {
                    *click = Some(Click::Undo(toast.id));
                }
                if toast.kind == Kind::Error && link(ui, fg, "Copy", "Copy message").on_hover_text("Copy the message").clicked() {
                    ui.ctx().copy_text(toast.text.clone());
                }
            });
        });
    });
    frame.response.contains_pointer()
}

/// A frameless text button; `name` is its accessible name (the top bar has its own "Undo").
fn link(ui: &mut Ui, fg: Color32, text: &str, name: &str) -> egui::Response {
    let r = ui.add(egui::Button::new(RichText::new(text).color(fg).strong()).frame(false));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    r
}

/// A painted X (no glyph), named "Dismiss" for screen readers and tests.
fn close(ui: &mut Ui, fg: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Dismiss"));
    let c = rect.center();
    let d = if resp.hovered() { 4.5 } else { 4.0 };
    let stroke = Stroke::new(1.6, fg);
    let p = ui.painter();
    p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
    p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
    resp.on_hover_text("Dismiss")
}

fn icon_rect(ui: &mut Ui) -> Rect {
    ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover()).0
}

fn paint_check(ui: &mut Ui, t: &Tokens) {
    let r = icon_rect(ui);
    let c = r.center();
    let p = ui.painter();
    p.circle_filled(c, 8.0, t.ok);
    let s = Stroke::new(1.8, t.on_accent);
    let pts: [Pos2; 3] = [c + vec2(-3.5, 0.2), c + vec2(-1.0, 2.8), c + vec2(3.8, -2.6)];
    p.line_segment([pts[0], pts[1]], s);
    p.line_segment([pts[1], pts[2]], s);
}

fn paint_alert(ui: &mut Ui, _t: &Tokens, fg: Color32, fill: Color32) {
    let r = icon_rect(ui);
    let c = r.center();
    let p = ui.painter();
    p.circle_filled(c, 8.0, fg);
    p.line_segment([c + vec2(0.0, -4.0), c + vec2(0.0, 1.2)], Stroke::new(2.0, fill));
    p.circle_filled(c + vec2(0.0, 4.0), 1.2, fill);
}
