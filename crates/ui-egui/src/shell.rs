//! The window frame: top bar (back/forward, search, create), the bounded sidebar (filter box,
//! starred, recent, filters, projects, spaces with page trees) and home.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::Id;

use trackcraft_core::ProjectKey;

use crate::directory::{self, Star};
use crate::theme::Tokens;
use crate::widgets::{card, empty_state};
use crate::{Action, ComboBox, FileRequest, Then, TrackApp, View, icons, keys};

pub(crate) fn top_bar(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::top("top")
        .exact_size(52.0)
        .frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(14, 8)).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if icons::app_mark(ui, t, 24.0).interact(egui::Sense::click()).on_hover_text("Home").clicked() {
                    app.push(Action::Nav(View::Home));
                }
                ui.label(RichText::new(&app.session.ws.name).strong().size(15.0));
                ui.add_space(8.0);
                ui.menu_button("File", |ui| {
                    for (label, sc, req) in [
                        ("New workspace", "", FileRequest::New),
                        ("Open…", "Mod+O", FileRequest::Open),
                        ("Save", "Mod+S", FileRequest::Save),
                        ("Save As…", "Mod+Shift+S", FileRequest::SaveAs),
                    ] {
                        if ui.add(egui::Button::new(label).shortcut_text(crate::key(sc))).clicked() {
                            app.push(Action::File(req));
                            ui.close();
                        }
                    }
                });
                ui.add_space(8.0);
                // UI-8: back and forward, with the keymap's bindings in the tooltip.
                let back = keys::shortcut_for(app, keys::Act::Back);
                if nav_arrow(ui, t, false, app.can_back(), "Back", back).clicked() {
                    app.push(Action::Back);
                }
                let fwd = keys::shortcut_for(app, keys::Act::Forward);
                if nav_arrow(ui, t, true, app.can_forward(), "Forward", fwd).clicked() {
                    app.push(Action::Forward);
                }
                ui.add_space(4.0);
                let w = (ui.available_width() - 420.0).clamp(160.0, 520.0);
                let mut q = app.jql.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut q).desired_width(w).hint_text("Search issues with JQL  ( / )").font(egui::TextStyle::Monospace),
                );
                if app.take_focus_search() {
                    resp.request_focus();
                }
                if resp.changed() {
                    app.jql = q;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    app.push(Action::Search(app.jql.clone()));
                }
                let create = egui::Button::new(RichText::new("+ Create").color(t.on_accent).strong()).fill(t.accent);
                if ui.add(create).on_hover_text("Create an issue (C)").clicked() {
                    app.push(Action::OpenCreate(current_project(app)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let me = app.session.user.clone();
                    let display = app.session.ws.users.get(&me).map(|u| u.display_name.clone()).unwrap_or_else(|| me.clone());
                    // Right-to-left: the combo is drawn before its label, and linked to it after.
                    let acting = ComboBox::from_id_salt("acting-user")
                        .selected_text(display.clone())
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            let users: Vec<(String, String)> =
                                app.session.ws.users.values().map(|u| (u.id.clone(), u.display_name.clone())).collect();
                            for (id, name) in users {
                                if ui.selectable_label(id == me, &name).clicked() {
                                    app.push(Action::Run {
                                        id: "user.switch",
                                        params: json!({"id": id}),
                                        then: Then::Toast(format!("Acting as {name}")),
                                    });
                                }
                            }
                        })
                        .response;
                    icons::avatar(ui, t, &me, &display, 26.0);
                    let label = ui.label(RichText::new("Acting as").color(t.muted).size(12.0));
                    let _ = acting.labelled_by(label.id);
                    let theme = if t.dark { "Light" } else { "Dark" };
                    if ui.small_button(theme).on_hover_text("Switch the interface theme").clicked() {
                        app.push(Action::ToggleTheme);
                    }
                    let redo = ui
                        .add_enabled(app.session.can_redo(), egui::Button::new("Redo").small())
                        .on_hover_text(format!("Redo ({})", crate::key("Mod+Shift+Z")));
                    if redo.clicked() {
                        app.push(Action::Run { id: "edit.redo", params: json!({}), then: Then::Nothing });
                    }
                    let undo = ui
                        .add_enabled(app.session.can_undo(), egui::Button::new("Undo").small())
                        .on_hover_text(format!("Undo ({})", crate::key("Mod+Z")));
                    if undo.clicked() {
                        app.push(Action::Run { id: "edit.undo", params: json!({}), then: Then::Nothing });
                    }
                    if ui.small_button(crate::key("Mod+K")).on_hover_text("Command palette").clicked() {
                        app.push(Action::TogglePalette);
                    }
                });
            });
        });
}

