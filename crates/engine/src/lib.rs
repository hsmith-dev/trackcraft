//! TrackCraft engine (L4): the session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`issue.create`, `issue.transition`, `page.edit`), a label, the menu location, a default
//! shortcut, a params description and a `run` function. The UI, shortcuts, command palette, CLI,
//! control channel, REST API and MCP server all call [`Session::run`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod authz;
pub mod catalog;
pub mod cmd;
mod json;
mod params;
pub mod sample;
pub mod sync;

use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::Value;
pub use trackcraft_model as model;
use trackcraft_model::{Timestamp, Workspace};

/// Most undo steps kept.
pub const UNDO_MAX: usize = 200;

pub type CmdResult = Result<Value, String>;

#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Where it lives in the UI: `Issue`, `Issue › Workflow`, `Page`, `File`…
    pub location: &'static str,
    /// Default shortcut in a portable form (`Mod+K`, `C`, `Mod+Shift+Z`). `Mod` = Ctrl, or ⌘ on macOS.
    pub shortcut: &'static str,
    /// Parameter description for agents (`{"key": "LUME-1", "to": "Done"}`).
    pub params: &'static str,
    pub mutates: bool,
    /// `None` = enabled; `Some(reason)` = disabled.
    pub enabled: fn(&Session) -> Option<&'static str>,
    pub run: fn(&mut Session, &Value) -> CmdResult,
}

fn always(_: &Session) -> Option<&'static str> {
    None
}

impl CommandSpec {
    pub const fn new(id: &'static str, label: &'static str, location: &'static str, run: fn(&mut Session, &Value) -> CmdResult) -> Self {
        CommandSpec { id, label, location, shortcut: "", params: "{}", mutates: true, enabled: always, run }
    }
    pub const fn key(mut self, s: &'static str) -> Self {
        self.shortcut = s;
        self
    }
    pub const fn params(mut self, p: &'static str) -> Self {
        self.params = p;
        self
    }
    /// Read-only: no undo step, no dirty flag.
    pub const fn query(mut self) -> Self {
        self.mutates = false;
        self
    }
    pub const fn when(mut self, f: fn(&Session) -> Option<&'static str>) -> Self {
        self.enabled = f;
        self
    }
}

/// All commands, in menu order.
pub fn commands() -> &'static [CommandSpec] {
    static ALL: OnceLock<Vec<CommandSpec>> = OnceLock::new();
    ALL.get_or_init(cmd::all)
}

pub fn command(id: &str) -> Option<&'static CommandSpec> {
    commands().iter().find(|c| c.id == id)
}

/// Commands that act on the host or on this session's own history rather than on the shared
/// workspace. The server API refuses them, and so does a session in remote mode
/// ([`Session::set_remote`]): `file.open` / `workspace.import` read host paths (and would not
/// replay identically elsewhere), undo would undo another user's change.
pub const LOCAL_ONLY: &[&str] = &["file.open", "file.save", "workspace.new", "workspace.import", "edit.undo", "edit.redo", "user.switch"];

/// A mutating command that changed the workspace, as the `after_command` observer sees it
/// ([`Session::set_after_command`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Executed {
    pub command: String,
    pub params: Value,
    pub actor: String,
    /// The pinned clock the command ran with.
    pub at: Timestamp,
    /// The workspace id counter before the command ([`sync::ids_next`]).
    pub ids_before: u64,
    /// [`Session::revision`] after it.
    pub revision: u64,
}

/// Observer called after each local command that changed the workspace.
pub type AfterCommand = Box<dyn FnMut(&Executed) + Send>;

/// How `Session::exec` records a command.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `Session::run`: undo step (unless remote), journal entry, observer.
    Local,
    /// Replaying someone else's command: none of those.
    Replay,
}

/// The workspace being edited, who is editing it, and its history.
pub struct Session {
    pub ws: Workspace,
    /// The acting user (reporter/author of what they create; `currentUser()` in JQL).
    pub user: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    undo: Vec<Workspace>,
    redo: Vec<Workspace>,
    /// Bumped on every change to the workspace (see [`Session::revision`]).
    revision: u64,
    clock: fn() -> Timestamp,
    /// While a command runs, every `now()` returns this (one command, one timestamp; replays
    /// pin it to the journal entry's `at`).
    pinned: Option<Timestamp>,
    /// Remote mode: no undo history, [`LOCAL_ONLY`] commands refused.
    remote: bool,
    after: Option<AfterCommand>,
    sync: sync::SyncState,
    /// Server mode: local commands are checked against the acting user's permissions and
    /// queries see only what they may browse ([`Session::set_enforce`]).
    enforce: bool,
}

pub fn system_now() -> Timestamp {
    let d = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).unwrap_or_default();
    let secs = i64::try_from(d.as_secs()).unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, d.subsec_nanos()).unwrap_or_default()
}

impl Session {
    pub fn new(ws: Workspace, user: &str) -> Self {
        Session {
            ws,
            user: user.to_string(),
            path: None,
            dirty: false,
            undo: vec![],
            redo: vec![],
            revision: 0,
            clock: system_now,
            pinned: None,
            remote: false,
            after: None,
            sync: sync::SyncState::default(),
            enforce: false,
        }
    }

    /// A session on the sample workspace, acting as its first user.
    pub fn sample() -> Self {
        let ws = sample::workspace(system_now());
        let user = ws.users.keys().next().cloned().unwrap_or_default();
        Session::new(ws, &user)
    }

