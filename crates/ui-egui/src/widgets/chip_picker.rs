//! M2.4: a set of names from a fixed list (components, versions, multi-select options) as chips,
//! like the label editor: each chip has a remove cross, and an "Add" menu lists what's left.
//! Also a text field that commits on Enter or focus loss (dates, text, URLs).

use egui::{RichText, Ui};

use super::remove_button;
use crate::ComboBox;
use crate::theme::Tokens;

/// Options listed in the add menu at most.
const SHOWN: usize = 200;

/// Draws `selected` as chips and an add menu over `options` (named by `label`; `what` names a
/// chip for the remove buttons: "Remove component Docs"). Returns the new list when it changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chip_picker(
    ui: &mut Ui,
    t: &Tokens,
    id_salt: &str,
    what: &str,
    selected: &[String],
    options: &[String],
    label: egui::Id,
    width: f32,
) -> Option<Vec<String>> {
    let mut out: Option<Vec<String>> = None;
    ui.vertical(|ui| {
        ui.set_max_width(width);
        if !selected.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for s in selected {
                    egui::Frame::new()
                        .fill(t.surface_alt)
                        .stroke(egui::Stroke::new(1.0, t.border))
                        .corner_radius(10)
                        .inner_margin(egui::Margin { left: 8, right: 2, top: 1, bottom: 1 })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let shown: String = s.chars().take(40).collect();
                                ui.add(egui::Label::new(RichText::new(shown).size(12.0)).extend()).on_hover_text(s);
                                if remove_button(ui, t, &format!("Remove {what} {s}"), 16.0).clicked() {
                                    out = Some(selected.iter().filter(|o| *o != s).cloned().collect());
                                }
                            });
                        });
                }
            });
        }
        let left: Vec<&String> = options.iter().filter(|o| !selected.contains(o)).take(SHOWN).collect();
        let hint = if options.is_empty() { format!("No {what}s yet") } else { format!("Add {what}") };
        ui.add_enabled_ui(!left.is_empty(), |ui| {
            ComboBox::from_id_salt(id_salt)
                .selected_text(RichText::new(hint).color(t.muted))
                .width(width.min(180.0))
                .show_ui(ui, |ui| {
                    for o in &left {
                        let shown: String = o.chars().take(60).collect();
                        if ui.selectable_label(false, shown).clicked() {
                            let mut next = out.clone().unwrap_or_else(|| selected.to_vec());
                            next.push((*o).clone());
                            out = Some(next);
                        }
                    }
                })
                .response
                .labelled_by(label);
        });
    });
    out
}

#[allow(clippy::too_many_arguments)]
/// A one-line text field for `value` that commits on Enter or focus loss (Escape puts the old
/// value back). `check` validates the typed text: while it fails, the reason shows under the
/// field and nothing is committed. Returns the committed text when it differs from `value`.
pub(crate) fn commit_field(
    ui: &mut Ui,
    t: &Tokens,
    id_salt: &str,
    value: &str,
    hint: &str,
    label: egui::Id,
    width: f32,
    check: impl Fn(&str) -> Result<(), String>,
) -> Option<String> {
    let id = ui.make_persistent_id(id_salt);
    let draft = ui.data(|d| d.get_temp::<String>(id));
    let mut text = draft.clone().unwrap_or_else(|| value.to_string());
    let mut out = None;
    ui.vertical(|ui| {
        // Inside a grid the cell is as narrow as last frame's column; claim the width first.
        ui.set_min_width(width);
        let r = ui
            .add_sized([width, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut text).id(id.with("edit")).hint_text(hint))
            .labelled_by(label)
            .on_hover_text("Enter to save, Escape to cancel");
        let bad = check(&text).err();
        if r.has_focus() {
            if let Some(why) = &bad {
                ui.label(RichText::new(why).size(11.0).color(t.danger));
            }
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
            return;
        }
        ui.data_mut(|d| d.remove::<String>(id));
        if !r.lost_focus() || draft.is_none() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return;
        }
        if bad.is_none() && text.trim() != value.trim() {
            out = Some(text.trim().to_string());
        }
    });
    out
}
