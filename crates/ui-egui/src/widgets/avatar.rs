//! A user's avatar (or the unassigned placeholder) and display name, by user id.
//! The avatar itself is painted by [`icons::avatar`]. A user id with no user (removed, or from
//! another workspace) gets a neutral grey avatar and shows as its raw id (UI-6).

use egui::Ui;
use trackcraft_model::Workspace;

use crate::icons;
use crate::theme::Tokens;

pub(crate) fn who(ui: &mut Ui, t: &Tokens, ws: &Workspace, id: Option<&str>, size: f32) {
    match id {
        Some(i) => match ws.users.get(i) {
            Some(u) => {
                icons::avatar(ui, t, &u.id, &u.display_name, size);
            }
            None => {
                unknown(ui, t, i, size);
            }
        },
        None => {
            icons::unassigned(ui, t, size);
        }
    }
}

/// A grey avatar with a question mark, for a user id that isn't in the workspace.
fn unknown(ui: &mut Ui, t: &Tokens, id: &str, size: f32) {
    let (r, resp) = icons::square(ui, size);
    let p = ui.painter();
    p.circle_filled(r.center(), r.width() / 2.0, t.surface_alt);
    p.circle_stroke(r.center(), r.width() / 2.0 - 0.5, egui::Stroke::new(1.0, t.border));
    p.text(r.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(r.width() * 0.5), t.muted);
    let shown: String = id.chars().take(128).collect();
    resp.on_hover_text(format!("Unknown user: {shown}"));
}

/// The user's display name; the raw id for an unknown user; "Unassigned" for none.
pub(crate) fn name_of(ws: &Workspace, id: Option<&str>) -> String {
    match id {
        Some(i) => ws.users.get(i).map_or_else(|| i.chars().take(128).collect(), |u| u.display_name.clone()),
        None => "Unassigned".to_string(),
    }
}
