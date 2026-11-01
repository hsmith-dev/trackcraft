//! Page restrictions (UI-37): the Restrictions… dialog (`page.restrict`) and the lock badge on
//! restricted pages (sidebar tree and page header).
//!
//! View limits apply to child pages too, so the dialog says when a parent page already limits
//! who can view this one. Edit limits apply to the page alone.

use egui::{RichText, Ui};
use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_model::Workspace;

use crate::theme::Tokens;
use crate::widgets::{name_of, person_adder, remove_button};
use crate::{TrackApp, icons};

/// People on one list at most (the engine's cap).
const MAX_LIST: usize = 1000;

/// The open Restrictions dialog.
#[derive(Debug, Default)]
pub(crate) struct RestrictForm {
    pub page: Id,
    /// "Only these people can view" (else anyone in the space).
    pub view_only: bool,
    pub view: Vec<String>,
    /// "Only these people can edit" (else anyone who can edit in the space).
    pub edit_only: bool,
    pub edit: Vec<String>,
    pub error: Option<String>,
}

impl RestrictForm {
    /// The page's current restrictions.
    pub(crate) fn open(ws: &Workspace, page: Id) -> Self {
        let r = ws.access.pages.get(&page).cloned().unwrap_or_default();
        RestrictForm {
            page,
            view_only: r.view.is_some(),
            view: r.view.unwrap_or_default(),
            edit_only: r.edit.is_some(),
            edit: r.edit.unwrap_or_default(),
            error: None,
        }
    }
}

/// The page has a view or edit restriction of its own.
pub(crate) fn restricted(ws: &Workspace, id: Id) -> bool {
    ws.access.pages.get(&id).is_some_and(|r| r.view.is_some() || r.edit.is_some())
}

/// The nearest ancestor that limits who can view (its title), if any.
fn inherited(ws: &Workspace, id: Id) -> Option<String> {
    ws.ancestors(id)
        .into_iter()
        .find(|a| ws.access.pages.get(a).is_some_and(|r| r.view.is_some()))
        .and_then(|a| ws.pages.get(&a))
        .map(|p| p.title.clone())
}

/// A lock badge with a tooltip saying what is limited (the page header).
pub(crate) fn badge(ui: &mut Ui, t: &Tokens, ws: &Workspace, id: Id) {
    let own = ws.access.pages.get(&id).cloned().unwrap_or_default();
    let parent = inherited(ws, id);
    let tip = match (own.view.is_some(), own.edit.is_some(), &parent) {
        (true, true, _) => "Only some people can view or edit this page".to_string(),
        (true, false, _) => "Only some people can view this page".to_string(),
        (false, true, None) => "Only some people can edit this page".to_string(),
        (false, true, Some(p)) => format!("Only some people can edit this page, and \u{201c}{p}\u{201d} limits who can view it"),
        (false, false, Some(p)) => format!("\u{201c}{p}\u{201d} limits who can view this page"),
        (false, false, None) => return,
    };
    let (rect, r) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    icons::paint_lock(ui.painter(), t.muted, rect.shrink(2.0));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Restricted page"));
    r.on_hover_text(tip);
}

/// The lock badge applies: the page or an ancestor restricts it.
pub(crate) fn badged(ws: &Workspace, id: Id) -> bool {
    restricted(ws, id) || inherited(ws, id).is_some()
}

/// One list: the two choices, then (when limited) chips and a person picker.
#[allow(clippy::too_many_arguments)]
fn list(ui: &mut Ui, t: &Tokens, app_ws: &Workspace, me: &str, salt: &str, only: &mut bool, people: &mut Vec<String>, words: [&str; 3]) {
    let [anyone, these, add] = words;
    let was = *only;
    ui.radio_value(only, false, anyone);
    ui.radio_value(only, true, these);
    if *only && !was && people.is_empty() {
        // Nobody locks themself out: the restricting person starts on the list.
        people.push(me.to_string());
    }
    if !*only {
        return;
    }
    ui.indent(salt, |ui| {
        let mut gone = None;
        ui.horizontal_wrapped(|ui| {
            for (n, u) in people.iter().enumerate() {
                let name = name_of(app_ws, Some(u));
                egui::Frame::new().fill(t.surface_alt).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        crate::widgets::who(ui, t, app_ws, Some(u), 16.0);
                        ui.label(&name);
                        if remove_button(ui, t, &format!("Remove {name}"), 16.0).clicked() {
                            gone = Some(n);
                        }
                    });
                });
            }
        });
        if let Some(n) = gone
            && n < people.len()
        {
            people.remove(n);
        }
        ui.horizontal(|ui| {
            let l = ui.label(add);
            let picked = person_adder(ui, app_ws, salt, "Choose someone", l.id, 200.0);
            if let Some(Some(u)) = picked.choice
                && !people.contains(&u)
                && people.len() < MAX_LIST
            {
                people.push(u);
            }
        });
        if people.is_empty() {
            ui.label(RichText::new("No one is on the list: only space admins could open the page.").color(t.danger).size(12.5));
        }
    });
}

/// Draws the Restrictions dialog; returns false when it closed.
pub(crate) fn dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens, form: &mut RestrictForm) -> bool {
    let ws = &app.session.ws;
    let Ok(page) = ws.page(form.page) else { return false };
    let title = page.title.clone();
    let parent = inherited(ws, form.page);
    let me = app.session.user.clone();
    let mut close = false;
    let mut submit = false;
    let modal = egui::Modal::new(egui::Id::unique("restrict-page")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.heading("Restrictions");
        ui.add(egui::Label::new(RichText::new(&title).color(t.muted)).truncate());
        ui.add_space(8.0);
        if let Some(p) = &parent {
            egui::Frame::new().fill(t.accent_soft).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    icons::paint_lock(ui.painter(), t.text, rect);
                    ui.add(egui::Label::new(format!("\u{201c}{p}\u{201d}, a parent page, limits who can view it. Those limits apply here too: only people on both lists can view this page.")).wrap());
                });
            });
            ui.add_space(8.0);
        }
        ui.label(RichText::new("Viewing").strong());
        list(ui, t, ws, &me, "restrict-view", &mut form.view_only, &mut form.view,  ["Anyone in the space can view", "Only these people can view", "Add a viewer"]);
        ui.label(RichText::new("Child pages follow these viewing limits.").color(t.muted).size(12.0));
        ui.add_space(10.0);
        ui.label(RichText::new("Editing").strong());
        list(ui, t, ws, &me, "restrict-edit", &mut form.edit_only, &mut form.edit,  ["Anyone who can edit in the space can edit", "Only these people can edit", "Add an editor"]);
        ui.label(RichText::new("People who can edit can also view this page.").color(t.muted).size(12.0));
        if let Some(e) = &form.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(t.danger));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            submit = ui.add(egui::Button::new(RichText::new("Apply").color(t.on_accent).strong()).fill(t.accent)).clicked();
            close |= ui.button("Cancel").clicked();
        });
    });
    close |= modal.should_close();
    if submit {
        let side = |only: bool, people: &[String]| if only { json!(people) } else { Value::Null };
        let p = json!({"id": form.page.0, "view": side(form.view_only, &form.view), "edit": side(form.edit_only, &form.edit)});
        match app.session.run("page.restrict", &p) {
            Ok(_) => {
                app.jql_ran = None;
                let any = form.view_only || form.edit_only;
                app.notify_done(if any { "Restrictions saved" } else { "Restrictions removed" }, true);
                return false;
            }
            Err(e) => form.error = Some(e),
        }
    }
    !close
}
