//! What a view shows when it has nothing to show (UI-6): a title, a muted hint, and at most one
//! action button. Centred in the space it's given; text wraps, so long hints never widen a layout.

use egui::{RichText, Ui};

use crate::theme::Tokens;

/// Draws the empty state; returns true when its action button was clicked. `title` may be empty
/// (a hint-only state, for small places like an empty board column).
pub(crate) fn empty_state(ui: &mut Ui, t: &Tokens, title: &str, hint: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        let big = !title.is_empty();
        ui.add_space(if big { 18.0 } else { 6.0 });
        if big {
            ui.add(egui::Label::new(RichText::new(title).size(16.0).strong().color(t.text)).wrap());
            ui.add_space(4.0);
        }
        if !hint.is_empty() {
            ui.add(egui::Label::new(RichText::new(hint).color(t.muted)).wrap());
        }
        if let Some(a) = action {
            ui.add_space(10.0);
            clicked = ui.button(a).clicked();
        }
        ui.add_space(if big { 18.0 } else { 6.0 });
    });
    clicked
}
