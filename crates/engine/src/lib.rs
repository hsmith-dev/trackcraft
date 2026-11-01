//! TrackCraft engine (L4): the session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`issue.create`, `issue.transition`, `page.edit`), a label, the menu location, a default
//! shortcut, a params description and a `run` function. The UI, shortcuts, command palette, CLI,
//! control channel, REST API and MCP server all call [`Session::run`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod catalog;
pub mod cmd;
mod json;
mod params;
pub mod sample;

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

/// The workspace being edited, who is editing it, and its history.
pub struct Session {
    pub ws: Workspace,
    /// The acting user (reporter/author of what they create; `currentUser()` in JQL).
    pub user: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    undo: Vec<Workspace>,
    redo: Vec<Workspace>,
    clock: fn() -> Timestamp,
}

pub fn system_now() -> Timestamp {
    let d = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).unwrap_or_default();
    let secs = i64::try_from(d.as_secs()).unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, d.subsec_nanos()).unwrap_or_default()
}

impl Session {
    pub fn new(ws: Workspace, user: &str) -> Self {
        Session { ws, user: user.to_string(), path: None, dirty: false, undo: vec![], redo: vec![], clock: system_now }
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
        (self.clock)()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Runs a command by id. A failing command leaves the workspace exactly as it was; an escaped
    /// panic (a bug) is turned into an error and the workspace is restored.
    pub fn run(&mut self, id: &str, params: &Value) -> CmdResult {
        let spec = command(id).ok_or_else(|| format!("unknown command '{}'", id.chars().take(80).collect::<String>()))?;
        if let Some(why) = (spec.enabled)(self) {
            return Err(format!("{} is unavailable: {why}", spec.id));
        }
        let before = spec.mutates.then(|| self.ws.clone());
        // Every history entry one command records shares one change id.
        if spec.mutates {
            self.ws.begin_batch();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, params)));
        self.ws.end_batch();
        let result = match result {
            Ok(r) => r,
            Err(_) => Err(format!("internal error in {}: the workspace was restored (please report this)", spec.id)),
        };
        if let Some(before) = before {
            match &result {
                Ok(_) if before != self.ws => {
                    self.undo.push(before);
                    if self.undo.len() > UNDO_MAX {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                    self.dirty = true;
                }
                Ok(_) => {}
                Err(_) => self.ws = before,
            }
        }
        result
    }

    pub(crate) fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.ws, prev));
                self.dirty = true;
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
    }
}

#[cfg(test)]
mod tests;
