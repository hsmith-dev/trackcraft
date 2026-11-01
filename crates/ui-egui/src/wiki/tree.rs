//! Page tree moves (UI-30): drag and drop in the sidebar tree (onto a row: child; its top or
//! bottom quarter: before or after; onto a space's row: under its home, so pages move between
//! spaces too), and the Copy and Move to… dialogs from a page's More menu.

use egui::{Rect, RichText, Stroke, Ui, pos2};
use serde_json::json;
use trackcraft_core::Id;
use trackcraft_model::Workspace;

use crate::theme::Tokens;
use crate::{Action, ComboBox, Then, TrackApp, View};

/// The drag payload of a sidebar tree row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageDrag(pub Id);

/// Where a page goes relative to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Place {
    #[default]
    Child,
    Before,
    After,
}

impl Place {
    fn params(self, id: Id, target: Id) -> serde_json::Value {
        match self {
            Place::Child => json!({"id": id.0, "parent": target.0}),
            Place::Before => json!({"id": id.0, "before": target.0}),
            Place::After => json!({"id": id.0, "after": target.0}),
        }
    }
}

/// `target` is `page` or inside its subtree (a page can't move into itself).
fn inside(ws: &Workspace, page: Id, target: Id) -> bool {
    target == page || ws.ancestors(target).contains(&page)
}

/// The name of the space `target` is in, when that's not `page`'s space (a cross-space move).
fn other_space(ws: &Workspace, page: Id, target: Id) -> Option<String> {
    let to = &ws.pages.get(&target)?.space;
    if ws.pages.get(&page).map(|p| &p.space) == Some(to) {
        return None;
    }
    Some(ws.spaces.get(to).map_or_else(|| to.to_string(), |s| s.name.clone()))
}

/// The toast after moving `title` next to or under `target`.
fn moved_text(ws: &Workspace, page: Id, target: Id, title: &str) -> String {
    match other_space(ws, page, target) {
        Some(space) => format!("Moved “{title}” to {space}"),
        None => format!("Moved “{title}”"),
    }
}

/// How long a dragged page hovers a collapsed row before it opens.
const OPEN_AFTER: f64 = 0.6;

/// Drop handling for one sidebar tree row: paints the indicator while a page is dragged over
/// `row`, opens a collapsed row after a short hover, and returns the move on release.
pub(crate) fn tree_drop(app: &mut TrackApp, ui: &Ui, t: &Tokens, row: Rect, target: Id, collapsed: bool) -> Option<Action> {
    drop_on(app, ui, t, row, target, collapsed, false)
}

/// Drop handling for a space's row in the sidebar: a page dropped there goes under the space's
/// home page (from another space too). Returns the move on release, and whether a page is being
/// dragged over the row (the sidebar opens a closed space then).
pub(crate) fn space_drop(app: &mut TrackApp, ui: &Ui, t: &Tokens, row: Rect, home: Id) -> (Option<Action>, bool) {
    let over = egui::DragAndDrop::has_payload_of_type::<PageDrag>(ui.ctx()) && ui.ctx().pointer_latest_pos().is_some_and(|p| row.contains(p));
    (drop_on(app, ui, t, row, home, false, true), over)
}

