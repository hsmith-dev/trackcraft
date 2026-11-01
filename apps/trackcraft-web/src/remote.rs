//! Server workspaces in the browser: the hook for M4.2 (sync).
//!
//! When the page is served by `trackcraft-server serve --web DIR`, the browser shell detects the
//! server (`GET api/health`, same origin) and tells the user. Opening and editing a workspace that
//! lives on the server needs the sync protocol (M4.2): an authenticated snapshot fetch, then a
//! stream of operations both ways. Until that lands, [`open_server_workspace`] is the single
//! place it plugs in, and it returns an error instead of guessing at a protocol.
// Nothing in the browser shell calls the hook yet (see the TODO below); tests do.
#![cfg_attr(not(test), allow(dead_code))]

use trackcraft_engine::Session;

/// What the user gives to open a server workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerWorkspace {
    /// The server's origin or prefix as the page sees it (`""` = the page's own origin and path).
    pub base: String,
    /// The workspace name on the server (`NAME` of `DIR/NAME.track`).
    pub workspace: String,
    /// An API token from `trackcraft-server token create` (sent as `Authorization: Bearer`).
    pub token: String,
}

/// Opens a server workspace for live editing.
///
/// TODO(M4.2): fetch the workspace snapshot with the token, build a `Session` over it and keep it
/// in sync (send each command, apply the server's operations). The browser shell will call this
/// from a "Open server workspace" dialog once it exists; nothing calls it yet.
pub fn open_server_workspace(target: &ServerWorkspace) -> Result<Session, String> {
    let _ = target;
    Err("opening server workspaces in the browser needs sync (M4.2), which hasn't landed yet".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_workspaces_are_not_available_yet() {
        let t = ServerWorkspace { base: String::new(), workspace: "lab".into(), token: "t".into() };
        assert!(open_server_workspace(&t).err().unwrap().contains("M4.2"));
    }
}