/// The project the current view belongs to (board, backlog, issue).
pub(crate) fn current_project(app: &TrackApp) -> Option<String> {
    match &app.view {
        View::Board(p) | View::Backlog(p) | View::SprintBoard(p, _) => Some(p.to_string()),
        View::Issue(k) => Some(k.project.to_string()),
        _ => None,
    }
}

/// The space the current view belongs to (a page or its history).
pub(crate) fn current_space(app: &TrackApp) -> Option<String> {
    match &app.view {
        View::Page(id) | View::PageHistory(id) => app.session.ws.pages.get(id).map(|p| p.space.to_string()),
        _ => None,
    }
}

/// A back or forward button: a painted chevron, labelled for assistive tech.
fn nav_arrow(ui: &mut Ui, t: &Tokens, forward: bool, enabled: bool, label: &str, keys: Option<String>) -> egui::Response {
    let size = egui::vec2(26.0, 26.0);
    let (r, resp) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(r, 6.0, t.surface_alt);
    }
    let c = r.center();
    let s = 4.5;
    let dir = if forward { 1.0 } else { -1.0 };
    let col = if !enabled {
        t.border
    } else if resp.hovered() {
        t.text
    } else {
        t.muted
    };
    let pts = vec![c + egui::vec2(-dir * s * 0.5, -s), c + egui::vec2(dir * s * 0.5, 0.0), c + egui::vec2(-dir * s * 0.5, s)];
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.8, col)));
    let tip = match keys {
        Some(k) => format!("{label} ({k})"),
        None => label.to_string(),
    };
    resp.on_hover_text(tip)
}

fn nav_item(ui: &mut Ui, t: &Tokens, selected: bool, indent: f32, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let text = if selected { RichText::new(label).color(t.accent).strong() } else { RichText::new(label) };
        ui.add(egui::Button::selectable(selected, text).frame_when_inactive(false).truncate())
    })
    .inner
}

/// What a sidebar row reported: the item, its star or its chevron was clicked.
#[derive(Default)]
struct RowHit {
    open: bool,
    star: bool,
    chevron: bool,
}

/// A sidebar row: optional chevron, a selectable label and, on hover or when starred, a star at
/// the right edge.
struct Row<'a> {
    indent: f32,
    selected: bool,
    strong: bool,
    label: &'a str,
    /// `Some(open)` draws a disclosure chevron.
    chevron: Option<bool>,
    /// `Some((starred, name))` offers the star toggle for `name`.
    star: Option<(bool, &'a str)>,
    /// Draw a "+" button (create a page in a space) on hover or when selected.
    plus: bool,
}

