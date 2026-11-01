//! The backlog (UI-11): one section per open sprint (active first), then the Backlog section,
//! each in rank order (from `board.backlog`). Rows drag by their handle to reorder
//! (`issue.rank`) or to move between sections (`issue.setSprint` with a rank anchor); the
//! keyboard does the same ([`SHORTCUTS`]). Each section ends in an inline create row.

use std::collections::BTreeSet;

use egui::{Color32, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use trackcraft_core::{Id, IssueKey, ProjectKey};
use trackcraft_model::{SprintState, StatusCategory, priority_rank};

use crate::issues::{date_range, days_left, state_chip};
use crate::theme::Tokens;
use crate::{Action, ComboBox, Then, TrackApp, View, icons};

/// The backlog's keys (also listed under `Scope::Backlog` in `keys::UI_BINDINGS` for the overlay;
/// keep both in step).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const SHORTCUTS: &[(&str, &str)] = &[
    ("J / Down", "Next issue"),
    ("K / Up", "Previous issue"),
    ("Enter / O", "Open the selected issue"),
    ("Mod+Shift+Up / Mod+Shift+Down", "Rank the issue up or down one step"),
    ("Mod+Shift+Home / Mod+Shift+End", "Rank to the top or bottom of its section"),
    ("Shift+S", "Move to sprint…"),
];

const ROW_H: f32 = 36.0;
const ROW_GAP: f32 = 2.0;

