//! A titled surface: rounded, bordered, full width.

use egui::{RichText, Ui};

use crate::theme::Tokens;

pub(crate) fn card<R>(ui: &mut Ui, t: &Tokens, title: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(t.surface)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(15.0));
                ui.add_space(6.0);
            }
            body(ui)
        })
        .inner
}