    /// Replaces the clock (tests, deterministic replays).
    pub fn with_clock(mut self, clock: fn() -> Timestamp) -> Self {
        self.clock = clock;
        self
    }

    pub fn now(&self) -> Timestamp {
        self.pinned.unwrap_or_else(|| (self.clock)())
    }

    /// Remote mode (a client of a sync server): local commands keep no undo history (undo would
    /// revert other people's changes) and [`LOCAL_ONLY`] commands are refused. Turning it on
    /// drops the current undo/redo stacks.
    pub fn set_remote(&mut self, on: bool) {
        self.remote = on;
        if on {
            self.undo.clear();
            self.redo.clear();
        }
    }

    pub fn is_remote(&self) -> bool {
        self.remote
    }

    /// Installs (or with `None` removes) the observer called after every local command that
    /// changed the workspace — how a sync client forwards commands without the UI knowing.
    /// Replays ([`Session::apply_remote`]) never call it.
    pub fn set_after_command(&mut self, f: Option<AfterCommand>) {
        self.after = f;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// A change counter: it rises on every mutating command that changed the workspace, every
    /// undo and redo, and when a workspace is opened or created. Autosave waits until it stops
    /// moving; a UI compares it to know when its cached views are stale.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// How many steps `edit.undo` can take (a toast's Undo is live while this is unchanged).
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// How many steps `edit.redo` can take.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Runs a command by id. A failing command leaves the workspace exactly as it was; an escaped
    /// panic (a bug) is turned into an error and the workspace is restored.
    pub fn run(&mut self, id: &str, params: &Value) -> CmdResult {
        self.exec(id, params, Mode::Local).map(|(v, _)| v)
    }

    /// The one place commands run. Returns the result and whether the workspace changed.
    fn exec(&mut self, id: &str, params: &Value, mode: Mode) -> Result<(Value, bool), String> {
        let spec = command(id).ok_or_else(|| format!("unknown command '{}'", id.chars().take(80).collect::<String>()))?;
        if (self.remote || mode == Mode::Replay) && LOCAL_ONLY.contains(&spec.id) {
            return Err(format!("{} is not available while connected to a server", spec.id));
        }
        if let Some(why) = (spec.enabled)(self) {
            return Err(format!("{} is unavailable: {why}", spec.id));
        }
        // Permissions (servers): mutations are checked, queries run on the actor's view.
        let enforced = self.enforce && mode == Mode::Local && !LOCAL_ONLY.contains(&spec.id);
        if enforced && spec.mutates {
            self.authorize(&self.user, spec.id, params).map_err(|d| d.to_string())?;
        }
        let full = if enforced && !spec.mutates { self.ws.view_for(&self.user).map(|v| std::mem::replace(&mut self.ws, v)) } else { None };
        // One command, one timestamp (a replay has already pinned the entry's).
        let pinned_here = self.pinned.is_none();
        let at = self.now();
        self.pinned = Some(at);
        let ids_before = sync::ids_next(&self.ws);
        let before = spec.mutates.then(|| self.ws.clone());
        let scope = (mode == Mode::Local && spec.mutates && self.sync.journaling()).then(|| authz::scope(&self.ws, spec.id, params));
        // Every history entry one command records shares one change id.
        if spec.mutates {
            self.ws.begin_batch();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, params)));
        self.ws.end_batch();
        if let Some(full) = full {
            self.ws = full;
        }
        if pinned_here {
            self.pinned = None;
        }
        let result = match result {
            Ok(r) => r,
            Err(_) => Err(format!("internal error in {}: the workspace was restored (please report this)", spec.id)),
        };
        let mut changed = false;
        if let Some(before) = before {
            match &result {
                Ok(_) if before != self.ws => {
                    changed = true;
                    if mode == Mode::Local && !self.remote {
                        self.undo.push(before);
                        if self.undo.len() > UNDO_MAX {
                            self.undo.remove(0);
                        }
                        self.redo.clear();
                    }
                    self.dirty = true;
                    self.bump();
                    if mode == Mode::Local {
                        self.record(spec.id, params, at, ids_before, scope.unwrap_or_default());
                    }
                }
                Ok(_) => {}
                Err(_) => self.ws = before,
            }
        }
        result.map(|v| (v, changed))
    }

    /// After a local change: the journal entry (servers) and the observer (sync clients).
    fn record(&mut self, id: &str, params: &Value, at: Timestamp, ids_before: u64, scope: authz::Scope) {
        let actor = self.user.clone();
        let ids_after = sync::ids_next(&self.ws);
        let inbox_after = self.ws.inbox.next_id;
        self.sync.record(id, params, &actor, at, ids_before, sync::Meta { scope, ids_after, inbox_after });
        if let Some(f) = self.after.as_mut() {
            f(&Executed { command: id.to_string(), params: params.clone(), actor, at, ids_before, revision: self.revision });
        }
    }

    pub(crate) fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.ws, prev));
                self.dirty = true;
                self.bump();
                true
            }
            None => false,
        }
    }

    pub(crate) fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.ws, next));
                self.dirty = true;
                self.bump();
                true
            }
            None => false,
        }
    }

    /// Clears undo history (after open/new).
    pub(crate) fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.bump();
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_access;
#[cfg(test)]
mod tests_notify;
#[cfg(test)]
mod tests_reports; // M2.5
#[cfg(test)]
mod tests_sync;
#[cfg(test)]
mod tests_wiki; // M4.5
