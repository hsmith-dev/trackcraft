//! The JQL issue list with sortable headers and keyboard selection.

use egui::{RichText, Ui};

use crate::theme::Tokens;
use crate::widgets::{card, name_of, pill, who};
use crate::{Action, TrackApp, View, icons};
use trackcraft_model::priority_rank;

pub(crate) fn list(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    app.refresh_search();
    ui.heading(RichText::new("Issues").size(22.0));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mut q = app.jql.clone();
        let r = ui.add(
            egui::TextEdit::singleline(&mut q)
                .font(egui::TextStyle::Monospace)
                .desired_width(ui.available_width() - 90.0)
                .hint_text("project = LUME AND statusCategory != Done"),
        );
        if r.changed() {
            app.jql = q;
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Search").clicked() || enter {
            app.jql_ran = None;
        }
    });
    let keys = match &app.jql_result {
        Ok(k) => k.clone(),
        Err(e) => {
            egui::Frame::new().fill(t.danger_soft).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.label(RichText::new(e).color(t.danger));
            });
            return;
        }
    };
    ui.label(RichText::new(format!("{} issue{}", keys.len(), if keys.len() == 1 { "" } else { "s" })).color(t.muted));
    ui.add_space(6.0);
    let ws = &app.session.ws;
    let now = app.session.now();
    let mut open = None;
    let mut sort: Option<&str> = None;
    let sorted = crate::order::current_sort(&app.jql);
    // Grid row 0 is the header.
    let sel_row = app.list_sel.as_ref().and_then(|s| keys.iter().position(|k| k == s)).map(|n| n.saturating_add(1));
    let scroll = std::mem::take(&mut app.scroll_sel);
    let sel_fill = t.accent_soft;
    card(ui, t, "", |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            let grid = egui::Grid::new("issue-grid").num_columns(7).spacing([14.0, 8.0]).with_row_color(move |row, style| {
                if Some(row) == sel_row {
                    Some(sel_fill)
                } else if row % 2 == 1 {
                    Some(style.visuals.faint_bg_color)
                } else {
                    None
                }
            });
            grid.show(ui, |ui| {
                ui.label("");
                for (h, field) in LIST_COLUMNS {
                    let on = sorted.as_ref().filter(|(f, _)| f == field).map(|(_, desc)| *desc);
                    let r = ui
                        .horizontal(|ui| {
                            let color = if on.is_some() { t.text } else { t.muted };
                            let r = ui.add(egui::Button::new(RichText::new(h).size(12.0).color(color).strong()).frame(false));
                            if let Some(desc) = on {
                                sort_arrow(ui, t, desc);
                            }
                            r
                        })
                        .inner;
                    let next = if on == Some(false) { "descending" } else { "ascending" };
                    if r.on_hover_text(format!("Sort by {} ({next})", h.to_lowercase())).clicked() {
                        sort = Some(field);
                    }
                }
                ui.end_row();
                for k in &keys {
                    let Some(i) = ws.issues.get(k) else { continue };
                    let mut row = icons::issue_type(ui, &i.issue_type, 16.0).rect;
                    let r = ui.link(RichText::new(i.key.to_string()).monospace());
                    row = row.union(r.rect);
                    let mut clicked = r.clicked();
                    let summary: String = if i.summary.chars().count() > 70 {
                        format!("{}…", i.summary.chars().take(69).collect::<String>())
                    } else {
                        i.summary.clone()
                    };
                    let r = ui.add(egui::Label::new(summary).sense(egui::Sense::click())).on_hover_text(&i.summary);
                    row = row.union(r.rect);
                    clicked |= r.clicked();
                    row = row.union(pill(ui, t, ws.status_category(i), &i.status).rect);
                    let r = ui.horizontal(|ui| {
                        icons::priority(ui, t, &i.priority, priority_rank(&i.priority).unwrap_or(3), 14.0);
                        ui.label(&i.priority);
                    });
                    row = row.union(r.response.rect);
                    let r = ui.horizontal(|ui| {
                        who(ui, t, ws, i.assignee.as_deref(), 18.0);
                        ui.label(name_of(ws, i.assignee.as_deref()));
                    });
                    row = row.union(r.response.rect);
                    row = row.union(ui.label(RichText::new(crate::ago(now, i.updated)).color(t.muted)).rect);
                    // The rest of the row (pill, priority, assignee, the gaps) opens the issue too.
                    let key = i.key.to_string();
                    let whole = ui.interact(row, egui::Id::unique(("issue-row", &key)), egui::Sense::click());
                    whole.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Open {key}")));
                    if whole.clicked() || clicked {
                        open = Some(i.key.clone());
                    }
                    if scroll && app.list_sel.as_ref() == Some(&i.key) {
                        ui.scroll_to_rect(row, None);
                    }
                    ui.end_row();
                }
            });
        });
    });
    if let Some(field) = sort {
        app.push(Action::Search(crate::order::sort_by(&app.jql, field)));
    }
    if let Some(k) = open {
        app.list_sel = Some(k.clone());
        app.push(Action::Nav(View::Issue(k)));
    }
}

/// Sortable list columns: (header, JQL field).
const LIST_COLUMNS: [(&str, &str); 6] =
    [("Key", "key"), ("Summary", "summary"), ("Status", "status"), ("Priority", "priority"), ("Assignee", "assignee"), ("Updated", "updated")];

/// A small painted triangle (the bundled font has no arrow glyphs): up for ascending.
fn sort_arrow(ui: &mut Ui, t: &Tokens, desc: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    let c = rect.center();
    let (tip, base) = if desc { (c.y + 3.0, c.y - 3.0) } else { (c.y - 3.0, c.y + 3.0) };
    let pts = vec![egui::pos2(c.x, tip), egui::pos2(c.x - 4.0, base), egui::pos2(c.x + 4.0, base)];
    ui.painter().add(egui::Shape::convex_polygon(pts, t.accent, egui::Stroke::NONE));
}
