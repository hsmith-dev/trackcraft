//! An issue picker (UI-9): a combo box whose menu searches the given issues by key or summary.
//! Enter picks the first match; `allow_none` adds a "None" choice (clear a parent).

use egui::Ui;
use trackcraft_core::IssueKey;

use crate::widgets::Picked;
use crate::{ComboBox, icons};

/// Matches listed in the menu at most.
const SHOWN: usize = 50;

/// One issue the picker offers.
pub(crate) struct Candidate {
    pub key: IssueKey,
    pub summary: String,
    pub issue_type: String,
}

/// `KEY  summary`, the summary cut to `n` characters.
pub(crate) fn short(key: &IssueKey, summary: &str, n: usize) -> String {
    let s: String = summary.chars().take(n).collect();
    let more = if summary.chars().count() > n { "..." } else { "" };
    format!("{key}  {s}{more}")
}

/// Draws the picker showing `selected`, named by `label`. `candidates` is only built while the
/// menu is open (a 10k-issue workspace isn't listed every frame). The choice is `Some(key)`, or
/// `None` for "None" (only offered with `allow_none`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn issue_picker(
    ui: &mut Ui,
    id_salt: &str,
    selected: String,
    candidates: impl FnOnce() -> Vec<Candidate>,
    current: Option<&IssueKey>,
    allow_none: bool,
    label: egui::Id,
    width: f32,
) -> Picked<Option<IssueKey>> {
    let search_id = ui.make_persistent_id((id_salt, "search"));
    let mut choice: Option<Option<IssueKey>> = None;
    let inner = ComboBox::from_id_salt(id_salt)
        .selected_text(selected)
        .width(width)
        .truncate()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui_with_header(
            ui,
            |ui| {
                let mut q = ui.data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
                let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Search by key or summary").desired_width(f32::INFINITY));
                let opened = search_id.with("opened");
                if !ui.data(|d| d.get_temp::<bool>(opened).unwrap_or(false)) {
                    r.request_focus();
                    ui.data_mut(|d| d.insert_temp(opened, true));
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.data_mut(|d| d.insert_temp(search_id, q.clone()));
                (q.trim().to_lowercase(), enter)
            },
            |ui, (q, enter)| {
                let all = candidates();
                let hits: Vec<&Candidate> = all
                    .iter()
                    .filter(|c| q.is_empty() || c.key.to_string().to_lowercase().contains(&q) || c.summary.to_lowercase().contains(&q))
                    .take(SHOWN)
                    .collect();
                if allow_none && q.is_empty() && ui.selectable_label(current.is_none(), "None").clicked() {
                    choice = Some(None);
                }
                if hits.is_empty() {
                    ui.label("No issues match");
                }
                for c in &hits {
                    ui.horizontal(|ui| {
                        icons::issue_type(ui, &c.issue_type, 14.0);
                        if ui.selectable_label(current == Some(&c.key), short(&c.key, &c.summary, 60)).clicked() {
                            choice = Some(Some(c.key.clone()));
                        }
                    });
                }
                if enter && let Some(c) = hits.first() {
                    choice = Some(Some(c.key.clone()));
                }
                if choice.is_some() {
                    ui.close();
                }
            },
        );
    if inner.inner.is_none() {
        ui.data_mut(|d| {
            d.remove::<String>(search_id);
            d.remove::<bool>(search_id.with("opened"));
        });
    }
    let response = inner.response.labelled_by(label);
    Picked { response, choice: choice.filter(|c| c.as_ref() != current) }
}
