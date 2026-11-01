//! One row of a two-column field grid: a muted label, then the value.

use egui::{RichText, Ui};

use crate::theme::Tokens;

/// Draws `label`, then `value` (given the label's id, for `labelled_by`), then ends the grid row.
pub(crate) fn field_row<R>(ui: &mut Ui, t: &Tokens, label: &str, value: impl FnOnce(&mut Ui, egui::Id) -> R) -> R {
    // Labels never truncate, even where the values do (UI-6).
    let id = ui.add(egui::Label::new(RichText::new(label).color(t.muted)).extend()).id;
    let r = value(ui, id);
    ui.end_row();
    r
}
