//! A status pill: upper-case status name on its category colour.

use egui::{RichText, Ui};
use trackcraft_model::StatusCategory;

use crate::theme::Tokens;

pub(crate) fn pill(ui: &mut Ui, t: &Tokens, cat: StatusCategory, text: &str) -> egui::Response {
    let (fill, fg) = t.status(cat);
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| ui.label(RichText::new(text.to_uppercase()).size(11.0).strong().color(fg)))
        .response
}
