//! A person picker (UI-9): a combo box whose menu has a search field (by display name or user
//! id), "Unassigned", "Assign to me" and the matching people. Enter picks the first match.

use egui::Ui;
use trackcraft_model::Workspace;

use crate::ComboBox;
use crate::widgets::name_of;

/// People listed in the menu at most (the search narrows the rest).
const SHOWN: usize = 200;

/// What a picker reports.
pub(crate) struct Picked<T> {
    /// The combo box (its `id.with("popup")` opens the menu).
    pub response: egui::Response,
    /// The choice made this frame, if any.
    pub choice: Option<T>,
}

/// Draws the picker for `current` (a user id or none), named by `label`. `me` is the signed-in
/// user, for "Assign to me". The choice is `Some(user id)` or `None` (unassigned).
pub(crate) fn user_picker(
    ui: &mut Ui,
    ws: &Workspace,
    me: &str,
    id_salt: &str,
    current: Option<&str>,
    label: egui::Id,
    width: f32,
) -> Picked<Option<String>> {
    let search_id = ui.make_persistent_id((id_salt, "search"));
    let mut choice: Option<Option<String>> = None;
    let inner = ComboBox::from_id_salt(id_salt)
        .selected_text(name_of(ws, current))
        .width(width)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui_with_header(
            ui,
            |ui| {
                let mut q = ui.data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
                let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Search people").desired_width(f32::INFINITY));
                // Focus the search the first frame the menu is open.
                let opened = search_id.with("opened");
                if !ui.data(|d| d.get_temp::<bool>(opened).unwrap_or(false)) {
                    r.request_focus();
                    ui.data_mut(|d| d.insert_temp(opened, true));
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.data_mut(|d| d.insert_temp(search_id, q.clone()));
                (q.trim().to_lowercase(), enter)
            },
            |ui, (q, enter)| {
                let hits: Vec<(&str, &str)> = ws
                    .users
                    .values()
                    .filter(|u| q.is_empty() || u.display_name.to_lowercase().contains(&q) || u.id.to_lowercase().contains(&q))
                    .take(SHOWN)
                    .map(|u| (u.id.as_str(), u.display_name.as_str()))
                    .collect();
                if q.is_empty() {
                    if ui.selectable_label(current.is_none(), "Unassigned").clicked() {
                        choice = Some(None);
                    }
                    if current != Some(me) && ui.selectable_label(false, "Assign to me").clicked() {
                        choice = Some(Some(me.to_string()));
                    }
                    ui.separator();
                }
                if hits.is_empty() {
                    ui.label("No one matches");
                }
                for (id, name) in &hits {
                    let shown: String = name.chars().take(60).collect();
                    if ui.selectable_label(current == Some(*id), shown).on_hover_text(*id).clicked() {
                        choice = Some(Some((*id).to_string()));
                    }
                }
                if enter && let Some((id, _)) = hits.first() {
                    choice = Some(Some((*id).to_string()));
                }
                if choice.is_some() {
                    ui.close();
                }
            },
        );
    if inner.inner.is_none() {
        // Closed: the next open starts with an empty, focused search.
        ui.data_mut(|d| {
            d.remove::<String>(search_id);
            d.remove::<bool>(search_id.with("opened"));
        });
    }
    let response = inner.response.labelled_by(label);
    Picked { response, choice: choice.filter(|c| c.as_deref() != current) }
}