impl Row<'_> {
    fn show(self, ui: &mut Ui, t: &Tokens) -> (RowHit, Option<egui::Response>) {
        let mut hit = RowHit::default();
        let mut plus = None;
        ui.horizontal(|ui| {
            let row = ui.max_rect();
            let hovered = ui.rect_contains_pointer(egui::Rect::from_min_max(row.min, egui::pos2(row.max.x, row.min.y + 22.0)));
            ui.add_space(self.indent);
            if let Some(open) = self.chevron {
                let c = icons::chevron(ui, t, open, 14.0);
                c.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{} {}", if open { "Collapse" } else { "Expand" }, self.label))
                });
                hit.chevron = c.clicked();
            }
            let tail = 20.0 + if self.plus { 24.0 } else { 0.0 };
            let w = (row.right() - ui.cursor().left() - tail).max(40.0);
            let text = match (self.selected, self.strong) {
                (true, _) => RichText::new(self.label).color(t.accent).strong(),
                (false, true) => RichText::new(self.label).strong(),
                (false, false) => RichText::new(self.label),
            };
            let r = ui.scope(|ui| {
                ui.set_max_width(w);
                ui.add(egui::Button::selectable(self.selected, text).frame_when_inactive(false).truncate())
            });
            hit.open = r.inner.clicked();
            let y = r.inner.rect.center().y;
            if self.plus && (hovered || self.selected) {
                let rect = egui::Rect::from_center_size(egui::pos2(row.right() - 32.0, y), egui::vec2(20.0, 18.0));
                plus = Some(ui.put(rect, egui::Button::new("+").small()));
            }
            if let Some((on, name)) = self.star
                && (on || hovered)
            {
                let rect = egui::Rect::from_center_size(egui::pos2(row.right() - 8.0, y), egui::vec2(14.0, 14.0));
                let s = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| directory::star_toggle(ui, t, on, name, 14.0)).inner;
                hit.star = s.clicked();
            }
        });
        (hit, plus)
    }
}

/// A collapsible section header with an optional "All …" link; returns whether it is open.
fn section(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, title: &'static str, all: Option<(&str, View)>) -> bool {
    let open = !app.side.collapsed.contains(title);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let c = icons::chevron(ui, t, open, 12.0);
        c.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{} {title}", if open { "Collapse" } else { "Expand" })));
        let l = ui.add(egui::Label::new(RichText::new(title.to_uppercase()).size(11.0).color(t.muted).strong()).sense(egui::Sense::click()));
        if c.clicked() || l.clicked() {
            if open {
                app.side.collapsed.insert(title);
            } else {
                app.side.collapsed.remove(title);
            }
        }
        if let Some((label, to)) = all {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.link(RichText::new(label).size(11.0).color(t.muted)).clicked() {
                    app.push(Action::Nav(to));
                }
            });
        }
    });
    ui.add_space(2.0);
    open
}

/// The sidebar's built-in filters: (label, JQL).
pub(crate) const FILTERS: &[(&str, &str)] = &[
    ("My open issues", "assignee = currentUser() AND statusCategory != Done ORDER BY priority"),
    ("Reported by me", "reporter = currentUser() ORDER BY created DESC"),
    ("All issues", "ORDER BY updated DESC"),
    ("Recently resolved", "resolved >= -14d ORDER BY resolved DESC"),
];

/// At most this many projects or spaces are listed; the rest are in the directories.
const SECTION_CAP: usize = 8;

/// A workspace with at most this many projects (or spaces) opens them all in the sidebar.
const SMALL: usize = 3;

pub(crate) fn sidebar(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(250.0)
        .frame(egui::Frame::new().fill(t.sidebar).inner_margin(egui::Margin::symmetric(12, 6)))
        .show(ui, |ui| {
            app.side.sync(ui.ctx());
            ui.add_space(4.0);
            let r = ui.add(egui::TextEdit::singleline(&mut app.side.filter).hint_text("Filter projects and spaces").desired_width(f32::INFINITY));
            // The hint is not an accessible name; give the box one.
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Filter projects and spaces"));
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.side.filter.clear();
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let q = app.side.filter.trim().to_string();
                let mut shown = 0usize;
                if q.is_empty() {
                    ui.add_space(6.0);
                    if nav_item(ui, t, app.view == View::Home, 0.0, "Home").clicked() {
                        app.push(Action::Nav(View::Home));
                    }
                }
                shown += starred(app, ui, t, &q);
                shown += recent(app, ui, t, &q);
                shown += filters(app, ui, t, &q);
                shown += projects(app, ui, t, &q);
                shown += spaces(app, ui, t, &q);
                if !q.is_empty() && shown == 0 {
                    ui.add_space(12.0);
                    ui.label(RichText::new(format!("Nothing matches \u{201c}{q}\u{201d}.")).color(t.muted));
                }
            });
        });
}

