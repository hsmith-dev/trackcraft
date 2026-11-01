//! Remote mode (M4.2): `trackcraft --server https://host[:port] --workspace team --token TOKEN`
//! (`http://` for a server on this computer).
//!
//! The sync logic lives in `trackcraft-sync-client` (tested against an in-process server); this
//! file only resolves the token, wires the repaint waker, turns sync notices into toasts and
//! (M4.7) reports the connection to the status bar indicator, whose "Reconnect now" it polls.
//! The UI keeps running commands through `Session::run`; the session's after-command observer
//! (installed by `Remote::connect`) forwards them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use trackcraft_engine::Session;
use trackcraft_sync_client::{Config, ConnState, Notice, Remote};
use trackcraft_ui_egui::{ConnectionKind, ConnectionState, TrackApp};

/// Name of the token file in the config folder (`paths::config_dir`).
pub const TOKEN_FILE: &str = "server-token";

/// The token: `--token`, else `TRACKCRAFT_TOKEN`, else the first line of the token file.
pub fn resolve_token(flag: Option<&str>, env: Option<&str>, file: Option<&Path>) -> Result<String, String> {
    if let Some(t) = flag.or(env).map(str::trim).filter(|t| !t.is_empty()) {
        return Ok(t.to_string());
    }
    if let Some(f) = file {
        match std::fs::read_to_string(f) {
            Ok(s) => {
                if let Some(t) = s.lines().next().map(str::trim).filter(|t| !t.is_empty()) {
                    return Ok(t.chars().take(1024).collect());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", f.display())),
        }
    }
    Err(format!(
        "--server needs a token: pass --token, set TRACKCRAFT_TOKEN, or put it in {}",
        file.map(|f| f.display().to_string()).unwrap_or_else(|| TOKEN_FILE.to_string())
    ))
}

pub fn token_path() -> Option<PathBuf> {
    crate::paths::config_dir().map(|d| d.join(TOKEN_FILE))
}

/// The sync handle plus the context to repaint when the network has news.
pub struct RemoteMode {
    remote: Remote,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl RemoteMode {
    /// Fetches the workspace (fails fast if the server can't be reached) and starts syncing.
    pub fn connect(server: &str, workspace: Option<&str>, token: &str) -> Result<(RemoteMode, Session), String> {
        let ctx: Arc<OnceLock<egui::Context>> = Arc::default();
        let waker = Arc::clone(&ctx);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if let Some(c) = waker.get() {
                c.request_repaint();
            }
        });
        let (remote, session) = Remote::connect(Config::new(server, workspace, token), Some(wake))?;
        Ok((RemoteMode { remote, ctx }, session))
    }

    pub fn set_context(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    /// Syncs once (every frame): local commands out, server entries in, notices as toasts, the
    /// connection state to the status bar.
    pub fn pump(&mut self, app: &mut TrackApp, ctx: &egui::Context) {
        // M4.7: "Reconnect now" on the indicator.
        if app.take_reconnect_request() {
            self.remote.reconnect_now();
            app.notify(format!("Reconnecting to {}", self.remote.server()));
        }
        for n in self.remote.pump(&mut app.session) {
            let msg = n.message(self.remote.server());
            match n {
                Notice::Dropped(_) | Notice::Resync(_) | Notice::State(ConnState::Unauthorized { .. }) => app.notify_error(msg),
                Notice::State(_) => app.notify(msg),
            }
        }
        app.set_remote(true);
        let state = self.connection_state();
        if app.connection() != Some(&state) {
            app.set_connection(Some(state));
        }
        // Retries and reconnects happen off-thread; keep the status fresh while not online.
        if self.remote.state() != &ConnState::Online || self.remote.pending() > 0 {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// The status bar indicator: kind, label and a tooltip with the server, the changes waiting
    /// to be sent and the last error (M4.7).
    pub fn connection_state(&self) -> ConnectionState {
        connection_state(&self.remote)
    }

    /// Plain `http://` to another computer (warned about at start and in the tooltip).
    pub fn unencrypted(&self) -> bool {
        self.remote.unencrypted_remote()
    }

    /// Appended to the window title: the connection state and the server.
    pub fn title_suffix(&self) -> String {
        format!(" — {} ({})", self.remote.status_text(), self.remote.server())
    }
}

/// [`RemoteMode::connection_state`], from the sync handle alone (testable without a window).
fn connection_state(r: &Remote) -> ConnectionState {
    let pending = r.pending();
    let kind = match r.state() {
        ConnState::Online if pending == 0 => ConnectionKind::Online,
        ConnState::Online | ConnState::Resyncing => ConnectionKind::Syncing,
        ConnState::Offline { .. } => ConnectionKind::Offline,
        ConnState::Unauthorized { .. } => ConnectionKind::Error,
    };
    let mut detail = vec![format!("Server: {}", r.server())];
    detail.push(match pending {
        0 => "All changes are on the server".to_string(),
        1 => "1 change waiting to be sent".to_string(),
        n => format!("{n} changes waiting to be sent"),
    });
    if let Some(e) = r.last_error() {
        detail.push(format!("Last error: {e}"));
    }
    if r.unencrypted_remote() {
        detail.push("Not encrypted (http://): use https:// for a server on another computer".into());
    }
    ConnectionState::new(kind, r.status_text(), detail.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_sources_in_order() {
        let d = std::env::temp_dir().join(format!("trackcraft-token-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(TOKEN_FILE);
        std::fs::write(&f, "  tct_file  \nignored\n").unwrap();
        assert_eq!(resolve_token(Some("tct_flag"), Some("tct_env"), Some(&f)).unwrap(), "tct_flag");
        assert_eq!(resolve_token(None, Some("tct_env"), Some(&f)).unwrap(), "tct_env");
        assert_eq!(resolve_token(None, Some("  "), Some(&f)).unwrap(), "tct_file");
        assert!(resolve_token(None, None, Some(&d.join("missing"))).unwrap_err().contains("--token"));
        assert!(resolve_token(None, None, None).is_err());
        std::fs::write(&f, "\n").unwrap();
        assert!(resolve_token(None, None, Some(&f)).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn https_connect_errors_are_clear() {
        // Nothing listens on port 1: the error names the server; no fallback to http://.
        let e = RemoteMode::connect("https://127.0.0.1:1", Some("team"), "tct_x").err().unwrap_or_default();
        assert!(e.contains("127.0.0.1"), "{e}");
        assert!(RemoteMode::connect("ftp://h", None, "tct_x").is_err());
    }
}