/// Per-view backlog state.
#[derive(Debug, Default)]
pub(crate) struct BacklogUi {
    /// The keyboard-selected row.
    pub sel: Option<IssueKey>,
    /// Collapsed sections, by [`section_key`].
    pub collapsed: BTreeSet<String>,
    /// The open inline create row.
    pub inline: Option<InlineCreate>,
    /// The issue the "Move to sprint…" picker is open for.
    pub move_pick: Option<IssueKey>,
    /// Only issues assigned to the acting user.
    pub mine: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct InlineCreate {
    pub project: ProjectKey,
    /// The section it creates into (`None` = the backlog).
    pub section: Option<Id>,
    pub issue_type: String,
    pub summary: String,
    /// Focus the summary field next frame.
    focus: bool,
}

/// One backlog section, as `board.backlog` returns it.
#[derive(Debug, Clone)]
pub(crate) struct Section {
    /// `None` is the Backlog section.
    pub sprint: Option<Id>,
    pub keys: Vec<IssueKey>,
    pub points: f64,
    /// To do, in progress, done.
    pub counts: [u64; 3],
}

fn section_key(project: &ProjectKey, sprint: Option<Id>) -> String {
    match sprint {
        Some(id) => format!("{project}/{id}"),
        None => format!("{project}/backlog"),
    }
}

fn parse_section(v: &Value, sprint: Option<Id>, issues: &str, points: &str, counts: &str) -> Section {
    let keys = v[issues].as_array().into_iter().flatten().filter_map(|i| i["key"].as_str()).filter_map(|k| IssueKey::parse(k).ok()).collect();
    let c = &v[counts];
    let n = |k: &str| c[k].as_u64().unwrap_or(0);
    Section { sprint, keys, points: v[points].as_f64().unwrap_or(0.0), counts: [n("todo"), n("inProgress"), n("done")] }
}

/// The project's sections in display order (open sprints, then the backlog).
pub(crate) fn sections(app: &mut TrackApp, project: &ProjectKey) -> Vec<Section> {
    let mut p = json!({"project": project.to_string()});
    if app.backlog.mine {
        p["jql"] = json!("assignee = currentUser()");
    }
    let Ok(v) = app.session.run("board.backlog", &p) else { return vec![] };
    let mut out: Vec<Section> = v["sprints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some(parse_section(s, Some(Id(s["id"].as_u64()?)), "issues", "points", "counts")))
        .collect();
    out.push(parse_section(&v, None, "backlog", "backlogPoints", "backlogCounts"));
    out
}

fn points(p: f64) -> String {
    if p.fract() == 0.0 { format!("{p:.0}") } else { format!("{p:.1}") }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

/// The rows the keyboard walks: every row of every expanded section, with its section.
fn flat(app: &TrackApp, project: &ProjectKey, secs: &[Section]) -> Vec<(IssueKey, usize)> {
    secs.iter()
        .enumerate()
        .filter(|(_, s)| !app.backlog.collapsed.contains(&section_key(project, s.sprint)))
        .flat_map(|(n, s)| s.keys.iter().map(move |k| (k.clone(), n)))
        .collect()
}

/// Where a dragged or keyboard-moved issue goes.
#[derive(Debug, Clone, PartialEq)]
struct Drop {
    key: IssueKey,
    to: Option<Id>,
    /// (neighbour, before it?)
    anchor: Option<(IssueKey, bool)>,
}

/// The command a drop runs, or `None` when nothing changes.
fn drop_action(app: &TrackApp, secs: &[Section], d: Drop) -> Option<Action> {
    let from = app.session.ws.issues.get(&d.key)?.sprint;
    let target = secs.iter().find(|s| s.sprint == d.to)?;
    // A drop on empty space in a section lands at its bottom.
    let anchor = d.anchor.or_else(|| target.keys.iter().rev().find(|k| **k != d.key).map(|k| (k.clone(), false)));
    if anchor.as_ref().is_some_and(|(a, _)| *a == d.key) {
        return None;
    }
    let mut p = json!({"keys": [d.key.to_string()]});
    if let Some((a, before)) = &anchor {
        p[if *before { "before" } else { "after" }] = json!(a.to_string());
    }
    if from == d.to {
        // Already where it would land?
        let pos = target.keys.iter().position(|k| *k == d.key)?;
        let same = match &anchor {
            None => true,
            Some((a, true)) => target.keys.get(pos.saturating_add(1)) == Some(a),
            Some((a, false)) => pos.checked_sub(1).and_then(|p| target.keys.get(p)) == Some(a),
        };
        if same {
            return None;
        }
        return Some(Action::Run { id: "issue.rank", params: p, then: Then::Nothing });
    }
    p["sprint"] = d.to.map_or(Value::Null, |i| json!(i.0));
    let name = section_name(app, d.to);
    Some(Action::Run { id: "issue.setSprint", params: p, then: Then::Toast(format!("{} moved to {name}", d.key)) })
}

fn section_name(app: &TrackApp, sprint: Option<Id>) -> String {
    sprint.and_then(|i| app.session.ws.sprints.get(&i)).map_or_else(|| "the backlog".to_string(), |s| s.name.clone())
}

/// Backlog keys; called from `keys::view_keys` when nothing types and no app dialog is open.
pub(crate) fn keys(app: &mut TrackApp, i: &mut egui::InputState, project: &ProjectKey) {
    use egui::{Key, Modifiers};
    if app.sprint_dialog.is_some() || app.backlog.move_pick.is_some() {
        return;
    }
    let secs = sections(app, project);
    let rows = flat(app, project, &secs);
    let pos = app.backlog.sel.as_ref().and_then(|s| rows.iter().position(|(k, _)| k == s));
    // Modified keys first: a plain-key pattern also matches them.
    let ms = Modifiers::COMMAND | Modifiers::SHIFT;
    let rank = [Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End].into_iter().find(|k| i.consume_key(ms, *k));
    if let (Some(k), Some(p)) = (rank, pos) {
        let Some((sel, n)) = rows.get(p).cloned() else { return };
        let Some(sec) = secs.get(n) else { return };
        let at = sec.keys.iter().position(|x| *x == sel).unwrap_or(0);
        let anchor = match k {
            Key::ArrowUp => at.checked_sub(1).and_then(|a| sec.keys.get(a)).map(|a| (a.clone(), true)),
            Key::ArrowDown => sec.keys.get(at.saturating_add(1)).map(|a| (a.clone(), false)),
            Key::Home => sec.keys.first().filter(|f| **f != sel).map(|a| (a.clone(), true)),
            _ => sec.keys.last().filter(|l| **l != sel).map(|a| (a.clone(), false)),
        };
        if let Some(anchor) = anchor
            && let Some(a) = drop_action(app, &secs, Drop { key: sel, to: sec.sprint, anchor: Some(anchor) })
        {
            app.push(a);
            app.scroll_sel = true;
        }
        return;
    }
    if i.consume_key(Modifiers::SHIFT, Key::S) {
        app.backlog.move_pick = app.backlog.sel.clone().filter(|_| pos.is_some());
        return;
    }
    let down = i.consume_key(Modifiers::NONE, Key::J) || i.consume_key(Modifiers::NONE, Key::ArrowDown);
    let up = i.consume_key(Modifiers::NONE, Key::K) || i.consume_key(Modifiers::NONE, Key::ArrowUp);
    let open = i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::O);
    let last = rows.len().saturating_sub(1);
    let next = match (pos, down, up) {
        (Some(p), true, false) => Some(p.saturating_add(1).min(last)),
        (Some(p), false, true) => Some(p.saturating_sub(1)),
        (None, true, false) => Some(0),
        (None, false, true) => Some(last),
        _ => pos,
    };
    let sel = next.and_then(|n| rows.get(n)).map(|r| r.0.clone());
    if (down || up) && sel.is_some() {
        app.backlog.sel = sel.clone();
        app.scroll_sel = true;
    }
    if open && let Some(k) = sel {
        app.push(Action::Nav(View::Issue(k)));
    }
}

/// The backlog view.
pub(crate) fn backlog(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    let Ok(pname) = app.session.ws.project(project.as_str()).map(|p| p.name.clone()) else {
        ui.label("This project no longer exists.");
        return;
    };
    let secs = sections(app, project);
    let has_active = app.session.ws.active_sprint(project).is_some();
    let now = app.session.now();
    ui.horizontal(|ui| {
        ui.heading(RichText::new(format!("{pname} backlog")).size(22.0));
        ui.add_space(8.0);
        ui.toggle_value(&mut app.backlog.mine, "Only my issues");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("+ Create sprint").color(t.on_accent).strong()).fill(t.accent)).clicked() {
                app.sprint_dialog = crate::sprints::SprintDialog::open(&app.session, "create", None, project.as_str()).ok();
            }
        });
    });
    let hint = format!(
        "Drag a row by its handle to reorder it or move it to another sprint. Keyboard: J / K select, {} ranks, Shift+S moves to a sprint.",
        crate::key("Mod+Shift+Up/Down")
    );
    ui.label(RichText::new(hint).color(t.muted).size(12.0));
    ui.add_space(8.0);

    let dragging: Option<IssueKey> = egui::DragAndDrop::payload::<IssueKey>(ui.ctx()).map(|k| (*k).clone());
    let released = ui.input(|i| i.pointer.any_released());
    let pointer = ui.ctx().pointer_latest_pos();
    let scroll = std::mem::take(&mut app.scroll_sel);
    let mut drop: Option<Drop> = None;
    let mut open: Option<IssueKey> = None;
    let mut actions: Vec<Action> = vec![];
    let mut submit: Option<InlineCreate> = None;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let clip = ui.clip_rect();
        let over = |r: Rect| pointer.is_some_and(|p| r.contains(p) && clip.contains(p));
        if secs.len() == 1 {
            ui.label(RichText::new("No sprints yet. Create one to plan the next stretch of work.").color(t.muted));
            ui.add_space(6.0);
        }
        for sec in &secs {
            let skey = section_key(project, sec.sprint);
            let collapsed = app.backlog.collapsed.contains(&skey);
            let sprint = sec.sprint.and_then(|i| app.session.ws.sprints.get(&i)).cloned();
            let mut line: Option<(Rect, bool)> = None;
            let frame =
                egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(10).inner_margin(egui::Margin::symmetric(10, 8));
            let shown = frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Header.
                ui.horizontal(|ui| {
                    let name = sprint.as_ref().map_or_else(|| "Backlog".to_string(), |s| s.name.clone());
                    let chev = icons::chevron(ui, t, !collapsed, 16.0);
                    chev.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{} {name}", if collapsed { "Expand" } else { "Collapse" }))
                    });
                    if chev.clicked() {
                        if collapsed {
                            app.backlog.collapsed.remove(&skey);
                        } else {
                            app.backlog.collapsed.insert(skey.clone());
                        }
                    }
                    ui.label(RichText::new(&name).strong().size(15.0));
                    if let Some(sp) = &sprint {
                        state_chip(ui, t, sp.state);
                        let mut when = date_range(sp.start, sp.end);
                        if sp.state == SprintState::Active
                            && let Some(left) = days_left(now, sp.end)
                        {
                            when = format!("{when} · {left}");
                        }
                        ui.label(RichText::new(when).color(t.muted).size(12.5));
                    }
                    let count = sec.keys.len();
                    ui.label(RichText::new(format!("{} · {} pts", plural(count, "issue"), points(sec.points))).color(t.muted).size(12.5));
                    status_bar(ui, t, sec.counts);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let Some(sp) = &sprint else { return };
                        let id = sp.id;
                        let more = more_button(ui, t, &format!("Sprint actions for {}", sp.name));
                        egui::Popup::menu(&more).show(|ui| {
                            if ui.button("Edit sprint…").clicked() {
                                app.sprint_dialog = crate::sprints::SprintDialog::open(&app.session, "edit", Some(id), "").ok();
                            }
                            if ui.button(RichText::new("Delete sprint…").color(t.danger)).clicked() {
                                app.sprint_dialog = crate::sprints::SprintDialog::open(&app.session, "delete", Some(id), "").ok();
                            }
                        });
                        match sp.state {
                            SprintState::Active => {
                                if ui.button("Complete sprint").clicked() {
                                    app.sprint_dialog = crate::sprints::SprintDialog::open(&app.session, "complete", Some(id), "").ok();
                                }
                                if ui.button("Board").on_hover_text("Open this sprint's board").clicked() {
                                    actions.push(Action::Nav(View::SprintBoard(project.clone(), id)));
                                }
                            }
                            SprintState::Future => {
                                let why = if has_active { "Complete the active sprint first" } else { "Choose dates and start this sprint" };
                                let b =
                                    egui::Button::new(RichText::new("Start sprint").color(if has_active { t.muted } else { t.on_accent }).strong())
                                        .fill(if has_active { t.surface_alt } else { t.accent });
                                let r = ui.add_enabled(!has_active, b).on_hover_text(why).on_disabled_hover_text(why);
                                if r.clicked() {
                                    app.sprint_dialog = crate::sprints::SprintDialog::open(&app.session, "start", Some(id), "").ok();
                                }
                            }
                            SprintState::Closed => {}
                        }
                    });
                });
                if let Some(sp) = sprint.as_ref().filter(|s| !s.goal.is_empty() && !collapsed) {
                    ui.label(RichText::new(&sp.goal).color(t.muted).size(12.5).italics());
                }
                if collapsed {
                    return;
                }
                ui.add_space(4.0);
                let ws = &app.session.ws;
                ui.spacing_mut().item_spacing.y = ROW_GAP;
                for k in &sec.keys {
                    let Some(issue) = ws.issues.get(k) else { continue };
                    let sel = app.backlog.sel.as_ref() == Some(k);
                    let w = ui.available_width();
                    let id = egui::Id::unique(("backlog-row", k.to_string()));
                    let inner = ui.dnd_drag_source(id, k.clone(), |ui| {
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
                        paint_row(ui, t, ws, issue, rect, sel, resp.hovered() || dragging.as_ref() == Some(k));
                        let label = format!("{} {}", issue.key, issue.summary);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
                        resp
                    });
                    let rect = inner.response.rect;
                    if inner.inner.on_hover_text(&issue.summary).clicked() {
                        open = Some(k.clone());
                    }
                    if sel && scroll {
                        ui.scroll_to_rect(rect, None);
                    }
                    if let (Some(d), Some(p)) = (&dragging, pointer)
                        && d != k
                        && over(rect.expand2(vec2(0.0, ROW_GAP)))
                    {
                        let before = p.y < rect.center().y;
                        line = Some((rect, before));
                        if released {
                            drop = Some(Drop { key: d.clone(), to: sec.sprint, anchor: Some((k.clone(), before)) });
                        }
                    }
                }
                if sec.keys.is_empty() {
                    let text = match &sprint {
                        Some(_) => "Drag issues here, or select one and press Shift+S to move it into this sprint.",
                        None => "Nothing in the backlog. Create an issue below, or move one out of a sprint.",
                    };
                    let hl = dragging.is_some();
                    egui::Frame::new()
                        .fill(if hl { t.drop_hint } else { t.surface_alt })
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(text).color(t.muted));
                        });
                }
                ui.add_space(4.0);
                inline_row(app, ui, t, project, sec.sprint, &mut submit);
            });
            let zone = shown.response.rect;
            if let Some(d) = &dragging
                && over(zone)
            {
                ui.painter().rect_stroke(zone, 10.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                if released && drop.is_none() {
                    drop = Some(Drop { key: d.clone(), to: sec.sprint, anchor: None });
                }
            }
            if let Some((r, before)) = line {
                let y = if before { r.top() - ROW_GAP / 2.0 } else { r.bottom() + ROW_GAP / 2.0 };
                drop_line(ui, t, r.left(), r.right(), y);
            }
            ui.add_space(10.0);
        }
    });

    for a in actions {
        app.push(a);
    }
    if let Some(d) = drop {
        egui::DragAndDrop::clear_payload(ui.ctx());
        app.backlog.sel = Some(d.key.clone());
        if let Some(a) = drop_action(app, &secs, d) {
            app.push(a);
        }
    }
    if let Some(k) = open {
        app.backlog.sel = Some(k.clone());
        app.push(Action::Nav(View::Issue(k)));
    }
    if let Some(c) = submit {
        create_inline(app, c);
    }
}