/// Where a starred item goes, its label and whether it is the current view; `None` when it no
/// longer exists.
fn star_target(app: &TrackApp, s: &Star) -> Option<(String, Action, bool)> {
    let ws = &app.session.ws;
    Some(match s {
        Star::Project(k) => {
            let key = ProjectKey::parse(k).ok()?;
            let p = ws.projects.get(&key)?;
            let sel = matches!(&app.view, View::Board(p) | View::SprintBoard(p, _) if *p == key);
            (p.name.clone(), Action::Nav(directory::board_view(app, &key)), sel)
        }
        Star::Space(k) => {
            let sp = ws.space(k).ok()?;
            (sp.name.clone(), Action::Nav(View::Page(sp.homepage)), app.view == View::Page(sp.homepage))
        }
        Star::Page(id) => {
            let p = ws.pages.get(&Id(*id))?;
            (p.title.clone(), Action::Nav(View::Page(p.id)), app.view == View::Page(p.id))
        }
        Star::Filter(label) => {
            let (_, jql) = FILTERS.iter().find(|f| f.0 == label)?;
            (label.clone(), Action::Search((*jql).to_string()), app.view == View::Issues && app.jql == *jql)
        }
    })
}

fn starred(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, q: &str) -> usize {
    let items: Vec<(Star, String, Action, bool)> = app
        .side
        .stars()
        .filter_map(|s| star_target(app, s).map(|(l, a, sel)| (s.clone(), l, a, sel)))
        .filter(|(_, l, _, _)| directory::hit_all(q, &[l]))
        .collect();
    if items.is_empty() || !section(app, ui, t, "Starred", None) {
        return items.len();
    }
    for (star, label, action, sel) in &items {
        let (hit, _) = Row { indent: 0.0, selected: *sel, strong: false, label, chevron: None, star: Some((true, label)), plus: false }.show(ui, t);
        if hit.star {
            app.side.set_star(star.clone(), false);
        } else if hit.open {
            app.push(action.clone());
        }
    }
    items.len()
}

fn recent(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, q: &str) -> usize {
    let items: Vec<(View, String)> = app
        .recent
        .iter()
        .filter(|v| **v != app.view)
        .filter_map(|v| directory::view_label(app, v).map(|l| (v.clone(), l)))
        .filter(|(_, l)| directory::hit_all(q, &[l]))
        .take(directory::RECENT)
        .collect();
    if items.is_empty() || !section(app, ui, t, "Recent", None) {
        return items.len();
    }
    for (v, label) in &items {
        let r = ui.horizontal(|ui| {
            match v {
                View::Issue(k) => {
                    let ty = app.session.ws.issues.get(k).map(|i| i.issue_type.clone()).unwrap_or_default();
                    icons::issue_type(ui, &ty, 14.0);
                }
                View::Page(_) | View::PageHistory(_) => {
                    icons::page(ui, t, 14.0);
                }
                _ => {
                    icons::app_mark(ui, t, 14.0);
                }
            }
            ui.add(egui::Button::selectable(false, RichText::new(label)).frame_when_inactive(false).truncate())
        });
        if r.inner.clicked() {
            app.push(Action::Nav(v.clone()));
        }
    }
    items.len()
}

