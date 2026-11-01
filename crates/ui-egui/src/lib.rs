//! TrackCraft's egui front end (L6). Thin by design: views read `Session` state and change it
//! only through [`Session::run`] — the same commands as the CLI, MCP and control channel.
//!
//! Each frame, views push [`Action`]s; [`TrackApp::ui`] applies them after drawing, so no view
//! holds a mutable borrow of the session while it paints.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod backlog; // UI-11
#[doc(hidden)]
pub mod fixtures; // UI-6
mod guard;
pub mod icons;
mod issues;
mod keys;
pub mod markup_view;
mod order;
mod palette;
mod route;
mod shell;
mod sprints; // UI-12
pub mod theme;
mod widgets;
mod wiki;

use serde_json::{Value, json};
use trackcraft_core::{Id, IssueKey};
use trackcraft_engine::Session;

pub use craft_ui::ComboBox;
pub use route::View;
use theme::Tokens;

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
    Run {
        id: &'static str,
        params: Value,
        then: Then,
    },
    Search(String),
    OpenCreate(Option<String>),
    OpenNewPage {
        space: String,
        parent: Option<Id>,
    },
    EditPage(Id),
    File(FileRequest),
    TogglePalette,
    ShowHelp,
    ToggleTheme,
    Toast(String),
    /// Close the create dialog (Escape or a backdrop click): asks first if it has typed text.
    CloseCreate,
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
    /// Focus the comment box next frame (`m`).
    pub focus_comment: bool,
    /// Focus the description editor next frame (`e`).
    pub focus_description: bool,
    /// Open the assignee picker next frame (`a`).
    pub open_assignee: bool,
    /// Open the status (transition) menu next frame (`t`).
    pub open_status: bool,
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

impl CreateForm {
    /// Typed text that closing the dialog would throw away.
    pub(crate) fn dirty(&self) -> bool {
        !self.summary.trim().is_empty() || !self.description.trim().is_empty()
    }
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
    /// The title and body as loaded, to tell whether anything was typed.
    pub loaded_title: String,
    pub loaded_body: String,
}

impl PageEditor {
    /// The editor holds text that differs from what was loaded.
    pub(crate) fn dirty(&self) -> bool {
        self.title != self.loaded_title || self.body != self.loaded_body || !self.message.trim().is_empty()
    }
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
    /// The keyboard-selected row of the issue list.
    pub(crate) list_sel: Option<IssueKey>,
    /// The keyboard-selected card on the board.
    pub(crate) board_sel: Option<IssueKey>,
    /// Scroll the selected row or card into view next frame.
    pub(crate) scroll_sel: bool,
    /// The keyboard shortcut overlay is open.
    pub(crate) help: bool,
    /// Actions held back because they would discard typed text; the "Discard your changes?"
    /// dialog is open while this is set (see `guard.rs`).
    pub(crate) guard: Option<guard::Guard>,
    /// Key events queued by the control channel (`ui.key`), fed in at the next frame.
    injected: Vec<egui::Event>,
    toast: Option<(String, f64)>,
    pending: Vec<Action>,
    file_request: Option<FileRequest>,
    focus_search: bool,
    // UI-3: issue list paging. `jql_start` is the first row of the shown page (of `jql_paged`;
    // a new query starts at 0), `jql_total` every match, `jql_ran_start` the cached page.
    pub(crate) jql_start: usize,
    pub(crate) jql_total: usize,
    jql_paged: String,
    jql_ran_start: usize,
    /// The labels prompt on a page (`l`): the page and the labels typed so far.
    pub(crate) page_labels: Option<(Id, String)>,
    /// The first key of a two-key chord (`G then I`) and when it was pressed.
    pub(crate) chord: Option<(keys::Chord, f64)>,
    /// Backlog selection, collapsed sections, inline create and the move picker (UI-11).
    pub(crate) backlog: backlog::BacklogUi,
    /// The open sprint dialog: start, complete, create, edit or delete (UI-12).
    pub(crate) sprint_dialog: Option<sprints::SprintDialog>,
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
            list_sel: None,
            board_sel: None,
            scroll_sel: false,
            help: false,
            guard: None,
            injected: vec![],
            toast: None,
            pending: vec![],
            file_request: None,
            focus_search: false,
            // UI-3
            jql_start: 0,
            jql_total: 0,
            jql_paged: String::new(),
            jql_ran_start: 0,
            page_labels: None,
            chord: None,
            backlog: backlog::BacklogUi::default(), // UI-11
            sprint_dialog: None,                    // UI-12
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