fn drop_on(app: &mut TrackApp, ui: &Ui, t: &Tokens, row: Rect, target: Id, collapsed: bool, child_only: bool) -> Option<Action> {
    let dragged = egui::DragAndDrop::payload::<PageDrag>(ui.ctx())?.0;
    let p = ui.ctx().pointer_latest_pos()?;
    if !row.contains(p) || !ui.clip_rect().contains(p) || inside(&app.session.ws, dragged, target) {
        return None;
    }
    let q = row.height() / 4.0;
    let place = if child_only {
        Place::Child
    } else if p.y < row.top() + q {
        Place::Before
    } else if p.y > row.bottom() - q {
        Place::After
    } else {
        Place::Child
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("tree-drop")));
    match place {
        Place::Child => {
            painter.rect_stroke(row.shrink(1.0), 5.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
        Place::Before | Place::After => {
            let y = if place == Place::Before { row.top() } else { row.bottom() };
            let x0 = row.left() + 6.0;
            painter.line_segment([pos2(x0, y), pos2(row.right() - 4.0, y)], Stroke::new(2.0, t.accent));
            painter.circle_filled(pos2(x0, y), 3.0, t.accent);
        }
    }
    // Hovering the middle of a collapsed row opens it.
    let key = egui::Id::unique(("tree-hover-open", target.0));
    let now = ui.input(|i| i.time);
    if collapsed && place == Place::Child {
        let since = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
        if now - since >= OPEN_AFTER {
            app.expanded.insert(target);
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    } else {
        ui.ctx().data_mut(|d| d.remove::<f64>(key));
    }
    if !ui.input(|i| i.pointer.any_released()) {
        return None;
    }
    egui::DragAndDrop::clear_payload(ui.ctx());
    if place == Place::Child {
        app.expanded.insert(target);
    }
    let ws = &app.session.ws;
    let what = ws.pages.get(&dragged).map(|p| p.title.clone()).unwrap_or_default();
    let text = moved_text(ws, dragged, target, &what);
    Some(Action::Run { id: "page.move", params: place.params(dragged, target), then: Then::Toast(text) })
}

/// One destination row: a page, how deep it is, its title and its breadcrumb.
#[derive(Debug, Clone)]
pub(crate) struct Dest {
    pub id: Id,
    pub depth: usize,
    pub title: String,
    pub crumbs: String,
    pub home: bool,
}

/// Every page in tree order, spaces by name (bounded).
pub(crate) fn destinations(ws: &Workspace) -> Vec<Dest> {
    let mut out = Vec::new();
    let mut spaces: Vec<_> = ws.spaces.values().collect();
    spaces.sort_by_key(|s| s.name.to_lowercase());
    for s in spaces {
        let mut stack = vec![(s.homepage, 0usize, s.name.clone())];
        while let Some((id, depth, crumbs)) = stack.pop() {
            if out.len() >= 2_000 {
                return out;
            }
            let Some(p) = ws.pages.get(&id) else { continue };
            let home = p.parent.is_none();
            let title = if home { format!("{} (space home)", s.name) } else { p.title.clone() };
            out.push(Dest { id, depth, title: title.clone(), crumbs: crumbs.clone(), home });
            if depth < 24 {
                let next = if home { s.name.clone() } else { format!("{crumbs} / {}", p.title) };
                for c in ws.children(id).iter().rev() {
                    stack.push((c.id, depth + 1, next.clone()));
                }
            }
        }
    }
    out
}

/// The Copy page dialog.
#[derive(Debug, Default)]
pub(crate) struct CopyForm {
    pub page: Id,
    pub title: String,
    /// `None` puts the copy beside the original.
    pub parent: Option<Id>,
    pub children: bool,
    pub error: Option<String>,
}

/// The Move to… dialog.
#[derive(Debug, Default)]
pub(crate) struct MoveForm {
    pub page: Id,
    pub filter: String,
    pub target: Option<Id>,
    pub place: Place,
    pub error: Option<String>,
}

fn primary(ui: &mut Ui, t: &Tokens, text: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text).color(t.on_accent).strong()).fill(t.accent)).clicked()
}