fn filters(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, q: &str) -> usize {
    let items: Vec<&(&str, &str)> = FILTERS.iter().filter(|(l, _)| directory::hit_all(q, &[l])).collect();
    if items.is_empty() || !section(app, ui, t, "Filters", None) {
        return items.len();
    }
    for (label, jql) in &items {
        let star = Star::Filter((*label).to_string());
        let on = app.side.starred(&star);
        let sel = app.view == View::Issues && app.jql == *jql;
        let (hit, _) = Row { indent: 0.0, selected: sel, strong: false, label, chevron: None, star: Some((on, label)), plus: false }.show(ui, t);
        if hit.star {
            app.side.toggle(star);
        } else if hit.open {
            app.push(Action::Search((*jql).to_string()));
        }
    }
    items.len()
}

/// Up to [`SECTION_CAP`] of `all` (already filtered): starred first, then the rest in order,
/// always keeping the current one. Returns the picks and how many were left out.
fn bounded<T: Clone>(all: &[T], starred: impl Fn(&T) -> bool, current: impl Fn(&T) -> bool) -> (Vec<T>, usize) {
    let mut pick: Vec<T> = all.iter().filter(|x| starred(x)).cloned().collect();
    pick.extend(all.iter().filter(|x| !starred(x)).cloned());
    if let Some(i) = pick.iter().position(&current)
        && i >= SECTION_CAP
    {
        let c = pick.remove(i);
        pick.insert(SECTION_CAP - 1, c);
    }
    let hidden = pick.len().saturating_sub(SECTION_CAP);
    pick.truncate(SECTION_CAP);
    (pick, hidden)
}

fn more_link(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, hidden: usize, what: &str, to: View) {
    if hidden == 0 {
        return;
    }
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        if ui.link(RichText::new(format!("{hidden} more {what}\u{2026}")).size(12.0).color(t.muted)).clicked() {
            app.push(Action::Nav(to));
        }
    });
}

fn projects(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, q: &str) -> usize {
    let all: Vec<(ProjectKey, String)> = app
        .session
        .ws
        .projects
        .values()
        .filter(|p| directory::hit_all(q, &[&p.name, p.key.as_str()]))
        .map(|p| (p.key.clone(), p.name.clone()))
        .collect();
    let mut all = all;
    all.sort_by_key(|(_, n)| n.to_lowercase());
    let small = app.session.ws.projects.len() <= SMALL;
    if all.is_empty() && !q.is_empty() {
        return 0;
    }
    if !section(app, ui, t, "Projects", Some(("All projects\u{2026}", View::Projects))) {
        return all.len();
    }
    let cur = current_project(app);
    let side = &app.side;
    let (pick, hidden) = bounded(&all, |(k, _)| side.starred(&Star::Project(k.to_string())), |(k, _)| cur.as_deref() == Some(k.as_str()));
    if all.is_empty() {
        ui.label(RichText::new("No projects yet.").color(t.muted).size(12.0));
    }
    for (pk, name) in pick {
        let key = pk.to_string();
        let current = cur.as_deref() == Some(key.as_str());
        // "-KEY" remembers a current (or small-workspace) project closed by hand.
        let closed = format!("-{key}");
        let open = app.side.open_projects.contains(&key) || ((current || small) && !app.side.open_projects.contains(&closed));
        let star = Star::Project(key.clone());
        let on = app.side.starred(&star);
        let label = format!("{name}  \u{b7}  {key}");
        let (hit, _) =
            Row { indent: 0.0, selected: false, strong: true, label: &label, chevron: Some(open), star: Some((on, &name)), plus: false }.show(ui, t);
        if hit.star {
            app.side.toggle(star);
        } else if hit.chevron {
            if open {
                app.side.open_projects.remove(&key);
                app.side.open_projects.insert(closed);
            } else {
                app.side.open_projects.remove(&closed);
                app.side.open_projects.insert(key.clone());
            }
        } else if hit.open {
            app.side.open_projects.remove(&closed);
            app.side.open_projects.insert(key.clone());
            app.push(Action::Nav(directory::board_view(app, &pk)));
        }
        if !open {
            continue;
        }
        let on_board = matches!(&app.view, View::Board(p) | View::SprintBoard(p, _) if *p == pk);
        if nav_item(ui, t, on_board, 18.0, "Board").clicked() {
            // UI-12: the active sprint's board when one is running.
            app.push(Action::Nav(directory::board_view(app, &pk)));
        }
        // UI-11
        if nav_item(ui, t, app.view == View::Backlog(pk.clone()), 18.0, "Backlog").clicked() {
            app.push(Action::Nav(View::Backlog(pk.clone())));
        }
        let jql = format!("project = {key} ORDER BY created DESC");
        if nav_item(ui, t, app.view == View::Issues && app.jql == jql, 18.0, "Issues").clicked() {
            app.push(Action::Search(jql));
        }
    }
    more_link(app, ui, t, hidden, "projects", View::Projects);
    all.len()
}

