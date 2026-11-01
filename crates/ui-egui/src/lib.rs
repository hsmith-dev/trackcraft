//! TrackCraft's egui front end (L6). Thin by design: views read `Session` state and change it
//! only through [`Session::run`] — the same commands as the CLI, MCP and control channel.
//!
//! Each frame, views push [`Action`]s; [`TrackApp::ui`] applies them after drawing, so no view
//! holds a mutable borrow of the session while it paints.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod icons;
mod issues;
pub mod markup_view;
mod palette;
mod shell;
pub mod theme;
mod wiki;

use serde_json::{Value, json};
use trackcraft_core::{Id, IssueKey, ProjectKey};
use trackcraft_engine::Session;

pub use craft_ui::ComboBox;
use theme::Tokens;

/// What the central area shows.
#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Home,
    Issues,
    Board(ProjectKey),
    Issue(IssueKey),
    Page(Id),
    PageHistory(Id),
}

/// A request the desktop host fulfils (file dialogs live outside this crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRequest {
    Open,
    Save,
    SaveAs,
    New,
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Nav(View),
    Back,
    Run { id: &'static str, params: Value, then: Then },
    Search(String),
    OpenCreate(Option<String>),
    OpenNewPage { space: String, parent: Option<Id> },
    EditPage(Id),
    File(FileRequest),
    TogglePalette,
    ToggleTheme,
    Toast(String),
}

/// What to do after a successful command.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Then {
    Nothing,
    Toast(String),
    /// Open the issue whose key is in the result's `key`.
    OpenIssue,
    /// Leave page editing.
    EndPageEdit,
    Nav(View),
}

#[derive(Debug, Default)]
pub(crate) struct IssueUi {
    pub editing_summary: Option<String>,
    pub editing_description: Option<String>,
    pub labels: Option<String>,
    pub comment: String,
    pub confirm_delete: bool,
}

#[derive(Debug, Default)]
pub(crate) struct CreateForm {
    pub project: String,
    pub issue_type: String,
    pub summary: String,
    pub description: String,
    pub priority: String,
    pub assignee: Option<String>,
    pub parent: String,
    pub error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct PageEditor {
    pub id: Id,
    pub title: String,
    pub body: String,
    pub message: String,
    pub base: u32,
    /// Set when a save hit a newer version: (their version).
    pub conflict: Option<u32>,
}

#[derive(Debug, Default)]
pub(crate) struct NewPageForm {
    pub space: String,
    pub parent: Option<Id>,
    pub title: String,
    pub error: Option<String>,
}

pub struct TrackApp {
    pub session: Session,
    pub view: View,
    back: Vec<View>,
    /// `None` follows the system; `Some(dark)` overrides.
    pub dark: Option<bool>,
    pub(crate) jql: String,
    pub(crate) jql_ran: Option<String>,
    pub(crate) jql_result: Result<Vec<IssueKey>, String>,
    pub(crate) board_mine: bool,
    pub(crate) board_text: String,
    pub(crate) issue_ui: IssueUi,
    pub(crate) create: Option<CreateForm>,
    pub(crate) new_page: Option<NewPageForm>,
    pub(crate) page_edit: Option<PageEditor>,
    pub(crate) history_preview: Option<u32>,
    pub(crate) palette: Option<palette::Palette>,
    pub(crate) expanded: std::collections::BTreeSet<Id>,
    toast: Option<(String, f64)>,
    pending: Vec<Action>,
    file_request: Option<FileRequest>,
    focus_search: bool,
}

impl TrackApp {
    pub fn new(session: Session) -> Self {
        let mut expanded = std::collections::BTreeSet::new();
        for s in session.ws.spaces.values() {
            expanded.insert(s.homepage);
        }
        TrackApp {
            session,
            view: View::Home,
            back: vec![],
            dark: None,
            jql: "statusCategory != Done ORDER BY priority, updated DESC".into(),
            jql_ran: None,
            jql_result: Ok(vec![]),
            board_mine: false,
            board_text: String::new(),
            issue_ui: IssueUi::default(),
            create: None,
            new_page: None,
            page_edit: None,
            history_preview: None,
            palette: None,
            expanded,
            toast: None,
            pending: vec![],
            file_request: None,
            focus_search: false,
        }
    }

