//! Wiki views: page view and editor (with conflict handling), version history, new page.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::Id;
use trackcraft_markup::Doc;

use crate::theme::Tokens;
use crate::widgets::{card, empty_state, link_truncated};

/// Breadcrumbs shown before the middle collapses to "…" (UI-6: 60-deep trees).
const CRUMBS: usize = 4;
/// Widest a single breadcrumb gets before it truncates.
const CRUMB_W: f32 = 220.0;
/// Blocks per lazily drawn chunk of a page body.
const CHUNK: usize = 40;
/// Bodies with more blocks than this draw only the chunks in view (UI-6: 5 000-line pages).
const LAZY_BLOCKS: usize = 200;

/// Draws `doc` inside a `ScrollArea::show_viewport` closure, laying out only the chunks of
/// [`CHUNK`] blocks that are in `viewport`; the rest take their measured (or estimated) height.
/// Heights are remembered per `salt` and width, so scrolling back is exact.
fn lazy_doc(ui: &mut Ui, t: &Tokens, doc: &Doc, salt: (u64, u32), viewport: egui::Rect) {
    if doc.blocks.len() <= LAZY_BLOCKS {
        markup_view::show(ui, t, doc);
        return;
    }
    let origin = ui.min_rect().top();
    let width = ui.available_width().round() as i32;
    for (n, chunk) in doc.blocks.chunks(CHUNK).enumerate() {
        let id = egui::Id::unique(("page-chunk", salt, width, n));
        let y = ui.cursor().top() - origin;
        let known = ui.data(|d| d.get_temp::<f32>(id));
        let (h, measured) = match known {
            Some(h) => (h, true),
            None => (CHUNK as f32 * 26.0, false),
        };
        let visible = y <= viewport.bottom() && y + h >= viewport.top();
        // Unmeasured chunks above the view are drawn once, so offsets above it are exact.
        if visible || (!measured && y < viewport.top()) {
            let part = Doc { blocks: chunk.to_vec() };
            let r = ui.scope(|ui| markup_view::show(ui, t, &part)).response;
            ui.data_mut(|d| d.insert_temp(id, r.rect.height()));
        } else {
            ui.allocate_space(egui::vec2(ui.available_width(), h));
        }
    }
}
use crate::{Action, NewPageForm, Then, TrackApp, View, icons, markup_view};

fn breadcrumbs(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, id: Id) {
    let ws = &app.session.ws;
    let Ok(page) = ws.page(id) else { return };
    let space_name = ws.spaces.get(&page.space).map(|s| s.name.clone()).unwrap_or_default();
    let mut chain: Vec<(Id, String)> = ws.ancestors(id).iter().rev().filter_map(|a| ws.pages.get(a).map(|p| (p.id, p.title.clone()))).collect();
    // The space home page is shown as the space itself.
    if !chain.is_empty() {
        chain.remove(0);
    }
    let home = ws.spaces.get(&page.space).map(|s| s.homepage);
    // UI-6: a deep page shows the first crumb, "…", and the last few.
    let hidden = chain.len().saturating_sub(CRUMBS);
    let mut go = None;
    ui.horizontal_wrapped(|ui| {
        let crumb = |ui: &mut Ui, text: &str| -> bool {
            ui.allocate_ui(egui::vec2(CRUMB_W, ui.spacing().interact_size.y), |ui| link_truncated(ui, RichText::new(text).color(t.muted)).clicked())
                .inner
        };
        if crumb(ui, &space_name) {
            go = home;
        }
        for (n, (pid, title)) in chain.iter().enumerate() {
            if hidden > 0 && n > 0 && n <= hidden {
                if n == 1 {
                    ui.label(RichText::new("/").color(t.muted));
                    ui.label(RichText::new("…").color(t.muted)).on_hover_text(format!("{hidden} more levels"));
                }
                continue;
            }
            ui.label(RichText::new("/").color(t.muted));
            if crumb(ui, title) {
                go = Some(*pid);
            }
        }
    });
    if let Some(g) = go {
        app.push(Action::Nav(View::Page(g)));
    }
}