/// A space in the sidebar: key, name, home page and (while filtering) its matching pages.
type SpaceItem = (String, String, Id, Vec<(Id, String)>);

fn spaces(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, q: &str) -> usize {
    let ws = &app.session.ws;
    // A space matches by name or key, or when one of its pages does (those pages are listed).
    let mut all: Vec<SpaceItem> = Vec::new();
    for s in ws.spaces.values() {
        let by_name = directory::hit_all(q, &[&s.name, s.key.as_str()]);
        let pages: Vec<(Id, String)> = if q.is_empty() {
            vec![]
        } else {
            ws.pages
                .values()
                .filter(|p| p.space == s.key && p.parent.is_some() && directory::hit_all(q, &[&p.title]))
                .take(SECTION_CAP)
                .map(|p| (p.id, p.title.clone()))
                .collect()
        };
        if by_name || !pages.is_empty() {
            all.push((s.key.to_string(), s.name.clone(), s.homepage, pages));
        }
    }
    all.sort_by_key(|s| s.1.to_lowercase());
    let small = app.session.ws.spaces.len() <= SMALL;
    if all.is_empty() && !q.is_empty() {
        return 0;
    }
    if !section(app, ui, t, "Spaces", Some(("All spaces\u{2026}", View::Spaces))) {
        return all.len();
    }
    let cur = current_space(app);
    let side = &app.side;
    let (pick, hidden) = bounded(&all, |s| side.starred(&Star::Space(s.0.clone())), |s| cur.as_deref() == Some(s.0.as_str()));
    if all.is_empty() {
        ui.label(RichText::new("No spaces yet.").color(t.muted).size(12.0));
    }
    for (key, name, home, pages) in pick {
        let star = Star::Space(key.clone());
        let on = app.side.starred(&star);
        let current = cur.as_deref() == Some(key.as_str());
        // "-KEY" remembers a current, starred (or small-workspace) space closed by hand.
        let closed = format!("-{key}");
        let open = app.side.open_spaces.contains(&key) || ((current || on || small) && !app.side.open_spaces.contains(&closed));
        let row = Row {
            indent: 0.0,
            selected: app.view == View::Page(home),
            strong: true,
            label: &name,
            chevron: Some(open),
            star: Some((on, &name)),
            plus: true,
        };
        let (hit, plus) = row.show(ui, t);
        if plus.is_some_and(|p| p.on_hover_text("Create a page in this space").clicked()) {
            app.push(Action::OpenNewPage { space: key.clone(), parent: None });
        }
        if hit.star {
            app.side.toggle(star);
        } else if hit.chevron {
            if open {
                app.side.open_spaces.remove(&key);
                app.side.open_spaces.insert(closed);
            } else {
                app.side.open_spaces.remove(&closed);
                app.side.open_spaces.insert(key.clone());
            }
        } else if hit.open {
            app.push(Action::Nav(View::Page(home)));
        }
        if !pages.is_empty() {
            // Filtering: the matching pages, flat.
            for (id, title) in pages {
                let sel = matches!(app.view, View::Page(p) | View::PageHistory(p) if p == id);
                if nav_item(ui, t, sel, 18.0, &title).clicked() {
                    app.push(Action::Nav(View::Page(id)));
                }
            }
        } else if open {
            tree(app, ui, t, home, 0);
        }
    }
    more_link(app, ui, t, hidden, "spaces", View::Spaces);
    all.len()
}

