//! The status bar (UI-5): a thin strip at the bottom of the window. Left, the save state and the
//! file name; middle, what the current view shows; right, the acting user and the shortcut hint.
//! On a server workspace (M4.7) the left section shows the connection indicator instead of the
//! save state: a dot and a label, a tooltip with the details, and "Reconnect now" when offline.

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
        View::Settings(tab) => format!("Settings: {}", tab.label()),           // UI-7
        View::PageDiff(_, from, to) => format!("Comparing v{from} and v{to}"), // UI-28
        View::Inbox => "Notifications".into(),                                 // M4.5
        // UI-24
        View::Reports(p) => format!("{p} reports"),
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
    let conn = app.conn.state.clone(); // M4.7
    let remote = app.is_remote(); // M4.7
    egui::Panel::bottom("status_bar")
        .exact_size(HEIGHT)
        .frame(egui::Frame::new().fill(t.sidebar).inner_margin(egui::Margin::symmetric(12, 0)).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let small = |s: &str| RichText::new(s).size(12.0).color(t.muted);
            ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)), |ui| {
                // M4.7: the connection indicator; a server workspace has no file to save.
                if let Some(c) = &conn {
                    connection_indicator(app, ui, t, c);
                }
                if remote {
                    return;
                }
                if conn.is_some() {
                    ui.label(small("·"));
                }
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

// M4.7
/// How the connection to a server is doing (the indicator's dot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    /// Connected; nothing waiting.
    Online,
    /// Connected; changes are being sent or the workspace is reloading.
    Syncing,
    /// The server can't be reached; changes are queued.
    Offline,
    /// The server refused us (e.g. the token); changes are queued.
    Error,
}

impl ConnectionKind {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "online" => ConnectionKind::Online,
            "syncing" => ConnectionKind::Syncing,
            "offline" => ConnectionKind::Offline,
            "error" => ConnectionKind::Error,
            _ => return None,
        })
    }

    /// Clicking the indicator offers "Reconnect now".
    pub(crate) fn reconnectable(self) -> bool {
        matches!(self, ConnectionKind::Offline | ConnectionKind::Error)
    }

    fn default_label(self) -> &'static str {
        match self {
            ConnectionKind::Online => "Online",
            ConnectionKind::Syncing => "Syncing",
            ConnectionKind::Offline => "Offline",
            ConnectionKind::Error => "Connection error",
        }
    }
}

/// What the host reports about its server connection ([`TrackApp::set_connection`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionState {
    /// A word or two for the status bar (`Online`, `Offline · 2 unsent`).
    pub label: String,
    pub kind: ConnectionKind,
    /// The tooltip: the server, changes waiting to be sent, the last error (one item per line).
    pub detail: String,
}

impl ConnectionState {
    /// `label` empty = the kind's own word.
    pub fn new(kind: ConnectionKind, label: impl Into<String>, detail: impl Into<String>) -> Self {
        let label = label.into();
        let label = if label.trim().is_empty() { kind.default_label().to_string() } else { label };
        ConnectionState { label, kind, detail: detail.into() }
    }
}

/// The indicator's state on [`TrackApp`].
#[derive(Debug, Default)]
pub(crate) struct Connection {
    pub state: Option<ConnectionState>,
    /// "Reconnect now" was chosen; the host takes it ([`TrackApp::take_reconnect_request`]).
    pub reconnect: bool,
    /// A server workspace ([`TrackApp::set_remote`]): no undo, no file to save.
    pub remote: bool,
}

/// Longest label and tooltip shown (host text is not trusted to be short).
const MAX_LABEL: usize = 40;
const MAX_DETAIL: usize = 600;

/// The dot + label; hover for the details; a click offers "Reconnect now" when offline.
fn connection_indicator(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, c: &ConnectionState) {
    let label = middle_truncate(c.label.lines().next().unwrap_or_default(), MAX_LABEL);
    let detail: String = c.detail.chars().take(MAX_DETAIL).collect();
    let clickable = c.kind.reconnectable();
    let col = if c.kind == ConnectionKind::Error { t.danger } else { t.text };
    let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(12.0), col);
    let size = egui::vec2(galley.size().x + 22.0, HEIGHT - 6.0);
    let (r, resp) = ui.allocate_exact_size(size, if clickable { egui::Sense::click() } else { egui::Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Connection: {label}")));
    let p = ui.painter();
    if clickable && resp.hovered() {
        p.rect_filled(r, 4.0, t.surface_alt);
    }
    let dot = egui::pos2(r.left() + 9.0, r.center().y);
    match c.kind {
        ConnectionKind::Online => {
            p.circle_filled(dot, 4.0, t.ok);
        }
        ConnectionKind::Syncing => {
            // A ring with a filled centre: "working".
            p.circle_stroke(dot, 4.0, egui::Stroke::new(1.5, t.accent));
            p.circle_filled(dot, 2.0, t.accent);
        }
        // Hollow: not connected (shape, not only colour, tells the states apart).
        ConnectionKind::Offline => {
            p.circle_stroke(dot, 3.5, egui::Stroke::new(1.5, t.muted));
        }
        ConnectionKind::Error => {
            p.circle_filled(dot, 4.0, t.danger);
        }
    }
    let text_pos = egui::pos2(r.left() + 18.0, r.center().y - galley.size().y / 2.0);
    p.galley(text_pos, galley, col);
    let resp = resp.on_hover_ui(|ui| {
        ui.set_max_width(360.0);
        ui.label(RichText::new(&label).strong());
        for line in detail.lines().filter(|l| !l.trim().is_empty()) {
            ui.label(RichText::new(line).size(12.0));
        }
        if clickable {
            ui.label(RichText::new("Click to reconnect now").size(12.0).color(t.muted));
        }
    });
    if clickable {
        egui::Popup::menu(&resp).show(|ui| {
            if ui.button("Reconnect now").on_hover_text("Try the server again without waiting").clicked() {
                app.conn.reconnect = true;
            }
        });
    }
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
