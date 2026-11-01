//! M4.5 notifications: the top bar's bell (unread count), the Inbox view, and the Watch toggle
//! on issue and page headers. Everything goes through commands (`inbox.markRead`,
//! `issue.watch`, `page.watch`…); notifications themselves are made by the model.

use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use serde_json::json;
use trackcraft_model::{Notification, NotifyKind, Target};

use crate::theme::Tokens;
use crate::widgets::{empty_state, name_of, who};
use crate::{Action, Then, TrackApp, View};

/// The bell button's accessible name: `Notifications` plus the unread count.
pub(crate) fn bell_label(unread: usize) -> String {
    match unread {
        0 => "Notifications".to_string(),
        1 => "Notifications, 1 unread".to_string(),
        n => format!("Notifications, {n} unread"),
    }
}

/// The bell in the top bar (drawn right to left, beside the avatar): opens the Inbox.
pub(crate) fn bell(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let unread = app.session.ws.inbox.unread(&app.session.user);
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 28.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, bell_label(unread)));
    let p = ui.painter();
    let on = resp.hovered() || app.view == View::Inbox;
    if on {
        p.rect_filled(rect, 6.0, t.surface_alt);
    }
    paint_bell(p, rect.shrink(6.0), if on { t.text } else { t.muted });
    if unread > 0 {
        let text = if unread > 99 { "99+".to_string() } else { unread.to_string() };
        let w = 8.0 + 6.0 * text.chars().count() as f32;
        let badge = Rect::from_center_size(pos2(rect.right() - w / 2.0 + 1.0, rect.top() + 7.0), vec2(w.max(16.0), 15.0));
        p.rect_filled(badge, 7.5, t.danger);
        p.text(badge.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(10.0), Color32::WHITE);
    }
    let tip = format!("{} ({})", bell_label(unread), crate::key("G then N"));
    if resp.on_hover_text(tip).clicked() {
        app.push(Action::Nav(View::Inbox));
    }
}

/// A bell drawn in code: a dome on a rim, and a clapper.
fn paint_bell(p: &egui::Painter, r: Rect, c: Color32) {
    let s = egui::Stroke::new(1.6, c);
    let (cx, top, bottom) = (r.center().x, r.top() + 1.0, r.bottom() - 3.5);
    let half = r.width() * 0.42;
    let mut pts = Vec::new();
    // Left side up, over the top, right side down: a rounded dome flaring at the rim.
    for i in 0..=16 {
        let a = std::f32::consts::PI * (i as f32 / 16.0);
        let x = cx - a.cos() * half * 0.72;
        let y = top + 4.0 - a.sin() * 3.5;
        pts.push(pos2(x, y));
    }
    let mut dome = vec![pos2(cx - half, bottom), pos2(cx - half * 0.72, bottom - 3.0)];
    dome.extend(pts);
    dome.push(pos2(cx + half * 0.72, bottom - 3.0));
    dome.push(pos2(cx + half, bottom));
    dome.push(pos2(cx - half, bottom));
    p.add(egui::Shape::line(dome, s));
    p.circle_filled(pos2(cx, bottom + 2.0), 1.8, c);
}

/// What a notification says after the actor's name.
fn verb(n: &Notification) -> &'static str {
    match n.kind {
        NotifyKind::Assigned => "assigned you",
        NotifyKind::Mentioned => "mentioned you in",
        NotifyKind::Commented => "commented on",
        NotifyKind::StatusChanged => "changed the status of",
        NotifyKind::Resolved => "resolved your issue",
    }
}

/// Where clicking a notification goes, if its issue or page still exists.
fn destination(app: &TrackApp, t: &Target) -> Option<View> {
    match t {
        Target::Issue(k) => app.session.ws.issues.contains_key(k).then(|| View::Issue(k.clone())),
        Target::Page(id) => app.session.ws.pages.contains_key(id).then_some(View::Page(*id)),
    }
}