/// Draws the copy dialog; returns false when it closed.
pub(crate) fn copy_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens, form: &mut CopyForm) -> bool {
    let dests = destinations(&app.session.ws);
    let kids = app.session.ws.subtree(form.page).len().saturating_sub(1);
    let mut close = false;
    let mut submit = false;
    let modal = egui::Modal::new(egui::Id::unique("copy-page")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading("Copy page");
        ui.add_space(6.0);
        let l = ui.label("Title");
        ui.add(egui::TextEdit::singleline(&mut form.title).desired_width(f32::INFINITY)).labelled_by(l.id);
        ui.add_space(4.0);
        ui.label("Put the copy under");
        let shown = match form.parent.and_then(|p| dests.iter().find(|d| d.id == p)) {
            Some(d) => d.title.clone(),
            None => "Beside the original".to_string(),
        };
        ComboBox::from_id_salt("copy-parent").selected_text(shown).width(420.0).height(320.0).show_ui(ui, |ui| {
            if ui.selectable_label(form.parent.is_none(), "Beside the original").clicked() {
                form.parent = None;
            }
            for d in &dests {
                let text = format!("{}{}", "    ".repeat(d.depth), d.title);
                if ui.selectable_label(form.parent == Some(d.id), text).on_hover_text(&d.crumbs).clicked() {
                    form.parent = Some(d.id);
                }
            }
        });
        ui.add_space(4.0);
        ui.add_enabled(kids > 0, egui::Checkbox::new(&mut form.children, format!("Include child pages ({kids})")));
        if let Some(e) = &form.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            submit = primary(ui, t, "Copy", !form.title.trim().is_empty());
            close |= ui.button("Cancel").clicked();
        });
    });
    close |= modal.should_close();
    if submit {
        let mut p = json!({"id": form.page.0, "title": form.title.trim(), "children": form.children && kids > 0});
        if let Some(parent) = form.parent {
            p["parent"] = json!(parent.0);
        }
        match app.session.run("page.copy", &p) {
            Ok(v) => {
                app.jql_ran = None;
                let n = v["copied"].as_array().map_or(1, Vec::len);
                app.notify_done(if n > 1 { format!("Copied {n} pages") } else { "Page copied".to_string() }, true);
                if let Some(parent) = v["parentId"].as_u64() {
                    app.expanded.insert(Id(parent));
                }
                if let Some(id) = v["id"].as_u64() {
                    app.push(Action::Nav(View::Page(Id(id))));
                }
                return false;
            }
            Err(e) => form.error = Some(e),
        }
    }
    !close
}

/// Draws the Move to… picker; returns false when it closed.
pub(crate) fn move_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens, form: &mut MoveForm) -> bool {
    let ws = &app.session.ws;
    let title = ws.pages.get(&form.page).map(|p| p.title.clone()).unwrap_or_default();
    // Every space is offered: across spaces the subtree, comments and history go along.
    let q = form.filter.to_lowercase();
    let dests: Vec<Dest> = destinations(ws)
        .into_iter()
        .filter(|d| !inside(ws, form.page, d.id))
        .filter(|d| q.is_empty() || d.title.to_lowercase().contains(&q) || d.crumbs.to_lowercase().contains(&q))
        .take(200)
        .collect();
    let mut close = false;
    let mut submit = false;
    let modal = egui::Modal::new(egui::Id::unique("move-page")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.heading("Move page");
        ui.label(RichText::new(format!("Move “{title}” and its child pages, in this space or another")).color(t.muted));
        ui.add_space(6.0);
        let l = ui.label("Find a page");
        let r = ui.add(egui::TextEdit::singleline(&mut form.filter).hint_text("Type to filter").desired_width(f32::INFINITY)).labelled_by(l.id);
        if form.filter.is_empty() && form.target.is_none() && ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
            for d in &dests {
                let sel = form.target == Some(d.id);
                let w = ui.available_width();
                let r = ui.add(egui::Button::selectable(sel, RichText::new(&d.title).strong()).min_size(egui::vec2(w, 22.0)).truncate());
                if r.clicked() {
                    form.target = Some(d.id);
                    if d.home {
                        form.place = Place::Child;
                    }
                }
                if !d.home {
                    ui.label(RichText::new(&d.crumbs).color(t.muted).size(11.0));
                }
            }
            if dests.is_empty() {
                ui.label(RichText::new("No pages match.").color(t.muted));
            }
        });
        ui.add_space(6.0);
        let home = form.target.and_then(|id| dests.iter().find(|d| d.id == id)).is_some_and(|d| d.home);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut form.place, Place::Child, "As a child");
            ui.add_enabled_ui(!home, |ui| {
                ui.selectable_value(&mut form.place, Place::Before, "Before");
                ui.selectable_value(&mut form.place, Place::After, "After");
            });
        });
        if let Some(e) = &form.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            submit = primary(ui, t, "Move", form.target.is_some());
            close |= ui.button("Cancel").clicked();
        });
    });
    close |= modal.should_close();
    if submit && let Some(target) = form.target {
        let text = moved_text(&app.session.ws, form.page, target, &title);
        match app.session.run("page.move", &form.place.params(form.page, target)) {
            Ok(v) => {
                app.jql_ran = None;
                if let Some(parent) = v["parentId"].as_u64() {
                    app.expanded.insert(Id(parent));
                }
                app.notify_done(text, true);
                return false;
            }
            Err(e) => form.error = Some(e),
        }
    }
    !close
}