/// The 2 px accent line between rows where a drop lands, with a small ring at its start.
fn drop_line(ui: &Ui, t: &Tokens, left: f32, right: f32, y: f32) {
    let p = ui.painter();
    p.line_segment([pos2(left + 6.0, y), pos2(right, y)], Stroke::new(2.0, t.accent));
    p.circle(pos2(left + 3.0, y), 3.0, t.surface, Stroke::new(2.0, t.accent));
}

/// A section's to do / in progress / done split as a small three-segment bar.
fn status_bar(ui: &mut Ui, t: &Tokens, counts: [u64; 3]) {
    let (rect, resp) = ui.allocate_exact_size(vec2(84.0, 6.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, t.surface_alt);
    let total: u64 = counts.iter().sum();
    if total > 0 {
        let mut x = rect.left();
        let cats = [StatusCategory::ToDo, StatusCategory::InProgress, StatusCategory::Done];
        for (n, cat) in counts.iter().zip(cats) {
            if *n == 0 {
                continue;
            }
            let w = rect.width() * (*n as f32 / total as f32);
            let seg = Rect::from_min_max(pos2(x, rect.top()), pos2((x + w).min(rect.right()), rect.bottom()));
            p.rect_filled(seg, 0.0, t.progress(cat));
            x += w;
        }
        // Round the ends over the square segments.
        p.rect_stroke(rect, 3.0, Stroke::new(1.0, t.surface), egui::StrokeKind::Outside);
    }
    let [a, b, c] = counts;
    resp.on_hover_text(format!("{a} to do, {b} in progress, {c} done"));
}

/// A small "more actions" button drawn as three dots.
fn more_button(ui: &mut Ui, t: &Tokens, label: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, t.accent_soft);
    }
    icons::paint_more(ui.painter(), if resp.hovered() { t.text } else { t.muted }, rect.center());
    let label = label.to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    resp
}

