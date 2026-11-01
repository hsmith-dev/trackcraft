//! Workspace hosting and the seams the suite server replaces: [`IdentityProvider`] (who is
//! calling) and [`EventSink`] (what they changed).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use serde_json::Value;
use tokio::sync::Notify;
use trackcraft_engine::Session;

/// Credentials as a client sent them (`Authorization` header).
#[derive(Debug, Clone, Copy)]
pub enum Credential<'a> {
    /// `Authorization: Bearer <token>`.
    Bearer(&'a str),
    /// `Authorization: Basic base64(<user or email>:<token>)`, as Jira/Confluence clients send it.
    Basic { user: &'a str, secret: &'a str },
}

/// An authenticated caller: the workspace user id every command runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user: String,
}

/// Turns credentials into a user. Standalone, `trackcraft-server` uses its token file; the suite
/// server passes one shared provider to both apps so a user signs in once.
pub trait IdentityProvider: Send + Sync + 'static {
    /// `None` = not authenticated (the API answers 401).
    fn authenticate(&self, credential: &Credential<'_>) -> Option<Identity>;
}

/// One mutating command that ran successfully on behalf of a user.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandEvent {
    pub workspace: String,
    pub user: String,
    pub command: String,
    pub params: Value,
    pub result: Value,
}

/// Receives command events after the workspace lock is released. Standalone: [`NullSink`] or
/// [`MemorySink`]; the suite server's sink fans events out to the integration API (Chatcraft
/// unfurls, notifications) and, later, WebSocket clients.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &CommandEvent);
}

/// Drops every event.
pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _: &CommandEvent) {}
}

/// Keeps the most recent events in memory (tests, a future activity feed).
#[derive(Default)]
pub struct MemorySink {
    events: Mutex<Vec<CommandEvent>>,
}

impl MemorySink {
    /// Most events kept; older ones are dropped.
    pub const MAX: usize = 10_000;
    pub fn events(&self) -> Vec<CommandEvent> {
        self.events.lock().map(|e| e.clone()).unwrap_or_else(|p| p.into_inner().clone())
    }
}

impl EventSink for MemorySink {
    fn emit(&self, event: &CommandEvent) {
        let mut v = self.events.lock().unwrap_or_else(|p| p.into_inner());
        if v.len() >= Self::MAX {
            v.remove(0);
        }
        v.push(event.clone());
    }
}

/// One hosted workspace: its session behind a mutex, where it is saved, and the revision last
/// written there.
pub struct Hosted {
    name: String,
    path: Option<PathBuf>,
    session: Mutex<Session>,
    saved: AtomicU64,
}

impl Hosted {
    /// `path`: where it is saved (`None` = in memory only, e.g. tests).
    pub fn new(name: &str, session: Session, path: Option<PathBuf>) -> Self {
        let rev = session.revision();
        Hosted { name: name.to_string(), path, session: Mutex::new(session), saved: AtomicU64::new(rev) }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Locks the session. A poisoned lock (a panic while held — `Session::run` already restores
    /// the workspace on panics) is recovered rather than taking the workspace offline.
    pub fn lock(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The revision last saved to disk.
    pub fn saved_revision(&self) -> u64 {
        self.saved.load(Ordering::SeqCst)
    }

    pub fn mark_saved(&self, revision: u64) {
        self.saved.store(revision, Ordering::SeqCst);
    }
}

/// Every hosted workspace, plus a signal raised after each change (the saver waits on it).
#[derive(Default)]
pub struct Hub {
    workspaces: RwLock<BTreeMap<String, Arc<Hosted>>>,
    default: RwLock<Option<String>>,
    changed: Notify,
}

impl Hub {
    pub fn new() -> Self {
        Hub::default()
    }

    pub fn add(&self, hosted: Hosted) -> Arc<Hosted> {
        let h = Arc::new(hosted);
        let mut map = self.workspaces.write().unwrap_or_else(|p| p.into_inner());
        map.insert(h.name().to_string(), Arc::clone(&h));
        h
    }

    /// The workspace served at the root paths (`/rest/…`, `/wiki/…`). Without one, the root
    /// serves the only workspace when there is exactly one.
    pub fn set_default(&self, name: Option<&str>) {
        *self.default.write().unwrap_or_else(|p| p.into_inner()) = name.map(str::to_string);
    }

    pub fn names(&self) -> Vec<String> {
        self.workspaces.read().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect()
    }

    pub fn all(&self) -> Vec<Arc<Hosted>> {
        self.workspaces.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<Hosted>> {
        self.workspaces.read().unwrap_or_else(|p| p.into_inner()).get(name).cloned()
    }

    /// The workspace for a request: `/w/{name}/…` names it; root paths use the default.
    pub fn resolve(&self, name: Option<&str>) -> Result<Arc<Hosted>, String> {
        if let Some(n) = name {
            return self.get(n).ok_or_else(|| format!("no workspace named '{}'", n.chars().take(80).collect::<String>()));
        }
        let default = self.default.read().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(d) = default {
            return self.get(&d).ok_or_else(|| format!("the default workspace '{d}' is not hosted"));
        }
        let map = self.workspaces.read().unwrap_or_else(|p| p.into_inner());
        match (map.len(), map.values().next()) {
            (1, Some(only)) => Ok(Arc::clone(only)),
            (0, _) => Err("no workspaces are hosted: create one with `trackcraft-server workspace create NAME`".into()),
            _ => Err(format!("several workspaces are hosted: use /w/{{name}}/… (one of: {})", map.keys().cloned().collect::<Vec<_>>().join(", "))),
        }
    }

    /// Raised after every request that changed a workspace.
    pub fn notify_changed(&self) {
        self.changed.notify_one();
    }

    /// Resolves after the next change (or at once if one happened since the last wait).
    pub async fn changed(&self) {
        self.changed.notified().await;
    }
}
