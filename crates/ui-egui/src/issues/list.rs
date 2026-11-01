//! The JQL issue list: a virtualised table (only the visible rows are laid out), sortable
//! headers that rewrite the query's `ORDER BY`, a column chooser, paging over `search.jql`'s
//! `startAt`/`total`, keyboard selection, and the empty and error states.

use egui::{RichText, Ui};
use egui_extras::TableBuilder;

use crate::theme::Tokens;
use crate::widgets::{Col, RowCx, card, empty_state, issue_row};
use crate::{Action, LIST_PAGE, TrackApp, View};

const ROW_HEIGHT: f32 = 30.0;
const HEADER_HEIGHT: f32 = 26.0;

pub(crate) fn list(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    app.refresh_search();
    let mut mask = columns(ui.ctx());
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Issues").size(22.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| column_chooser(ui, &mut mask));
    });
    set_columns(ui.ctx(), mask);
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
            ui.add_space(6.0);
            egui::Frame::new().fill(t.danger_soft).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(e).color(t.danger));
            });
            return;
        }
    };
    ui.add_space(4.0);
    if keys.is_empty() {
        empty(app, ui, t);
        return;
    }
    pager(app, ui, t, keys.len());
    ui.add_space(6.0);

    let cols: Vec<Col> = Col::ALL.into_iter().filter(|c| c.fixed() || mask & c.bit() != 0).collect();
    let sorted = crate::order::current_sort(&app.jql);
    let sel_row = app.list_sel.as_ref().and_then(|s| keys.iter().position(|k| k == s));
    let scroll = std::mem::take(&mut app.scroll_sel);
    let ws = &app.session.ws;
    let cx = RowCx { t, ws, now: app.session.now() };
    let mut open = None;
    let mut hover = None; // fixes: the row under the pointer gets a Peek button
    let pointer = ui.ctx().pointer_latest_pos();
    let mut sort: Option<&str> = None;
    card(ui, t, "", |ui| {
        ui.visuals_mut().selection.bg_fill = t.accent_soft;
        let mut table = TableBuilder::new(ui)
            .id_salt(("issue-table", mask))
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .min_scrolled_height(0.0)
            .max_scroll_height(f32::INFINITY);
        for c in &cols {
            table = table.column(c.column());
        }
        if scroll && let Some(r) = sel_row {
            table = table.scroll_to_row(r, None);
        }
        table
            .header(HEADER_HEIGHT, |mut header| {
                for c in &cols {
                    header.col(|ui| {
                        if c.header().is_empty() {
                            return;
                        }
                        let on = sorted.as_ref().filter(|(f, _)| f == c.field()).map(|(_, desc)| *desc);
                        let color = if on.is_some() { t.text } else { t.muted };
                        let r = ui.add(egui::Button::new(RichText::new(c.header()).size(12.0).color(color).strong()).frame(false).truncate());
                        if let Some(desc) = on {
                            sort_arrow(ui, t, desc);
                        }
                        let next = if on == Some(false) { "descending" } else { "ascending" };
                        if r.on_hover_text(format!("Sort by {} ({next})", c.header().to_lowercase())).clicked() {
                            sort = Some(c.field());
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, keys.len(), |mut row| {
                    let n = row.index();
                    let Some(issue) = keys.get(n).and_then(|k| ws.issues.get(k)) else { return };
                    row.set_selected(sel_row == Some(n));
                    let r = issue_row(&mut row, &cx, issue, &cols);
                    if r.as_ref().is_some_and(|r| r.clicked()) {
                        open = Some(issue.key.clone());
                    }
                    if let Some(r) = r
                        && pointer.is_some_and(|p| r.rect.contains(p))
                    {
                        hover = Some((r.rect, issue.key.clone()));
                    }
                });
            });
    });
    if let Some((rect, k)) = hover
        && crate::peek::hover_button(ui, rect, &k)
    {
        crate::peek::open(app, k);
    }
    if let Some(field) = sort {
        app.push(Action::Search(crate::order::sort_by(&app.jql, field)));
    }
    if let Some(k) = open {
        app.list_sel = Some(k.clone());
        app.push(Action::Nav(View::Issue(k)));
    }
}

/// "12 345 issues", and when there is more than one page "1–1 000 of 12 345" with Prev/Next.
fn pager(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, shown: usize) {
    let total = app.jql_total.max(shown);
    let start = app.jql_start;
    ui.horizontal(|ui| {
        let noun = if total == 1 { "issue" } else { "issues" };
        if total <= LIST_PAGE && start == 0 {
            ui.label(RichText::new(format!("{} {noun}", group(total))).color(t.muted));
            return;
        }
        let first = start.saturating_add(1);
        let last = start.saturating_add(shown);
        ui.label(RichText::new(range_text(first, last, total)).color(t.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(last < total, egui::Button::new("Next")).on_hover_text("Next page").clicked() {
                app.jql_start = start.saturating_add(LIST_PAGE);
            }
            if ui.add_enabled(start > 0, egui::Button::new("Prev")).on_hover_text("Previous page").clicked() {
                app.jql_start = start.saturating_sub(LIST_PAGE);
            }
        });
    });
}

/// "1–1 000 of 12 345".
pub(crate) fn range_text(first: usize, last: usize, total: usize) -> String {
    format!("{}\u{2013}{} of {}", group(first), group(last), group(total))
}

/// `12345` as "12 345".
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// No matches: say so, and offer to clear the filter (keeping the sort) or create an issue.
fn empty(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let (filter, order) = crate::order::split_order_by(&app.jql);
    let cleared = order.map(|o| format!("ORDER BY {o}")).unwrap_or_default();
    let has_filter = !filter.is_empty();
    let mut action = None;
    card(ui, t, "", |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.label(RichText::new("No issues match this search").size(16.0).strong());
            ui.add_space(4.0);
            empty_state(ui, t, "", if has_filter { "Try a broader query, or clear the filter." } else { "This workspace has no issues yet." }, None);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                // Centre the buttons: measure is overkill here, a fixed indent is enough.
                ui.add_space((ui.available_width() - 220.0).max(0.0) / 2.0);
                if has_filter && ui.button("Clear filter").clicked() {
                    action = Some(Action::Search(cleared.clone()));
                }
                if ui.button("Create issue").clicked() {
                    action = Some(Action::OpenCreate(None));
                }
            });
            ui.add_space(24.0);
        });
    });
    if let Some(a) = action {
        app.push(a);
    }
}