/// The Inbox: the acting user's notifications, newest first.
pub(crate) fn view(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let me = app.session.user.clone();
    let list: Vec<Notification> = app.session.ws.inbox.of(&me).iter().rev().cloned().collect();
    let unread = list.iter().filter(|n| !n.read).count();
    let only_id = egui::Id::unique("inbox-unread-only");
    let mut only = ui.data(|d| d.get_temp::<bool>(only_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Notifications").size(22.0).strong());
        ui.label(RichText::new(if unread == 0 { "All caught up".to_string() } else { format!("{unread} unread") }).color(t.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(unread > 0, egui::Button::new("Mark all read")).clicked() {
                app.push(Action::Run { id: "inbox.markRead", params: json!({"all": true}), then: Then::Nothing });
            }
            if ui.checkbox(&mut only, "Unread only").changed() {
                ui.data_mut(|d| d.insert_temp(only_id, only));
            }
        });
    });
    ui.label(RichText::new("Assignments, mentions, and comments or status changes on what you watch.").color(t.muted).size(12.0));
    ui.add_space(10.0);
    let shown: Vec<&Notification> = list.iter().filter(|n| !only || !n.read).collect();
    if shown.is_empty() {
        let (title, hint) = if list.is_empty() {
            ("No notifications yet", "You'll hear here when someone assigns you, mentions you, or changes what you watch.")
        } else {
            ("Nothing unread", "Turn off \"Unread only\" to see earlier notifications.")
        };
        empty_state(ui, t, title, hint, None);
        return;
    }
    let now = app.session.now();
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().id_salt("inbox").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(860.0);
        for n in shown {
            if let Some(a) = row(app, ui, t, n, now) {
                actions.push(a);
            }
            ui.add_space(4.0);
        }
    });
    for a in actions {
        app.push(a);
    }
}

/// One notification; clicking it marks it read and opens its issue or page.
fn row(app: &TrackApp, ui: &mut Ui, t: &Tokens, n: &Notification, now: trackcraft_model::Timestamp) -> Option<Action> {
    let ws = &app.session.ws;
    let actor = name_of(ws, Some(&n.actor));
    let verb = verb(n);
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal_top(|ui| {
            // The unread dot (or its space).
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 26.0), Sense::hover());
            if !n.read {
                ui.painter().circle_filled(dot.center(), 4.0, t.accent);
            }
            who(ui, t, ws, Some(&n.actor), 26.0);
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let strong = |s: &str| RichText::new(s).strong().color(t.text);
                    ui.label(strong(&actor));
                    ui.label(RichText::new(verb).color(t.text));
                    ui.add(egui::Label::new(strong(&n.title)).truncate());
                });
                let detail = match n.kind {
                    NotifyKind::Commented | NotifyKind::Mentioned => n.excerpt.clone(),
                    NotifyKind::StatusChanged => format!("Now {}", n.excerpt),
                    _ => String::new(),
                };
                if !detail.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&detail).color(t.muted).size(12.5)).truncate());
                }
                ui.label(RichText::new(crate::ago(now, n.at)).color(t.muted).size(11.5));
            });
        });
    });
    let rect = inner.response.rect;
    let resp = ui.interact(rect, egui::Id::unique(("inbox-row", n.id)), Sense::click());
    let label = format!("{actor} {verb} {}", n.title);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    let fill = if resp.hovered() {
        t.surface_alt
    } else if n.read {
        t.surface
    } else {
        t.accent_soft
    };
    ui.painter().set(bg, egui::Shape::rect_filled(rect, 8.0, fill));
    let open = destination(app, &n.target);
    let resp = resp.on_hover_text(if open.is_some() { "Open and mark read" } else { "This no longer exists" });
    if !resp.clicked() {
        return None;
    }
    let mark = json!({"ids": [n.id]});
    Some(match (open, n.read) {
        (Some(v), false) => Action::Run { id: "inbox.markRead", params: mark, then: Then::Nav(v) },
        (Some(v), true) => Action::Nav(v),
        (None, false) => Action::Run { id: "inbox.markRead", params: mark, then: Then::Toast("That item no longer exists".into()) },
        (None, true) => Action::Toast("That item no longer exists".into()),
    })
}

/// The Watch / Watching toggle in an issue or page header.
pub(crate) fn watch_toggle(app: &mut TrackApp, ui: &mut Ui, target: Target) {
    let on = app.session.ws.is_watching(&app.session.user, &target);
    let (text, tip) = if on {
        ("Watching", "You're notified of comments and status changes. Click to stop watching.")
    } else {
        ("Watch", "Get notified of comments and status changes")
    };
    if ui.add(egui::Button::selectable(on, text).small()).on_hover_text(tip).clicked() {
        let (id, params) = match (&target, on) {
            (Target::Issue(k), false) => ("issue.watch", json!({"key": k.to_string()})),
            (Target::Issue(k), true) => ("issue.unwatch", json!({"key": k.to_string()})),
            (Target::Page(p), false) => ("page.watch", json!({"id": p.0})),
            (Target::Page(p), true) => ("page.unwatch", json!({"id": p.0})),
        };
        app.push(Action::Run { id, params, then: Then::Nothing });
    }
}