/// One painted row: handle, type, key, summary, status pill, priority, points, assignee.
fn paint_row(ui: &Ui, t: &Tokens, ws: &trackcraft_model::Workspace, i: &trackcraft_model::Issue, rect: Rect, sel: bool, hover: bool) {
    let p = ui.painter();
    if sel {
        p.rect_filled(rect, 6.0, t.accent_soft);
        p.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 1.5, t.accent);
    } else if hover {
        p.rect_filled(rect, 6.0, t.surface_alt);
    }
    let cy = rect.center().y;
    icons::paint_drag_handle(p, if hover || sel { t.text } else { t.muted }, pos2(rect.left() + 12.0, cy));
    icons::paint_issue_type(p, Rect::from_center_size(pos2(rect.left() + 34.0, cy), vec2(16.0, 16.0)), &i.issue_type);
    let key_font = egui::FontId::monospace(12.0);
    p.text(pos2(rect.left() + 50.0, cy), egui::Align2::LEFT_CENTER, i.key.to_string(), key_font, t.muted);

    // Right block, right to left: avatar, points, priority, status pill.
    let mut x = rect.right() - 10.0;
    let av = Rect::from_center_size(pos2(x - 11.0, cy), vec2(22.0, 22.0));
    match i.assignee.as_deref().and_then(|u| ws.users.get(u)) {
        Some(u) => icons::paint_avatar(p, t, av, &u.id, &u.display_name),
        None => {
            p.circle_stroke(av.center(), 10.5, Stroke::new(1.0, t.muted));
        }
    }
    x = av.left() - 10.0;
    let chip = Rect::from_min_max(pos2(x - 28.0, cy - 10.0), pos2(x, cy + 10.0));
    if let Some(sp) = i.story_points {
        p.rect_filled(chip, 10.0, t.surface_alt);
        p.rect_stroke(chip, 10.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        p.text(chip.center(), egui::Align2::CENTER_CENTER, points(sp), egui::FontId::proportional(11.5), t.text);
    } else {
        p.text(chip.center(), egui::Align2::CENTER_CENTER, "-", egui::FontId::proportional(11.5), t.muted);
    }
    x = chip.left() - 12.0;
    let pr = Rect::from_min_max(pos2(x - 14.0, cy - 7.0), pos2(x, cy + 7.0));
    icons::paint_priority(p, t, pr, priority_rank(&i.priority).unwrap_or(3));
    x = pr.left() - 12.0;
    let (fill, fg) = t.status(ws.status_category(i));
    let pill_font = egui::FontId::proportional(10.5);
    let galley = truncated(ui, &i.status.to_uppercase(), pill_font, fg, 104.0);
    let pill = Rect::from_min_max(pos2(x - galley.size().x - 12.0, cy - 9.0), pos2(x, cy + 9.0));
    p.rect_filled(pill, 4.0, fill);
    p.galley(pos2(pill.left() + 6.0, cy - galley.size().y / 2.0), galley, fg);

    // Summary fills what is left.
    let left = rect.left() + 128.0;
    let room = (pill.left() - 16.0 - left).max(40.0);
    let done = ws.status_category(i) == StatusCategory::Done;
    let galley = truncated(ui, &i.summary, egui::FontId::proportional(14.0), if done { t.muted } else { t.text }, room);
    p.galley(pos2(left, cy - galley.size().y / 2.0), galley, t.text);
}

