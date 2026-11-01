//! The status bar (UI-5): a thin strip at the bottom of the window. Left, the save state and the
//! file name; middle, what the current view shows; right, the acting user and the shortcut hint.

use egui::{Align, Layout, RichText, Ui, UiBuilder};

use crate::theme::Tokens;
use crate::{Action, TrackApp, View};

pub(crate) const HEIGHT: f32 = 24.0;

/// "Unsaved changes" / "Saved" / "Not saved yet".
pub(crate) fn save_state(app: &TrackApp) -> &'static str {
    if app.session.dirty {
        "Unsaved changes"
    } else if app.session.path.is_some() {
        "Saved"
    } else {
        "Not saved yet"
    }
}

/// The workspace file's name (not its folder), if it has one.
pub(crate) fn file_name(app: &TrackApp) -> Option<String> {
    let p = app.session.path.as_ref()?;
    let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
    Some(middle_truncate(&name, 48))
}

/// What the current view shows, in a few words.
pub(crate) fn context(app: &TrackApp) -> String {
    let ws = &app.session.ws;
    let issues = |n: usize| if n == 1 { "1 issue".to_string() } else { format!("{n} issues") };
    let s = match &app.view {
        View::Home => format!("{} in the workspace", issues(ws.issues.len())),
        View::Issues => match &app.jql_result {
            Ok(_) => format!("{} match the search", issues(app.jql_total)),
            Err(_) => "The search has an error".into(),
        },
        View::Board(p) => format!("{} on the {p} board", issues(ws.issues.keys().filter(|k| k.project == *p).count())),
        View::Backlog(p) => format!("{p} backlog"),
        View::SprintBoard(_, id) => match ws.sprints.get(id) {
            Some(sp) => format!("Sprint: {}", sp.name),
            None => "Sprint board".into(),
        },
        View::Issue(k) => match ws.issues.get(k) {
            Some(i) => format!("{k} is {}", i.status),
            None => k.to_string(),
        },
        View::Page(id) | View::PageHistory(id) => match ws.page(*id) {
            Ok(p) => format!("Page version {}", p.version),
            Err(_) => "Page".into(),
        },
        // The directory itself shows the count; the bar names the place.
        View::Projects => "Project directory".into(),
        View::Spaces => "Space directory".into(),
        View::Settings(tab) => format!("Settings: {}", tab.label()), // UI-7
    };
    middle_truncate(&s, 80)
}

/// Shortens `s` to `max` characters by cutting out its middle (char-boundary safe).
fn middle_truncate(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max || max < 5 {
        return s.to_string();
    }
    let keep = max - 3;
    let head: String = s.chars().take(keep / 2 + keep % 2).collect();
    let tail: String = s.chars().skip(n - keep / 2).collect();
    format!("{head}...{tail}")
}

pub(crate) fn show(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let state = save_state(app);
    let file = file_name(app);
    let middle = context(app);
    let user = crate::widgets::name_of(&app.session.ws, Some(&app.session.user));
    egui::Panel::bottom("status_bar")
        .exact_size(HEIGHT)
        .frame(egui::Frame::new().fill(t.sidebar).inner_margin(egui::Margin::symmetric(12, 0)).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let small = |s: &str| RichText::new(s).size(12.0).color(t.muted);
            ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)), |ui| {
                let dot = if app.session.dirty { t.accent } else { t.muted };
                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, dot);
                let col = if app.session.dirty { t.text } else { t.muted };
                ui.label(RichText::new(state).size(12.0).color(col));
                if let Some(f) = &file {
                    ui.label(small("·"));
                    ui.label(small(f)).on_hover_text(app.session.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
                }
            });
            // The middle third, text centred in it.
            let mid = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width() / 3.0, rect.height()));
            ui.put(mid, egui::Label::new(small(&middle)).truncate());
            ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)), |ui| {
                let hint = ui.add(egui::Button::new(small("? Shortcuts")).frame(false)).on_hover_text("Show keyboard shortcuts");
                if hint.clicked() {
                    app.push(Action::ShowHelp);
                }
                ui.label(small("·"));
                ui.label(small(&format!("Acting as {user}")));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::middle_truncate;

    #[test]
    fn middle_truncate_keeps_both_ends() {
        assert_eq!(middle_truncate("short", 10), "short");
        let s = middle_truncate("abcdefghijklmnopqrstuvwxyz", 11);
        assert_eq!(s.chars().count(), 11);
        assert!(s.starts_with("abcd") && s.ends_with("wxyz"));
        // Multi-byte characters never split.
        assert_eq!(middle_truncate("ééééééééééééé", 7).chars().count(), 7);
    }
}