    /// A file dialog the host should show (taken once).
    pub fn take_file_request(&mut self) -> Option<FileRequest> {
        self.file_request.take()
    }

    /// Shows a short message at the bottom of the window.
    pub fn notify(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), -1.0));
    }

    /// The window title: workspace name, a dot when unsaved.
    pub fn title(&self) -> String {
        let dirty = if self.session.dirty { " •" } else { "" };
        format!("{}{dirty} — TrackCraft", self.session.ws.name)
    }

    pub(crate) fn tokens(&self, ctx: &egui::Context) -> Tokens {
        let dark = self.dark.unwrap_or_else(|| ctx.global_style().visuals.dark_mode);
        Tokens::for_dark(dark)
    }

    pub(crate) fn push(&mut self, a: Action) {
        self.pending.push(a);
    }

    pub fn navigate(&mut self, v: View) {
        if v != self.view {
            self.back.push(std::mem::replace(&mut self.view, v));
            if self.back.len() > 100 {
                self.back.remove(0);
            }
        }
        self.issue_ui = IssueUi::default();
        self.history_preview = None;
        if !matches!(self.view, View::Page(_)) {
            self.page_edit = None;
        }
    }

    /// Runs the JQL in the search field (cached until it changes or the workspace does).
    pub(crate) fn refresh_search(&mut self) {
        // `jql_ran` is cleared after every command, so a match means nothing changed.
        let key = self.jql.clone();
        if self.jql_ran.as_deref() == Some(key.as_str()) {
            return;
        }
        self.jql_result = match self.session.run("search.jql", &json!({"jql": self.jql, "maxResults": 1000})) {
            Ok(v) => {
                Ok(v["issues"].as_array().into_iter().flatten().filter_map(|i| i["key"].as_str()).filter_map(|k| IssueKey::parse(k).ok()).collect())
            }
            Err(e) => Err(e),
        };
        self.jql_ran = Some(key);
    }

    /// Per-frame logic without drawing (shortcuts, toasts).
    pub fn logic(&mut self, ctx: &egui::Context) {
        let t = self.tokens(ctx);
        ctx.set_visuals(t.visuals());
        let typing = ctx.egui_wants_keyboard_input();
        let modal = self.create.is_some() || self.palette.is_some() || self.new_page.is_some();
        ctx.input_mut(|i| {
            use egui::{Key, KeyboardShortcut, Modifiers};
            let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
            let cmd_shift = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
            if i.consume_shortcut(&cmd(Key::K)) || i.consume_shortcut(&cmd_shift(Key::P)) {
                self.pending.push(Action::TogglePalette);
            }
            if i.consume_shortcut(&cmd_shift(Key::Z)) {
                self.pending.push(Action::Run { id: "edit.redo", params: json!({}), then: Then::Nothing });
            } else if i.consume_shortcut(&cmd(Key::Z)) && !typing {
                self.pending.push(Action::Run { id: "edit.undo", params: json!({}), then: Then::Nothing });
            }
            if i.consume_shortcut(&cmd_shift(Key::S)) {
                self.file_request = Some(FileRequest::SaveAs);
            } else if i.consume_shortcut(&cmd(Key::S)) {
                self.file_request = Some(FileRequest::Save);
            }
            if i.consume_shortcut(&cmd(Key::O)) {
                self.file_request = Some(FileRequest::Open);
            }
            if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::ArrowLeft)) {
                self.pending.push(Action::Back);
            }
            if !typing && !modal {
                if i.consume_key(Modifiers::NONE, Key::C) {
                    self.pending.push(Action::OpenCreate(None));
                }
                if i.consume_key(Modifiers::NONE, Key::Slash) {
                    self.focus_search = true;
                }
            }
            if i.key_pressed(Key::Escape) {
                self.create = None;
                self.palette = None;
                self.new_page = None;
            }
        });
        let now = ctx.input(|i| i.time);
        if let Some((_, at)) = &mut self.toast {
            if *at < 0.0 {
                *at = now;
            }
            if now - *at > 4.0 {
                self.toast = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
    }

    /// Draws the whole window into `ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens(ui.ctx());
        shell::top_bar(self, ui, &t);
        shell::sidebar(self, ui, &t);
        egui::CentralPanel::default_margins().frame(egui::Frame::new().fill(t.bg).inner_margin(egui::Margin::symmetric(24, 16))).show(ui, |ui| {
            match self.view.clone() {
                View::Home => shell::home(self, ui, &t),
                View::Issues => issues::list(self, ui, &t),
                View::Board(p) => issues::board(self, ui, &t, &p),
                View::Issue(k) => issues::detail(self, ui, &t, &k),
                View::Page(id) => wiki::page(self, ui, &t, id),
                View::PageHistory(id) => wiki::history(self, ui, &t, id),
            }
        });
        issues::create_dialog(self, ui.ctx(), &t);
        wiki::new_page_dialog(self, ui.ctx(), &t);
        palette::show(self, ui.ctx(), &t);
        self.show_toast(ui.ctx(), &t);
        self.apply_pending();
    }

    fn show_toast(&self, ctx: &egui::Context, t: &Tokens) {
        let Some((msg, _)) = &self.toast else { return };
        egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0)).order(egui::Order::Foreground).show(
            ctx,
            |ui| {
                egui::Frame::new().fill(t.text).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                    ui.label(egui::RichText::new(msg).color(t.bg));
                });
            },
        );
    }

    pub(crate) fn take_focus_search(&mut self) -> bool {
        std::mem::take(&mut self.focus_search)
    }

    fn apply_pending(&mut self) {
        let actions = std::mem::take(&mut self.pending);
        for a in actions {
            self.apply(a);
        }
    }

    fn apply(&mut self, a: Action) {
        match a {
            Action::Nav(v) => self.navigate(v),
            Action::Back => {
                if let Some(v) = self.back.pop() {
                    self.view = v;
                    self.issue_ui = IssueUi::default();
                    self.page_edit = None;
                }
            }
            Action::Search(q) => {
                self.jql = q;
                self.jql_ran = None;
                self.navigate(View::Issues);
            }
            Action::OpenCreate(project) => self.create = Some(issues::new_form(self, project)),
            Action::OpenNewPage { space, parent } => self.new_page = Some(NewPageForm { space, parent, ..NewPageForm::default() }),
            Action::EditPage(id) => {
                if let Ok(p) = self.session.ws.page(id) {
                    self.page_edit = Some(PageEditor {
                        id,
                        title: p.title.clone(),
                        body: p.body.to_markdown(),
                        message: String::new(),
                        base: p.version,
                        conflict: None,
                    });
                }
            }
            Action::File(r) => self.file_request = Some(r),
            Action::TogglePalette => self.palette = if self.palette.is_some() { None } else { Some(palette::Palette::default()) },
            Action::ToggleTheme => self.dark = Some(!self.dark.unwrap_or(false)),
            Action::Toast(m) => self.notify(m),
            Action::Run { id, params, then } => self.run(id, &params, then),
        }
    }

    pub(crate) fn run(&mut self, id: &str, params: &Value, then: Then) {
        match self.session.run(id, params) {
            Ok(v) => {
                self.jql_ran = None;
                // Undo can remove what the current view shows.
                self.fix_view();
                match then {
                    Then::Nothing => {}
                    Then::Toast(m) => self.notify(m),
                    Then::OpenIssue => {
                        if let Some(k) = v["key"].as_str().and_then(|k| IssueKey::parse(k).ok()) {
                            self.notify(format!("Created {k}"));
                            self.navigate(View::Issue(k));
                        }
                    }
                    Then::EndPageEdit => {
                        self.page_edit = None;
                        self.notify("Page saved");
                    }
                    Then::Nav(view) => self.navigate(view),
                }
            }
            Err(e) => {
                // A stale page save opens the conflict choice instead of a bare error.
                if id == "page.edit"
                    && let Some(ed) = &mut self.page_edit
                    && let Ok(p) = self.session.ws.page(ed.id)
                    && p.version != ed.base
                {
                    ed.conflict = Some(p.version);
                    return;
                }
                self.notify(e);
            }
        }
    }

    fn fix_view(&mut self) {
        let gone = match &self.view {
            View::Issue(k) => !self.session.ws.issues.contains_key(k),
            View::Page(id) | View::PageHistory(id) => !self.session.ws.pages.contains_key(id),
            View::Board(p) => !self.session.ws.projects.contains_key(p),
            _ => false,
        };
        if gone {
            self.view = View::Home;
            self.page_edit = None;
        }
    }

    /// The control channel / `ui_shot` entry point: `{"method": …, "params": …}`.
    ///
    /// - `engine.execute` `{"command": id, "params": {…}}`
    /// - `ui.navigate` `{"view": "home"|"issues"|"board"|"issue"|"page"|"history", "project"|"key"|"id": …}`
    /// - `ui.search` `{"jql": …}` · `ui.create` `{"project": …}` · `ui.palette` · `ui.editPage` `{"id": n}`
    /// - `ui.theme` `{"dark": bool}`
    pub fn control(&mut self, req: &Value) -> Result<Value, String> {
        let method = req.get("method").and_then(Value::as_str).unwrap_or_default();
        let p = req.get("params").cloned().unwrap_or_else(|| json!({}));
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        match method {
            "engine.execute" => {
                let id = p.get("command").and_then(Value::as_str).ok_or("missing \"command\"")?;
                let r = self.session.run(id, p.get("params").unwrap_or(&json!({})));
                self.jql_ran = None;
                self.fix_view();
                r
            }
            "ui.navigate" => {
                let v = match s("view").as_str() {
                    "home" => View::Home,
                    "issues" => View::Issues,
                    "board" => View::Board(ProjectKey::parse(&s("project").to_ascii_uppercase()).map_err(|e| e.to_string())?),
                    "issue" => View::Issue(IssueKey::parse(&s("key")).map_err(|e| e.to_string())?),
                    "page" | "history" => {
                        let id = Id(p.get("id").and_then(Value::as_u64).ok_or("missing \"id\"")?);
                        self.session.ws.page(id).map_err(|e| e.to_string())?;
                        if s("view") == "page" { View::Page(id) } else { View::PageHistory(id) }
                    }
                    other => return Err(format!("unknown view {other}")),
                };
                self.navigate(v);
                Ok(json!({"ok": true}))
            }
            "ui.search" => {
                self.apply(Action::Search(s("jql")));
                Ok(json!({"ok": true}))
            }
            "ui.create" => {
                self.apply(Action::OpenCreate(Some(s("project")).filter(|p| !p.is_empty())));
                Ok(json!({"ok": true}))
            }
            "ui.palette" => {
                self.apply(Action::TogglePalette);
                Ok(json!({"ok": true}))
            }
            "ui.editPage" => {
                let id = Id(p.get("id").and_then(Value::as_u64).ok_or("missing \"id\"")?);
                self.navigate(View::Page(id));
                self.apply(Action::EditPage(id));
                Ok(json!({"ok": true}))
            }
            "ui.theme" => {
                self.dark = p.get("dark").and_then(Value::as_bool);
                Ok(json!({"ok": true}))
            }
            other => Err(format!("unknown method {}", other.chars().take(60).collect::<String>())),
        }
    }
}

/// A shortcut for display: `Cmd+K` on macOS, `Ctrl+K` elsewhere (text, not ⌘ symbols, which
/// the bundled fonts lack).
pub(crate) fn key(k: &str) -> String {
    let m = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
    k.replace("Mod", m)
}

/// "3 days ago"-style relative time.
pub(crate) fn ago(now: trackcraft_model::Timestamp, t: trackcraft_model::Timestamp) -> String {
    let secs = (now - t).num_seconds();
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 86_400 * 30 => {
            let d = s / 86_400;
            if d == 1 { "yesterday".into() } else { format!("{d} days ago") }
        }
        _ => t.format("%b %-d, %Y").to_string(),
    }
}

#[cfg(test)]
mod tests;