/// One line of text cut to `width` with an ellipsis.
fn truncated(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.fonts_mut(|f| f.layout_job(job))
}

/// "+ Create issue" at the bottom of a section, or the open inline form.
fn inline_row(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey, section: Option<Id>, submit: &mut Option<InlineCreate>) {
    let open_here = app.backlog.inline.as_ref().is_some_and(|c| &c.project == project && c.section == section);
    if !open_here {
        let b = egui::Button::new(RichText::new("+ Create issue").color(t.muted)).frame(false);
        if ui.add(b).clicked() {
            let types = plannable_types(app, project);
            let issue_type = ["Story", "Task"]
                .into_iter()
                .find(|d| types.iter().any(|x| x == d))
                .map(str::to_string)
                .or_else(|| types.first().cloned())
                .unwrap_or_else(|| "Task".into());
            app.backlog.inline = Some(InlineCreate { project: project.clone(), section, issue_type, summary: String::new(), focus: true });
        }
        return;
    }
    let types = plannable_types(app, project);
    let Some(c) = app.backlog.inline.as_mut() else { return };
    let mut cancel = false;
    egui::Frame::new().fill(t.surface_alt).stroke(Stroke::new(1.0, t.accent)).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 4)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                let label = ui.label(RichText::new("New issue").color(t.muted).size(12.0)).id;
                ui.add_space(4.0);
                icons::issue_type(ui, &c.issue_type, 16.0);
                let salt = format!("inline-type-{}", section_key(project, section));
                ComboBox::from_id_salt(salt)
                    .selected_text(c.issue_type.clone())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for ty in &types {
                            ui.selectable_value(&mut c.issue_type, ty.clone(), ty);
                        }
                    })
                    .response
                    .labelled_by(label);
                let w = (ui.available_width() - 150.0).max(120.0);
                let r = ui
                    .add(egui::TextEdit::singleline(&mut c.summary).desired_width(w).hint_text("What needs doing? Enter creates it, Escape cancels"))
                    .labelled_by(label);
                if std::mem::take(&mut c.focus) {
                    r.request_focus();
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape)) && (r.has_focus() || r.lost_focus());
                if (enter || ui.button("Create").clicked()) && !c.summary.trim().is_empty() {
                    *submit = Some(c.clone());
                    c.summary.clear();
                    c.focus = true;
                }
                cancel = escape || ui.button("Cancel").clicked();
            });
        },
    );
    if cancel {
        app.backlog.inline = None;
    }
}

