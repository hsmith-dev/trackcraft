//! A muted one-line message where a view has nothing to show.

use egui::{RichText, Ui};

use crate::theme::Tokens;

pub(crate) fn empty_state(ui: &mut Ui, t: &Tokens, text: &str) -> egui::Response {
    ui.label(RichText::new(text).color(t.muted))
}
