//! Labels as chips (UI-9): each chip has a painted remove cross; typing a word and pressing
//! Enter (or a comma or space) adds it; matching labels already used in the workspace are
//! offered under the field.

use egui::{RichText, Ui};

use crate::theme::Tokens;

/// Suggestions shown under the field at most.
const SUGGESTIONS: usize = 6;

/// A small painted cross button, named `name` for accessibility.
pub(crate) fn remove_button(ui: &mut Ui, t: &Tokens, name: &str, size: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let name = name.to_string();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &name));
    let color = if r.hovered() { t.danger } else { t.muted };
    if r.hovered() {
        ui.painter().circle_filled(rect.center(), size / 2.0, t.surface_alt);
    }
    let d = size * 0.22;
    let c = rect.center();
    let s = egui::Stroke::new(1.4, color);
    ui.painter().line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], s);
    ui.painter().line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], s);
    r.on_hover_text(name)
}

/// The words of `text` (split on commas and whitespace).
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Draws the chips and the add field (named by `label`, holding `draft`). Returns the new label
/// list when it changed. `suggestions` are labels used elsewhere in the workspace.
#[allow(clippy::too_many_arguments)]
pub(crate) fn label_editor(
    ui: &mut Ui,
    t: &Tokens,
    id_salt: &str,
    draft: &mut String,
    labels: &[String],
    suggestions: &[String],
    label: egui::Id,
    width: f32,
) -> Option<Vec<String>> {
    let id = ui.make_persistent_id(id_salt);
    let mut out: Option<Vec<String>> = None;
    ui.vertical(|ui| {
        ui.set_max_width(width);
        if !labels.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for l in labels {
                    egui::Frame::new()
                        .fill(t.surface_alt)
                        .stroke(egui::Stroke::new(1.0, t.border))
                        .corner_radius(10)
                        .inner_margin(egui::Margin { left: 8, right: 2, top: 1, bottom: 1 })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let shown: String = l.chars().take(40).collect();
                                ui.add(egui::Label::new(RichText::new(shown).size(12.0)).extend()).on_hover_text(l);
                                if remove_button(ui, t, &format!("Remove label {l}"), 16.0).clicked() {
                                    out = Some(labels.iter().filter(|o| *o != l).cloned().collect());
                                }
                            });
                        });
                }
            });
        }
        let r =
            ui.add(egui::TextEdit::singleline(draft).id(id.with("edit")).hint_text("Add a label").desired_width(width.min(180.0))).labelled_by(label);
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let separator = draft.ends_with(',') || draft.ends_with(' ');
        if enter || separator {
            let mut next: Vec<String> = out.clone().unwrap_or_else(|| labels.to_vec());
            for w in words(draft) {
                if !next.contains(&w) {
                    next.push(w);
                }
            }
            if next.as_slice() != labels {
                out = Some(next);
            }
            draft.clear();
            if enter {
                r.request_focus();
            }
        }
        let typed = draft.trim().to_lowercase();
        if !typed.is_empty() {
            let hits: Vec<&String> =
                suggestions.iter().filter(|s| !labels.contains(s) && s.to_lowercase().contains(&typed)).take(SUGGESTIONS).collect();
            if !hits.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Used:").size(11.0).color(t.muted));
                    for s in hits {
                        if ui.small_button(s.as_str()).on_hover_text(format!("Add label {s}")).clicked() {
                            let mut next: Vec<String> = out.clone().unwrap_or_else(|| labels.to_vec());
                            next.push(s.clone());
                            out = Some(next);
                            draft.clear();
                        }
                    }
                });
            }
        }
    });
    out
}