    /// Shows `v`, dropping any open edits (the UI asks first: see [`Action::Nav`] and `guard.rs`).
    /// Showing the current view again changes nothing.
    pub fn navigate(&mut self, v: View) {
        if v == self.view {
            return;
        }
        self.back.push(std::mem::replace(&mut self.view, v));
        if self.back.len() > 100 {
            self.back.remove(0);
        }
        self.left_view();
    }

    /// Clears the per-view state after the view changed (the page editor survives only on its page).
    fn left_view(&mut self) {
        self.issue_ui = IssueUi::default();
        self.history_preview = None;
        if !self.page_edit.as_ref().is_some_and(|e| self.view == View::Page(e.id)) {
            self.page_edit = None;
        }
    }

    /// Runs the JQL in the search field, one page of [`LIST_PAGE`] rows from `jql_start`
    /// (cached until the query, the page or the workspace changes).
    pub(crate) fn refresh_search(&mut self) {
        if self.jql_paged != self.jql {
            self.jql_paged = self.jql.clone();
            self.jql_start = 0;
        }
        // `jql_ran` is cleared after every command, so a match means nothing changed.
        let key = self.jql.clone();
        if self.jql_ran.as_deref() == Some(key.as_str()) && self.jql_ran_start == self.jql_start {
            return;
        }
        let run = |s: &mut Session, start: usize| s.run("search.jql", &json!({"jql": key, "startAt": start, "maxResults": LIST_PAGE}));
        let mut res = run(&mut self.session, self.jql_start);
        // A page past the end (issues were deleted, or a hostile start): show the last page.
        if let Ok(v) = &res {
            let total = v["total"].as_u64().map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
            if self.jql_start > 0 && self.jql_start >= total {
                self.jql_start = total.saturating_sub(1) / LIST_PAGE * LIST_PAGE;
                res = run(&mut self.session, self.jql_start);
            }
        }
        self.jql_result = match res {
            Ok(v) => {
                self.jql_total = v["total"].as_u64().map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
                Ok(v["issues"].as_array().into_iter().flatten().filter_map(|i| i["key"].as_str()).filter_map(|k| IssueKey::parse(k).ok()).collect())
            }
            Err(e) => {
                self.jql_total = 0;
                Err(e)
            }
        };
        self.jql_ran = Some(key);
        self.jql_ran_start = self.jql_start;
    }

