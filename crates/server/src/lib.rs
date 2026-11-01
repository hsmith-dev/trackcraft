//! The mountable TrackCraft server (L6 — it sits above `trackcraft-api`, L5): hosts the
//! workspaces in a data directory, authenticates API tokens, applies request limits, logs
//! requests and saves changes. `apps/trackcraft-server` is a thin binary over it; the suite
//! server mounts [`Server::router`] next to Chatcraft's (see `docs/server.md`).
//!
//! This crate spawns tasks on the caller's Tokio runtime but never creates one.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod config;
pub mod static_files;
pub mod ticket;
pub mod workspaces;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
pub use trackcraft_api as api;
use trackcraft_api::{ApiError, ApiState, EventSink, Flavor, Hub, IdentityProvider, LocalSyncHub, NullSink, SyncHub};

pub use config::{Config, TokenIdentity, TokenRecord, mint_token, revoke_token};

#[derive(Debug)]
pub enum ServerError {
    Io { path: String, source: std::io::Error },
    Config(String),
    Store(String),
}

impl ServerError {
    pub fn io(path: &Path, source: std::io::Error) -> Self {
        ServerError::Io { path: path.display().to_string(), source }
    }
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Io { path, source } => write!(f, "{path}: {source}"),
            ServerError::Config(m) | ServerError::Store(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for ServerError {}

/// Request limits and behaviour.
#[derive(Debug, Clone)]
pub struct Options {
    /// Largest request body accepted (413 above it).
    pub max_body_bytes: usize,
    /// Longest a request may take (503 after it; the command itself still finishes or fails
    /// atomically under the workspace lock).
    pub timeout: Duration,
    /// How long the saver waits after a change before writing (changes in the window are
    /// written together).
    pub save_debounce: Duration,
    /// Log one line per request to stderr.
    pub log_requests: bool,
    /// Public origin for `self` links; `None` = from the `Host` header.
    pub base_url: Option<String>,
    /// A static directory (the web client's `dist/web`) served for paths no API route matches
    /// (`serve --web DIR`, `static_files`); `None` serves nothing but the API.
    pub web: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_body_bytes: 4 * 1024 * 1024,
            timeout: Duration::from_secs(30),
            save_debounce: Duration::from_millis(500),
            log_requests: true,
            base_url: None,
            web: None,
        }
    }
}

/// A configured server: the hub of workspaces, identity, event sink and options.
#[derive(Clone)]
pub struct Server {
    pub hub: Arc<Hub>,
    pub identity: Arc<dyn IdentityProvider>,
    pub events: Arc<dyn EventSink>,
    /// Live-sync fan-out (M4.2, `docs/sync.md`); [`Server::with_sync_hub`] replaces it.
    pub sync: Arc<dyn SyncHub>,
    pub options: Options,
    pub data: Option<PathBuf>,
    flushing: Arc<Mutex<()>>,
}

impl Server {
    /// A server over an existing hub (tests, embedding, the suite server).
    pub fn new(hub: Arc<Hub>, identity: Arc<dyn IdentityProvider>, events: Arc<dyn EventSink>, options: Options) -> Self {
        Server { hub, identity, events, sync: Arc::new(LocalSyncHub::new()), options, data: None, flushing: Arc::new(Mutex::new(())) }
    }

    /// Uses a shared sync hub (the suite server) instead of the built-in one.
    pub fn with_sync_hub(mut self, sync: Arc<dyn SyncHub>) -> Self {
        self.sync = sync;
        self
    }

    /// Opens a data directory: `server.json` (tokens, default workspace, base URL) and every
    /// `NAME.track` in it.
    pub fn open(data: &Path, mut options: Options) -> Result<Self, ServerError> {
        std::fs::create_dir_all(data).map_err(|e| ServerError::io(data, e))?;
        let cfg = Config::load(data)?;
        let hub = Arc::new(Hub::new());
        workspaces::load_all(data, &hub)?;
        if let Some(d) = &cfg.default_workspace {
            if hub.get(d).is_none() {
                return Err(ServerError::Config(format!("defaultWorkspace '{d}' has no {d}.track in {}", data.display())));
            }
            hub.set_default(Some(d));
        }
        if options.base_url.is_none() {
            options.base_url = cfg.base_url.clone();
        }
        let identity = Arc::new(TokenIdentity::from_dir(data)?);
        let mut s = Server::new(hub, identity, Arc::new(NullSink), options);
        s.data = Some(data.to_path_buf());
        Ok(s)
    }