pub(crate) fn page(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, id: Id) {
    let Ok(p) = app.session.ws.page(id).cloned() else {
        ui.label("This page no longer exists.");
        return;
    };
    breadcrumbs(app, ui, t, id);
    if app.page_edit.as_ref().is_some_and(|e| e.id == id) {
        editor(app, ui, t);
        return;
    }
    let now = app.session.now();
    let author = app.session.ws.users.get(&p.author).map(|u| u.display_name.clone()).unwrap_or_else(|| p.author.clone());
    let is_home = p.parent.is_none();
    // Buttons first (right to left), so a 255-character title truncates instead (UI-6).
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !is_home && ui.small_button("Delete").on_hover_text("Delete this page and its children").clicked() {
                app.push(Action::Run {
                    id: "page.delete",
                    params: json!({"id": id.0}),
                    then: Then::Toast(format!("Page deleted ({} to undo)", crate::key("Mod+Z"))),
                });
            }
            if ui.small_button("History").clicked() {
                app.push(Action::Nav(View::PageHistory(id)));
            }
            if ui.small_button("+ Child page").clicked() {
                app.push(Action::OpenNewPage { space: p.space.to_string(), parent: Some(id) });
            }
            if ui
                .add(egui::Button::new(RichText::new("Edit").color(t.on_accent).strong()).fill(t.accent))
                .on_hover_text("Edit this page (E)")
                .clicked()
            {
                app.push(Action::EditPage(id));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(&p.title).size(26.0).strong()).truncate());
            });
        });
    });
    ui.horizontal_wrapped(|ui| {
        crate::widgets::who(ui, t, &app.session.ws, Some(&p.author), 18.0);
        ui.label(RichText::new(format!("v{} · {author} · {}", p.version, crate::ago(now, p.updated))).color(t.muted).size(12.0));
        for l in &p.labels {
            ui.label(RichText::new(l).size(11.0).background_color(t.surface_alt));
        }
    });
    ui.add_space(12.0);
    // `e` edits the page, `c` adds a child, `l` edits labels: see `keys.rs`.
    let kids: Vec<(Id, String)> = app.session.ws.children(id).iter().map(|c| (c.id, c.title.clone())).collect();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        ui.set_max_width(820.0);
        // UI-6: empty pages say what to do.
        if p.body.is_empty() {
            if is_home && kids.is_empty() {
                if empty_state(
                    ui,
                    t,
                    "This space has no pages yet",
                    "Write the first page; it appears in the sidebar under the space.",
                    Some("Create page"),
                ) {
                    app.push(Action::OpenNewPage { space: p.space.to_string(), parent: None });
                }
            } else if is_home {
                empty_state(ui, t, "", "This is the space's home page. Its pages are listed below.", None);
            } else if empty_state(ui, t, "This page is empty", "Edit it to add text (E).", Some("Edit page")) {
                app.push(Action::EditPage(id));
            }
        } else {
            lazy_doc(ui, t, &p.body, (id.0, p.version), viewport);
        }
        if !kids.is_empty() {
            ui.add_space(18.0);
            card(ui, t, "Child pages", |ui| {
                for (cid, title) in kids {
                    ui.horizontal(|ui| {
                        icons::page(ui, t, 16.0);
                        if link_truncated(ui, title).clicked() {
                            app.push(Action::Nav(View::Page(cid)));
                        }
                    });
                }
            });
        }
    });
}

fn editor(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let Some(ed) = app.page_edit.as_mut() else { return };
    let id = ed.id;
    let mut acts: Vec<Action> = Vec::new();
    ui.add(egui::TextEdit::singleline(&mut ed.title).font(egui::FontId::proportional(24.0)).desired_width(f32::INFINITY).hint_text("Page title"));
    ui.add_space(6.0);
    if let Some(theirs) = ed.conflict {
        egui::Frame::new().fill(t.danger_soft).corner_radius(6).inner_margin(10).show(ui, |ui| {
            ui.label(RichText::new(format!("Someone saved version {theirs} while you were editing version {}.", ed.base)).color(t.danger).strong());
            ui.horizontal(|ui| {
                if ui.button("Save mine anyway").on_hover_text("Your text becomes the newest version; theirs stays in the history").clicked() {
                    let p = json!({"id": id.0, "title": ed.title, "body": ed.body, "message": ed.message});
                    acts.push(Action::Run { id: "page.edit", params: p, then: Then::EndPageEdit });
                }
                if ui.button("Discard mine and reload").clicked() {
                    acts.push(Action::EditPage(id));
                }
            });
        });
        ui.add_space(6.0);
    }
    let h = (ui.available_height() - 90.0).max(200.0);
    let avail = ui.available_width();
    let mut save = false;
    let mut cancel = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(avail * 0.5 - 8.0);
            ui.label(RichText::new("Markdown").color(t.muted).size(12.0));
            egui::ScrollArea::vertical().id_salt("page-src").max_height(h).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut ed.body).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(24));
            });
        });
        ui.vertical(|ui| {
            ui.set_width(avail * 0.5 - 8.0);
            ui.label(RichText::new("Preview").color(t.muted).size(12.0));
            egui::ScrollArea::vertical().id_salt("page-preview").max_height(h).show(ui, |ui| {
                card(ui, t, "", |ui| markup_view::show(ui, t, &Doc::from_markdown(&ed.body)));
            });
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ed.message).hint_text("What changed? (optional)").desired_width(300.0));
        if ui.add(egui::Button::new(RichText::new("Publish").color(t.on_accent).strong()).fill(t.accent)).clicked() {
            save = true;
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    if save {
        let p = json!({"id": id.0, "title": ed.title, "body": ed.body, "baseVersion": ed.base, "message": ed.message});
        acts.push(Action::Run { id: "page.edit", params: p, then: Then::EndPageEdit });
    }
    if cancel {
        app.page_edit = None;
    }
    for a in acts {
        app.push(a);
    }
}