    /// Per-frame logic without drawing (shortcuts, toasts).
    pub fn logic(&mut self, ctx: &egui::Context) {
        let t = self.tokens(ctx);
        ctx.set_visuals(t.visuals());
        let typing = ctx.egui_wants_keyboard_input();
        let guarding = self.guard.is_some();
        let dialog = self.create.is_some()
            || self.palette.is_some()
            || self.new_page.is_some()
            || self.page_labels.is_some()
            || self.sprint_dialog.is_some()
            || self.backlog.move_pick.is_some();
        let modal = dialog || self.help || guarding;
        let gate = keys::Gate { typing, modal, help_only: self.help && !dialog && !guarding, guarding };
        let injected = std::mem::take(&mut self.injected);
        let fired = ctx.input_mut(|i| {
            i.events.extend(injected);
            use egui::{Key, Modifiers};
            // The scoped keymap (see `keys.rs`): registry shortcuts plus UI-only bindings.
            let fired = keys::dispatch(self, i, gate);
            // The backlog's own keys (listed in the overlay under the Backlog scope).
            if !typing
                && !modal
                && let View::Backlog(p) = self.view.clone()
            {
                backlog::keys(self, i, &p);
            }
            if guarding {
                // Escape on "Discard your changes?" is "Keep editing"; consumed so the dialog
                // underneath does not also see it.
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    self.guard = None;
                }
            } else if i.key_pressed(Key::Escape) {
                if modal {
                    if self.create.as_ref().is_some_and(CreateForm::dirty) {
                        // Typed text is kept: the guard asks before the dialog closes.
                        i.consume_key(Modifiers::NONE, Key::Escape);
                        self.hold(Action::CloseCreate);
                    } else {
                        self.create = None;
                    }
                    self.palette = None;
                    self.new_page = None;
                    self.page_labels = None;
                    self.sprint_dialog = None;
                    self.backlog.move_pick = None;
                    self.help = false;
                } else if matches!(self.view, View::Issue(_)) {
                    keys::cancel_issue_edits(self);
                }
            }
            fired
        });
        for act in fired {
            keys::run(self, act);
        }
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
                View::Backlog(p) => backlog::backlog(self, ui, &t, &p),                 // UI-11
                View::SprintBoard(p, id) => issues::sprint_board(self, ui, &t, &p, id), // UI-12
            }
        });
        issues::create_dialog(self, ui.ctx(), &t);
        wiki::new_page_dialog(self, ui.ctx(), &t);
        keys::labels_dialog(self, ui.ctx(), &t);
        palette::show(self, ui.ctx(), &t);
        keys::help(self, ui.ctx(), &t);
        guard::dialog(self, ui.ctx(), &t);
        backlog::move_picker(self, ui.ctx(), &t); // UI-11
        sprints::dialog(self, ui.ctx(), &t); // UI-12
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
            Action::Nav(v) => {
                if self.would_lose(&v) {
                    self.hold(Action::Nav(v));
                } else {
                    self.navigate(v);
                }
            }
            Action::Back => {
                let Some(to) = self.back.last() else { return };
                if self.would_lose(to) {
                    self.hold(Action::Back);
                } else if let Some(v) = self.back.pop()
                    && v != self.view
                {
                    self.view = v;
                    self.left_view();
                }
            }
            Action::Search(q) => {
                if self.would_lose(&View::Issues) {
                    self.hold(Action::Search(q));
                    return;
                }
                self.jql = q;
                self.jql_ran = None;
                self.navigate(View::Issues);
            }
            Action::CloseCreate => {
                if self.create.as_ref().is_some_and(CreateForm::dirty) {
                    self.hold(Action::CloseCreate);
                } else {
                    self.create = None;
                }
            }
            Action::OpenCreate(project) => self.create = Some(issues::new_form(self, project)),
            Action::OpenNewPage { space, parent } => self.new_page = Some(NewPageForm { space, parent, ..NewPageForm::default() }),
            Action::EditPage(id) => {
                // Reloading the same page is the conflict banner's explicit "Discard mine".
                if self.page_edit.as_ref().is_some_and(|e| e.id != id && e.dirty()) {
                    self.hold(Action::EditPage(id));
                    return;
                }
                if let Ok(p) = self.session.ws.page(id) {
                    let body = p.body.to_markdown();
                    self.page_edit = Some(PageEditor {
                        id,
                        title: p.title.clone(),
                        body: body.clone(),
                        message: String::new(),
                        base: p.version,
                        conflict: None,
                        loaded_title: p.title.clone(),
                        loaded_body: body,
                    });
                }
            }
            Action::File(r) => self.file_request = Some(r),
            Action::TogglePalette => self.palette = if self.palette.is_some() { None } else { Some(palette::Palette::default()) },
            Action::ShowHelp => self.help = true,
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
                            self.apply(Action::Nav(View::Issue(k)));
                        }
                    }
                    Then::EndPageEdit => {
                        self.page_edit = None;
                        self.notify("Page saved");
                    }
                    Then::Nav(view) => self.apply(Action::Nav(view)),
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
            View::Backlog(p) => !self.session.ws.projects.contains_key(p), // UI-11
            View::SprintBoard(_, id) => !self.session.ws.sprints.contains_key(id), // UI-12
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
    /// - `ui.navigate` `{"view": "home"|"issues"|"board"|"issue"|"page"|"history", "project"|"key"|"id": …, "force": bool}`
    /// - `ui.search` `{"jql": …, "force": bool}` · `ui.create` `{"project": …}` · `ui.palette` · `ui.editPage` `{"id": n, "force": bool}`
    ///
    /// Requests that would leave typed, unsaved text (page editor, issue summary/description)
    /// fail with "unsaved changes" instead of opening a dialog; `"force": true` discards the text.
    /// - `ui.theme` `{"dark": bool}`
    /// - `ui.key` `{"key": "j"|"ArrowDown"|"Enter"|"?"…, "shift"|"alt"|"command": bool}`: a key press at the next frame
    /// - `ui.sprintDialog` `{"kind": "start"|"complete"|"create"|"edit"|"delete", "id": n, "project": "LUME"}`:
    ///   opens a sprint dialog (`create` takes `project`; the others `id`, or `project` for its
    ///   active sprint (complete) or next planned one) (UI-12)
    pub fn control(&mut self, req: &Value) -> Result<Value, String> {
        let method = req.get("method").and_then(Value::as_str).unwrap_or_default();
        let p = req.get("params").cloned().unwrap_or_else(|| json!({}));
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let force = p.get("force").and_then(Value::as_bool).unwrap_or(false);
        match method {
            "engine.execute" => {
                let id = p.get("command").and_then(Value::as_str).ok_or("missing \"command\"")?;
                let r = self.session.run(id, p.get("params").unwrap_or(&json!({})));
                self.jql_ran = None;
                self.fix_view();
                r
            }
            "ui.navigate" => {
                let v = route::parse(&self.session.ws, &p)?;
                self.leave_for_control(self.would_lose(&v), force)?;
                self.navigate(v);
                Ok(json!({"ok": true}))
            }
            "ui.search" => {
                self.leave_for_control(self.would_lose(&View::Issues), force)?;
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
                let loses = self.would_lose(&View::Page(id)) || self.page_edit.as_ref().is_some_and(PageEditor::dirty);
                self.leave_for_control(loses, force)?;
                self.navigate(View::Page(id));
                self.apply(Action::EditPage(id));
                Ok(json!({"ok": true}))
            }
            "ui.key" => {
                let name = s("key");
                let key = egui::Key::from_name(&name).ok_or_else(|| format!("unknown key {}", name.chars().take(30).collect::<String>()))?;
                let on = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
                let mut modifiers = egui::Modifiers::NONE;
                for (k, m) in [("alt", egui::Modifiers::ALT), ("shift", egui::Modifiers::SHIFT), ("command", egui::Modifiers::COMMAND)] {
                    if on(k) {
                        modifiers |= m;
                    }
                }
                if self.injected.len() < 64 {
                    for pressed in [true, false] {
                        self.injected.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
                    }
                }
                Ok(json!({"ok": true}))
            }
            "ui.theme" => {
                self.dark = p.get("dark").and_then(Value::as_bool);
                Ok(json!({"ok": true}))
            }
            // UI-12
            "ui.sprintDialog" => {
                let id = p.get("id").and_then(Value::as_u64).map(Id);
                self.sprint_dialog = Some(sprints::SprintDialog::open(&self.session, &s("kind"), id, &s("project"))?);
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

// UI-3: rows per issue-list page (the engine's `search.jql` cap).
pub(crate) const LIST_PAGE: usize = 1000;

#[cfg(test)]
mod tests;