    pub fn api_state(&self) -> ApiState {
        ApiState {
            hub: Arc::clone(&self.hub),
            identity: Arc::clone(&self.identity),
            events: Arc::clone(&self.events),
            sync: Arc::clone(&self.sync),
            base_url: self.options.base_url.clone(),
        }
    }

    /// The full router: every API route plus the body limit, timeout and request log.
    pub fn router(&self) -> Router {
        router(self)
    }

    /// Writes every changed workspace now. Safe to call concurrently with the saver.
    pub fn flush(&self) -> Result<usize, ServerError> {
        let _guard = self.flushing.lock().unwrap_or_else(|p| p.into_inner());
        workspaces::flush(&self.hub)
    }

    /// Spawns the debounced saver on the current Tokio runtime: after each change it waits
    /// `save_debounce`, then writes what changed (atomically).
    pub fn spawn_saver(&self) -> tokio::task::JoinHandle<()> {
        let s = self.clone();
        tokio::spawn(async move {
            loop {
                s.hub.changed().await;
                tokio::time::sleep(s.options.save_debounce).await;
                let me = s.clone();
                match tokio::task::spawn_blocking(move || me.flush()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => eprintln!("trackcraft-server: save failed: {e}"),
                    Err(_) => eprintln!("trackcraft-server: save task failed"),
                }
            }
        })
    }

    /// Serves on `listener` until `shutdown` resolves, then saves everything.
    pub async fn serve(&self, listener: tokio::net::TcpListener, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), ServerError> {
        let saver = self.spawn_saver();
        let served = axum::serve(listener, self.router()).with_graceful_shutdown(shutdown).await;
        saver.abort();
        let flushed = self.flush();
        served.map_err(|e| ServerError::Config(format!("server error: {e}")))?;
        flushed.map(|_| ())
    }
}

/// The mountable router for a server: `trackcraft_api::router` with the server's limits.
pub fn router(server: &Server) -> Router {
    let o = &server.options;
    static_files::with_web(ticket::api_router(server), o.web.as_deref())
        .layer(DefaultBodyLimit::max(o.max_body_bytes))
        .layer(from_fn_with_state(o.timeout, timeout))
        .layer(from_fn_with_state(o.log_requests, log))
}

/// Mounts a server under `prefix` in a host router (the suite server: `/track` next to
/// Chatcraft's `/chat`). Set `Options::base_url` to the prefixed origin so `self` links match.
pub fn mount(host: Router, prefix: &str, server: &Server) -> Router {
    host.nest(prefix, server.router())
}

async fn timeout(State(limit): State<Duration>, req: Request, next: Next) -> Response {
    let flavor = Flavor::of(req.uri().path());
    match tokio::time::timeout(limit, next.run(req)).await {
        Ok(r) => r,
        Err(_) => ApiError::new(flavor, StatusCode::SERVICE_UNAVAILABLE, "the request timed out").into_response(),
    }
}

/// One line per request: method, path (no query string — it can carry search text), status, time.
async fn log(State(enabled): State<bool>, req: Request, next: Next) -> Response {
    if !enabled {
        return next.run(req).await;
    }
    let (method, path) = (req.method().clone(), req.uri().path().chars().take(300).collect::<String>());
    let t = Instant::now();
    let r = next.run(req).await;
    eprintln!(
        "{} {method} {path} {} {}ms",
        trackcraft_engine::system_now().format("%Y-%m-%dT%H:%M:%SZ"),
        r.status().as_u16(),
        t.elapsed().as_millis()
    );
    r
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_access;
#[cfg(test)]
mod tests_sync;