pub(crate) fn history(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, id: Id) {
    let Ok(p) = app.session.ws.page(id).cloned() else {
        ui.label("This page no longer exists.");
        return;
    };
    breadcrumbs(app, ui, t, id);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Back to page").clicked() {
                app.push(Action::Nav(View::Page(id)));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(format!("History of “{}”", p.title)).size(22.0).strong()).truncate());
            });
        });
    });
    ui.add_space(8.0);
    let now = app.session.now();
    let ws = &app.session.ws;
    let name = |u: &str| ws.users.get(u).map(|x| x.display_name.clone()).unwrap_or_else(|| u.to_string());
    let mut rows: Vec<(u32, String, String, trackcraft_model::Timestamp, String, Doc)> =
        p.history.iter().map(|v| (v.version, v.title.clone(), v.author.clone(), v.at, v.message.clone(), v.body.clone())).collect();
    rows.push((p.version, p.title.clone(), p.author.clone(), p.updated, p.message.clone(), p.body.clone()));
    rows.reverse();
    let avail = ui.available_width();
    let mut acts = Vec::new();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(avail * 0.45);
            card(ui, t, "", |ui| {
                egui::Grid::new("versions").num_columns(4).spacing([12.0, 8.0]).striped(true).show(ui, |ui| {
                    for (v, _title, author, at, msg, _) in &rows {
                        let current = *v == p.version;
                        let sel = app.history_preview == Some(*v) || (app.history_preview.is_none() && current);
                        if ui.selectable_label(sel, format!("v{v}{}", if current { " (current)" } else { "" })).clicked() {
                            app.history_preview = Some(*v);
                        }
                        ui.label(format!("{} · {}", name(author), crate::ago(now, *at)));
                        ui.label(RichText::new(msg).color(t.muted).italics());
                        if !current && ui.small_button("Restore").clicked() {
                            acts.push(Action::Run {
                                id: "page.restore",
                                params: json!({"id": id.0, "version": v}),
                                then: Then::Toast(format!("Restored version {v} as a new version")),
                            });
                        }
                        ui.end_row();
                    }
                });
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            let want = app.history_preview.unwrap_or(p.version);
            if let Some((v, title, _, _, _, body)) = rows.iter().find(|r| r.0 == want) {
                // UI-6: a huge version scrolls and draws lazily.
                egui::ScrollArea::vertical().id_salt("history-preview").auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
                    card(ui, t, "", |ui| {
                        ui.add(egui::Label::new(RichText::new(format!("v{v}: {title}")).strong().size(15.0)).truncate());
                        ui.add_space(6.0);
                        lazy_doc(ui, t, body, (id.0, *v), viewport);
                    });
                });
            }
        });
    });
    for a in acts {
        app.push(a);
    }
}

pub(crate) fn new_page_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut form) = app.new_page.take() else { return };
    let mut close = false;
    let mut submit = false;
    let parent_title = form.parent.and_then(|p| app.session.ws.pages.get(&p)).map(|p| p.title.clone());
    let space_name = app.session.ws.space(&form.space).map(|s| s.name.clone()).unwrap_or_default();
    let modal = egui::Modal::new(egui::Id::unique("new-page")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading("New page");
        ui.label(
            RichText::new(match &parent_title {
                Some(pt) => format!("In {space_name}, under “{pt}”"),
                None => format!("In {space_name}"),
            })
            .color(t.muted),
        );
        ui.add_space(8.0);
        let r = ui.add(egui::TextEdit::singleline(&mut form.title).hint_text("Title").desired_width(f32::INFINITY));
        if form.title.is_empty() && ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        if let Some(e) = &form.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Create").color(t.on_accent).strong()).fill(t.accent)).clicked() {
                submit = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if modal.should_close() {
        close = true;
    }
    if submit {
        let mut p = json!({"space": form.space, "title": form.title});
        if let Some(parent) = form.parent {
            p["parent"] = json!(parent.0);
        }
        match app.session.run("page.create", &p) {
            Ok(v) => {
                if let Some(n) = v["id"].as_u64() {
                    let id = Id(n);
                    if let Some(parent) = form.parent {
                        app.expanded.insert(parent);
                    }
                    // Both wait behind the unsaved-edit guard if another page has typed text.
                    app.push(Action::Nav(View::Page(id)));
                    app.push(Action::EditPage(id));
                }
                return;
            }
            Err(e) => form.error = Some(e),
        }
    }
    if !close {
        app.new_page = Some(NewPageForm { ..form });
    }
}
