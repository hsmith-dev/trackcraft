//! A user's avatar (or the unassigned placeholder) and display name, by user id.
//! The avatar itself is painted by [`icons::avatar`].

use egui::Ui;
use trackcraft_model::Workspace;

use crate::icons;
use crate::theme::Tokens;

pub(crate) fn who(ui: &mut Ui, t: &Tokens, ws: &Workspace, id: Option<&str>, size: f32) {
    match id.and_then(|u| ws.users.get(u)) {
        Some(u) => {
            icons::avatar(ui, t, &u.id, &u.display_name, size);
        }
        None => {
            icons::unassigned(ui, t, size);
        }
    }
}

pub(crate) fn name_of(ws: &Workspace, id: Option<&str>) -> String {
    id.and_then(|u| ws.users.get(u)).map_or_else(|| "Unassigned".to_string(), |u| u.display_name.clone())
}