/// Issue types a backlog row can be created as (epics and sub-tasks are planned elsewhere).
fn plannable_types(app: &TrackApp, project: &ProjectKey) -> Vec<String> {
    app.session
        .ws
        .project(project.as_str())
        .map(|p| {
            p.issue_types.iter().map(|t| t.name.clone()).filter(|n| !n.eq_ignore_ascii_case("Epic") && !n.eq_ignore_ascii_case("Sub-task")).collect()
        })
        .unwrap_or_default()
}

/// Creates the inline row's issue, then moves it into its sprint (new issues start in the
/// backlog, ranked last).
fn create_inline(app: &mut TrackApp, c: InlineCreate) {
    let p = json!({"project": c.project.to_string(), "type": c.issue_type, "summary": c.summary.trim()});
    match app.session.run("issue.create", &p) {
        Ok(v) => {
            app.jql_ran = None;
            let Some(k) = v["key"].as_str().and_then(|k| IssueKey::parse(k).ok()) else { return };
            if let Some(sp) = c.section
                && let Err(e) = app.session.run("issue.setSprint", &json!({"keys": [k.to_string()], "sprint": sp.0}))
            {
                app.notify(format!("Created {k}, but it stayed in the backlog: {e}"));
                app.backlog.sel = Some(k);
                return;
            }
            app.notify(format!("Created {k}"));
            app.backlog.sel = Some(k);
        }
        Err(e) => app.notify(e),
    }
}

