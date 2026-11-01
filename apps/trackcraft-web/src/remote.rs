//! Server workspaces in the browser (M4.6): the pure parts of the connect flow — the remembered
//! token's storage keys, the title, error bodies, client ids. The sync state machine is
//! [`crate::sync`]; the browser transport is `sync_web.rs`; the dialog lives in `web.rs`.
// Natively only the tests use this module; the constants are for the browser shell.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::Value;

/// `localStorage` key of the token, set only when "Remember on this device" is ticked.
pub const TOKEN_KEY: &str = "trackcraft.server.token";
/// `localStorage` key of the last workspace name connected to (with a remembered token).
pub const WORKSPACE_KEY: &str = "trackcraft.server.workspace";

/// What the user typed in the connect dialog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServerWorkspace {
    /// The workspace name on the server (`NAME` of `DIR/NAME.track`); empty = its default.
    pub workspace: String,
    /// An API token from `trackcraft-server token create` (sent as `Authorization: Bearer`).
    pub token: String,
}

impl ServerWorkspace {
    /// How the workspace is named in the title and toasts.
    pub fn label(&self) -> String {
        let w = self.workspace.trim();
        if w.is_empty() { "the server's default workspace".into() } else { w.chars().take(80).collect() }
    }
}

/// The document title: the app's title, plus the server workspace and connection state.
pub fn title(app_title: &str, server: Option<(&str, &str)>) -> String {
    match server {
        Some((label, status)) => format!("{app_title} · {label} · {status}"),
        None => app_title.to_string(),
    }
}

/// A readable message from an error body: the API's error shapes (`{"message"}`, `{"error"}`,
/// Jira's `{"errorMessages": [..]}`), else the text itself, capped.
pub fn error_message(body: &str) -> String {
    let from_json = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v.get("message")
            .or_else(|| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| v.get("errorMessages").and_then(|m| m.get(0)).and_then(Value::as_str).map(str::to_string))
    });
    let text = from_json.unwrap_or_else(|| body.trim().to_string());
    let text: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
    if text.is_empty() { "no details".into() } else { text }
}

/// The ticket from a `POST api/sync/ticket` answer.
pub fn parse_ticket(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let t = v.get("ticket")?.as_str()?;
    (t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')).then(|| t.to_string())
}

/// A client id from random bytes: 16 hex characters.
pub fn client_id(bytes: [u8; 8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_titles() {
        assert_eq!(ServerWorkspace { workspace: " lab ".into(), token: String::new() }.label(), "lab");
        assert!(ServerWorkspace::default().label().contains("default"));
        assert_eq!(title("Lab — TrackCraft", Some(("lab", "Offline · 2 unsent"))), "Lab — TrackCraft · lab · Offline · 2 unsent");
        assert_eq!(title("Lab — TrackCraft", None), "Lab — TrackCraft");
    }

    #[test]
    fn error_bodies() {
        assert_eq!(error_message(r#"{"message":"no workspace named 'x'"}"#), "no workspace named 'x'");
        assert_eq!(error_message(r#"{"errorMessages":["Issue does not exist"]}"#), "Issue does not exist");
        assert_eq!(error_message("Bad Gateway\n"), "Bad Gateway");
        assert_eq!(error_message(""), "no details");
        assert!(error_message(&"x".repeat(5000)).len() <= 200);
    }

    #[test]
    fn tickets_and_ids() {
        assert_eq!(parse_ticket(r#"{"ticket":"tcs_00ff","expiresIn":60}"#).as_deref(), Some("tcs_00ff"));
        for junk in ["", "{}", r#"{"ticket":"a b"}"#, r#"{"ticket":5}"#, "<html>"] {
            assert_eq!(parse_ticket(junk), None, "{junk}");
        }
        assert_eq!(client_id([0, 1, 2, 3, 0xaa, 0xbb, 0xcc, 0xff]), "00010203aabbccff");
        assert!(trackcraft_sync_proto::Origin::valid_client(&client_id([9; 8])));
    }
}
