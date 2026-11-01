//! Wiki views: page view and editor (with conflict handling), version history, new page.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::Id;
use trackcraft_markup::Doc;

use crate::theme::Tokens;
use crate::widgets::{card, empty_state};
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
    let mut go = None;
    ui.horizontal_wrapped(|ui| {
        if ui.link(RichText::new(&space_name).color(t.muted)).clicked() {
            go = home;
        }
        for (pid, title) in chain {
            ui.label(RichText::new("/").color(t.muted));
            if ui.link(RichText::new(title).color(t.muted)).clicked() {
                go = Some(pid);
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
    ui.horizontal(|ui| {
        ui.heading(RichText::new(&p.title).size(26.0).strong());
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
        });
    });
    ui.horizontal(|ui| {
        icons::avatar(ui, t, &p.author, &author, 18.0);
        ui.label(RichText::new(format!("v{} · {author} · {}", p.version, crate::ago(now, p.updated))).color(t.muted).size(12.0));
        for l in &p.labels {
            ui.label(RichText::new(l).size(11.0).background_color(t.surface_alt));
        }
    });
    ui.add_space(12.0);
    // `e` edits the page: see `keys::view_keys` (ignored while typing or behind a dialog).
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(820.0);
        if is_home && p.body.is_empty() {
            empty_state(ui, t, "This is the space's home page.");
        } else {
            markup_view::show(ui, t, &p.body);
        }
        let kids: Vec<(Id, String)> = app.session.ws.children(id).iter().map(|c| (c.id, c.title.clone())).collect();
        if !kids.is_empty() {
            ui.add_space(18.0);
            card(ui, t, "Child pages", |ui| {
                for (cid, title) in kids {
                    ui.horizontal(|ui| {
                        icons::page(ui, t, 16.0);
                        if ui.link(title).clicked() {
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
        ui.heading(RichText::new(format!("History of “{}”", p.title)).size(22.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Back to page").clicked() {
                app.push(Action::Nav(View::Page(id)));
            }
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
                card(ui, t, &format!("v{v}: {title}"), |ui| markup_view::show(ui, t, body));
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
