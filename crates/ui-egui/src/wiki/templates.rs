//! Page templates (UI-29): the picker in the new-page dialog, "Save as template" on a page and
//! the per-space template list.

use egui::{RichText, Ui};
use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_markup::Doc;

use crate::theme::Tokens;
use crate::{TrackApp, markup_view};

/// `template.list` for a space, cached until the workspace changes.
#[derive(Debug, Default)]
pub(crate) struct TemplateCache {
    key: Option<(String, u64)>,
    list: Vec<Value>,
}

/// Built-in, workspace and `space`'s templates.
pub(crate) fn list(app: &mut TrackApp, space: &str) -> Vec<Value> {
    let key = (space.to_string(), app.session.revision());
    if app.wiki.templates.key.as_ref() != Some(&key) {
        let v = app.session.run("template.list", &json!({"space": space}));
        app.wiki.templates = TemplateCache { key: Some(key), list: v.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default() };
    }
    app.wiki.templates.list.clone()
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A template's display name, with where it comes from.
fn origin(t: &Value) -> &'static str {
    if t["builtin"].as_bool().unwrap_or(false) {
        "Built-in"
    } else if t["space"].is_null() {
        "Workspace"
    } else {
        "This space"
    }
}

/// The template column of the new-page dialog: Blank plus every template, then a preview of
/// the chosen one. `chosen` is the template id (`None` is Blank).
pub(crate) fn picker(ui: &mut Ui, t: &Tokens, list: &[Value], chosen: &mut Option<Value>, today: &str) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(230.0);
            ui.label(RichText::new("Template").color(t.muted).size(12.0).strong());
            egui::ScrollArea::vertical().id_salt("template-list").max_height(360.0).show(ui, |ui| {
                let row = |ui: &mut Ui, sel: bool, name: &str, desc: &str| -> bool {
                    let text = RichText::new(name).strong().color(if sel { t.accent } else { t.text });
                    let r = ui.add(egui::Button::selectable(sel, text).min_size(egui::vec2(220.0, 22.0)).truncate());
                    if !desc.is_empty() {
                        ui.add(egui::Label::new(RichText::new(desc).color(t.muted).size(11.5)).wrap());
                    }
                    ui.add_space(4.0);
                    r.clicked()
                };
                if row(ui, chosen.is_none(), "Blank", "An empty page.") {
                    *chosen = None;
                }
                for tpl in list {
                    let id = tpl["id"].clone();
                    let sel = chosen.as_ref() == Some(&id);
                    let desc = s(tpl, "description");
                    if row(ui, sel, &s(tpl, "name"), &format!("{} · {desc}", origin(tpl))) {
                        *chosen = Some(id);
                    }
                }
            });
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Preview").color(t.muted).size(12.0).strong());
            let body =
                chosen.as_ref().and_then(|c| list.iter().find(|x| &x["id"] == c)).map(|x| s(x, "body").replace("{date}", today)).unwrap_or_default();
            egui::ScrollArea::vertical().id_salt("template-preview").max_height(360.0).show(ui, |ui| {
                if body.is_empty() {
                    ui.label(RichText::new("Start from an empty page.").color(t.muted).italics());
                } else {
                    markup_view::show(ui, t, &Doc::from_markdown(&body));
                }
            });
        });
    });
}

/// The suggested title of the chosen template (`{date}` is filled in by the engine).
pub(crate) fn suggested_title(list: &[Value], chosen: Option<&Value>) -> String {
    chosen.and_then(|c| list.iter().find(|x| &x["id"] == c)).map(|x| s(x, "title")).unwrap_or_default()
}

/// "Save as template" for a page.
#[derive(Debug, Default)]
pub(crate) struct SaveForm {
    pub page: Id,
    pub space: String,
    pub name: String,
    pub description: String,
    /// Offer it in every space, not only this one.
    pub everywhere: bool,
    pub error: Option<String>,
}

/// Draws the save-as-template dialog; returns false when it closed.
pub(crate) fn save_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens, form: &mut SaveForm) -> bool {
    let mut close = false;
    let mut submit = false;
    let space_name = app.session.ws.space(&form.space).map(|s| s.name.clone()).unwrap_or_default();
    let modal = egui::Modal::new(egui::Id::unique("save-template")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading("Save as template");
        ui.label(RichText::new("New pages can start from this page's text.").color(t.muted));
        ui.add_space(8.0);
        let l = ui.label("Name");
        let r = ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY)).labelled_by(l.id);
        if form.name.is_empty() && ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        let l = ui.label("Description (optional)");
        ui.add(egui::TextEdit::singleline(&mut form.description).desired_width(f32::INFINITY)).labelled_by(l.id);
        ui.checkbox(&mut form.everywhere, "Offer in every space");
        if !form.everywhere {
            ui.label(RichText::new(format!("Only in {space_name}")).color(t.muted).size(12.0));
        }
        if let Some(e) = &form.error {
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let ok = !form.name.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(RichText::new("Save template").color(t.on_accent).strong()).fill(t.accent)).clicked() {
                submit = true;
            }
            close |= ui.button("Cancel").clicked();
        });
    });
    close |= modal.should_close();
    if submit {
        let body = app.session.ws.page(form.page).map(|p| p.body.to_markdown()).unwrap_or_default();
        let mut p = json!({"name": form.name.trim(), "body": body, "description": form.description.trim()});
        if !form.everywhere {
            p["space"] = json!(form.space);
        }
        match app.session.run("template.create", &p) {
            Ok(_) => {
                app.notify_done(format!("Saved template “{}”", form.name.trim()), true);
                return false;
            }
            Err(e) => form.error = Some(e),
        }
    }
    !close
}

/// Draws a space's template list (built-ins and saved ones, which can be deleted); returns
/// false when it closed.
pub(crate) fn manage_dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens, space: &str) -> bool {
    let list = list(app, space);
    let space_name = app.session.ws.space(space).map(|s| s.name.clone()).unwrap_or_default();
    let mut close = false;
    let mut delete = None;
    let modal = egui::Modal::new(egui::Id::unique("space-templates")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.heading(format!("Templates in {space_name}"));
        ui.label(RichText::new("Save a page as a template from its More menu.").color(t.muted));
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
            for tpl in &list {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(360.0);
                        ui.label(RichText::new(s(tpl, "name")).strong());
                        let desc = s(tpl, "description");
                        ui.label(
                            RichText::new(format!("{}{}", origin(tpl), if desc.is_empty() { String::new() } else { format!(" · {desc}") }))
                                .color(t.muted)
                                .size(12.0),
                        );
                    });
                    if !tpl["builtin"].as_bool().unwrap_or(true) {
                        let b = ui.small_button(RichText::new("Delete").color(t.danger));
                        let name = format!("Delete template {}", s(tpl, "name"));
                        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
                        if b.clicked() {
                            delete = tpl["id"].as_u64();
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });
        ui.add_space(6.0);
        close |= ui.button("Close").clicked();
    });
    close |= modal.should_close();
    if let Some(id) = delete {
        match app.session.run("template.delete", &json!({"id": id})) {
            Ok(_) => app.notify_done("Template deleted", true),
            Err(e) => app.notify_error(e),
        }
    }
    !close
}
