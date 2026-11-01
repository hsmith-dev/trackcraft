//! One row of a two-column field grid: a muted label, then the value.

use egui::{RichText, Ui};

use crate::theme::Tokens;

/// Draws `label`, then `value` (given the label's id, for `labelled_by`), then ends the grid row.
pub(crate) fn field_row<R>(ui: &mut Ui, t: &Tokens, label: &str, value: impl FnOnce(&mut Ui, egui::Id) -> R) -> R {
    let id = ui.label(RichText::new(label).color(t.muted)).id;
    let r = value(ui, id);
    ui.end_row();
    r
}
