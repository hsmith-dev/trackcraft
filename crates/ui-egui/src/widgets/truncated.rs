//! One-line text that never widens its row (UI-6): truncated to the space left, with an
//! ellipsis and the full text on hover.

use egui::{Response, Ui, WidgetText};

/// A link cut to the space left in the row.
pub(crate) fn link_truncated(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    ui.scope(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.link(text)
    })
    .inner
}