/// The "Move to sprint…" picker for the selected issue.
pub(crate) fn move_picker(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(key) = app.backlog.move_pick.clone() else { return };
    let View::Backlog(project) = app.view.clone() else {
        app.backlog.move_pick = None;
        return;
    };
    let Some(from) = app.session.ws.issues.get(&key).map(|i| i.sprint) else {
        app.backlog.move_pick = None;
        return;
    };
    let secs = sections(app, &project);
    let targets: Vec<(Option<Id>, String)> = secs
        .iter()
        .filter(|s| s.sprint != from)
        .map(|s| {
            let label = match s.sprint.and_then(|i| app.session.ws.sprints.get(&i)) {
                Some(sp) => format!("{} ({})", sp.name, if sp.state == SprintState::Active { "active" } else { "planned" }),
                None => "Backlog".to_string(),
            };
            (s.sprint, label)
        })
        .collect();
    let mut chosen: Option<Option<Id>> = None;
    let mut close = false;
    let summary = app.session.ws.issues.get(&key).map(|i| i.summary.clone()).unwrap_or_default();
    let modal = egui::Modal::new(egui::Id::unique("move-to-sprint")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading("Move to sprint");
        ui.label(RichText::new(format!("{key}  {summary}")).color(t.muted));
        ui.add_space(8.0);
        for (n, (to, label)) in targets.iter().enumerate() {
            let r = ui.add_sized([ui.available_width(), 28.0], egui::Button::new(label.as_str()));
            if n == 0 && !ui.memory(|m| m.focused().is_some()) {
                r.request_focus();
            }
            if r.clicked() {
                chosen = Some(*to);
            }
        }
        ui.add_space(6.0);
        close |= ui.button("Cancel").clicked();
    });
    if modal.should_close() {
        close = true;
    }
    if let Some(to) = chosen {
        if let Some(a) = drop_action(app, &secs, Drop { key, to, anchor: None }) {
            app.push(a);
        }
        close = true;
    }
    if close {
        app.backlog.move_pick = None;
    }
}