fn tree(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, parent: Id, depth: usize) {
    if depth > 24 {
        return;
    }
    let children: Vec<(Id, String, bool)> =
        app.session.ws.children(parent).iter().map(|c| (c.id, c.title.clone(), !app.session.ws.children(c.id).is_empty())).collect();
    for (id, title, has_kids) in children {
        let open = app.expanded.contains(&id);
        let star = Star::Page(id.0);
        let on = app.side.starred(&star);
        let sel = matches!(app.view, View::Page(p) | View::PageHistory(p) if p == id);
        let indent = 6.0 + depth as f32 * 12.0 + if has_kids { 0.0 } else { 14.0 + ui.spacing().item_spacing.x };
        let row =
            Row { indent, selected: sel, strong: false, label: &title, chevron: has_kids.then_some(open), star: Some((on, &title)), plus: false };
        let (hit, _) = row.show(ui, t);
        if hit.star {
            app.side.toggle(star);
        } else if hit.chevron {
            if open {
                app.expanded.remove(&id);
            } else {
                app.expanded.insert(id);
            }
        } else if hit.open {
            app.push(Action::Nav(View::Page(id)));
        }
        if open && has_kids {
            tree(app, ui, t, id, depth + 1);
        }
    }
}

pub(crate) fn home(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let ws = &app.session.ws;
    let me = app.session.user.clone();
    let name = ws.users.get(&me).map(|u| u.display_name.clone()).unwrap_or(me.clone());
    ui.heading(RichText::new(format!("Welcome back, {}", name.split_whitespace().next().unwrap_or(&name))).size(24.0));
    ui.label(
        RichText::new(format!("{} projects · {} issues · {} spaces · {} pages", ws.projects.len(), ws.issues.len(), ws.spaces.len(), ws.pages.len()))
            .color(t.muted),
    );
    ui.add_space(16.0);
    let now = app.session.now();
    let mut mine: Vec<_> = ws
        .issues
        .values()
        .filter(|i| i.assignee.as_deref() == Some(me.as_str()) && ws.status_category(i) != trackcraft_model::StatusCategory::Done)
        .collect();
    mine.sort_by_key(|i| (trackcraft_model::priority_rank(&i.priority).unwrap_or(9), std::cmp::Reverse(i.updated)));
    let mut pages: Vec<_> = ws.pages.values().filter(|p| p.parent.is_some()).collect();
    pages.sort_by_key(|p| std::cmp::Reverse(p.updated));
    let mut actions = Vec::new();
    ui.columns(2, |cols| {
        if let Some(c) = cols.get_mut(0) {
            card(c, t, "Assigned to me", |ui| {
                if mine.is_empty() {
                    empty_state(ui, t, "Nothing assigned to you. Nice.");
                }
                for i in mine.iter().take(10) {
                    ui.horizontal(|ui| {
                        icons::issue_type(ui, &i.issue_type, 16.0);
                        if ui.link(RichText::new(i.key.to_string()).color(t.muted).monospace()).clicked() || ui.link(&i.summary).clicked() {
                            actions.push(Action::Nav(View::Issue(i.key.clone())));
                        }
                    });
                }
            });
        }
        if let Some(c) = cols.get_mut(1) {
            card(c, t, "Recently updated pages", |ui| {
                for p in pages.iter().take(10) {
                    ui.horizontal(|ui| {
                        icons::page(ui, t, 16.0);
                        if ui.link(&p.title).clicked() {
                            actions.push(Action::Nav(View::Page(p.id)));
                        }
                        ui.label(RichText::new(format!("{} · {}", p.space, crate::ago(now, p.updated))).color(t.muted).size(12.0));
                    });
                }
            });
        }
    });
    for a in actions {
        app.push(a);
    }
}
