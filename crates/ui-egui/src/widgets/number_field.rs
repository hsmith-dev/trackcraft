//! A number text field that commits only on Enter or when it loses focus (UI-9, gap G6), so one
//! edit is one command and one undo step, not one per typed digit. Accepts decimals (`2.5` or
//! `2,5`); empty clears the value; Escape or text that isn't a number puts the old value back.

use egui::Ui;

/// How a value is shown in the field: `3`, `2.5`, empty for none.
pub(crate) fn format_number(v: Option<f64>) -> String {
    match v {
        Some(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{n:.0}"),
        Some(n) => n.to_string(),
        None => String::new(),
    }
}

/// Parses typed text: `Some(None)` for empty, `Some(Some(n))` for a finite number, `None` when
/// the text isn't a number.
pub(crate) fn parse_number(text: &str) -> Option<Option<f64>> {
    let t = text.trim();
    if t.is_empty() {
        return Some(None);
    }
    t.replace(',', ".").parse::<f64>().ok().filter(|n| n.is_finite()).map(Some)
}

/// Draws the field for `value`, named by `label` for accessibility. Returns the new value when an
/// edit was committed and differs from `value`.
pub(crate) fn number_field(ui: &mut Ui, id_salt: &str, value: Option<f64>, label: egui::Id, width: f32) -> Option<Option<f64>> {
    let id = ui.make_persistent_id(id_salt);
    let draft = ui.data(|d| d.get_temp::<String>(id));
    let mut text = draft.clone().unwrap_or_else(|| format_number(value));
    let r = ui
        .add(egui::TextEdit::singleline(&mut text).id(id.with("edit")).desired_width(width).hint_text("None"))
        .labelled_by(label)
        .on_hover_text("Enter to save, Escape to cancel");
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
        return None;
    }
    // Not focused: drop the draft, so a change from elsewhere (undo) shows.
    ui.data_mut(|d| d.remove::<String>(id));
    if !r.lost_focus() || draft.is_none() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return None;
    }
    match parse_number(&text) {
        Some(v) if v != value => Some(v),
        _ => None,
    }
}