/// The column chooser: a menu of checkboxes for the optional columns.
fn column_chooser(ui: &mut Ui, mask: &mut u32) {
    // Stays open while ticking boxes, so several columns can be changed at once.
    let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::containers::menu::MenuButton::new("Columns").config(config).ui(ui, |ui| {
        for c in Col::ALL.into_iter().filter(|c| !c.fixed()) {
            let mut on = *mask & c.bit() != 0;
            if ui.checkbox(&mut on, c.header()).changed() {
                *mask ^= c.bit();
            }
        }
        ui.separator();
        if ui.button("Reset columns").clicked() {
            *mask = default_columns();
        }
    });
}

fn columns_id() -> egui::Id {
    egui::Id::unique("issue-list-columns")
}

fn default_columns() -> u32 {
    Col::ALL.into_iter().filter(|c| c.default_on()).fold(0, |m, c| m | c.bit())
}

/// The chosen columns (a bit per [`Col`]), kept in egui's persisted memory.
pub(crate) fn columns(ctx: &egui::Context) -> u32 {
    ctx.data_mut(|d| *d.get_persisted_mut_or(columns_id(), default_columns()))
}

fn set_columns(ctx: &egui::Context, mask: u32) {
    ctx.data_mut(|d| d.insert_persisted(columns_id(), mask));
}

/// A small painted triangle (the bundled font has no arrow glyphs): up for ascending.
fn sort_arrow(ui: &mut Ui, t: &Tokens, desc: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    let c = rect.center();
    let (tip, base) = if desc { (c.y + 3.0, c.y - 3.0) } else { (c.y - 3.0, c.y + 3.0) };
    let pts = vec![egui::pos2(c.x, tip), egui::pos2(c.x - 4.0, base), egui::pos2(c.x + 4.0, base)];
    ui.painter().add(egui::Shape::convex_polygon(pts, t.accent, egui::Stroke::NONE));
}

#[cfg(test)]
mod tests {
    #[test]
    fn range_text_groups_thousands() {
        assert_eq!(super::range_text(1, 1000, 12_345), "1\u{2013}1 000 of 12 345");
        assert_eq!(super::group(0), "0");
        assert_eq!(super::group(999), "999");
        assert_eq!(super::group(1_234_567), "1 234 567");
    }
}
