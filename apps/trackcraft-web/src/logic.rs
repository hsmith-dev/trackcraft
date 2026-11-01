//! The browser shell's pure parts (no web APIs), shared with native unit tests.
// Natively only the tests use this module; the constants are for the browser shell.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::Value;
use trackcraft_engine::Session;

/// Largest `.track` file File › Open reads (checked before the bytes are read).
pub const MAX_OPEN_BYTES: f64 = (64u32 << 20) as f64;
/// Largest workspace kept in `localStorage` (browsers allow about 5 MB per origin). A larger one
/// stays in memory only, and leaving the page asks first.
pub const MAX_STORED_BYTES: usize = 4_000_000;
/// The `localStorage` key holding the last open workspace (`.track` JSON).
pub const STORAGE_KEY: &str = "trackcraft.workspace";
/// Longest workspace name used in a download's file name.
const MAX_NAME_CHARS: usize = 80;

/// A session over `.track` text (an opened file or the stored copy), as the first user.
pub fn session_from_text(text: &str) -> Result<Session, String> {
    let ws = trackcraft_store::from_str(text).map_err(|e| e.to_string())?;
    let user = ws.users.keys().next().cloned().unwrap_or_default();
    Ok(Session::new(ws, &user))
}

/// The `.track` text of the session's workspace.
pub fn to_text(session: &Session) -> Result<String, String> {
    trackcraft_store::to_string(&session.ws).map_err(|e| e.to_string())
}

/// The download name for a workspace: its name without characters file systems reject, capped,
/// ending in `.track`.
pub fn file_name(ws_name: &str) -> String {
    let clean: String = ws_name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .take(MAX_NAME_CHARS)
        .collect();
    let clean = clean.trim().trim_matches('.').trim();
    if clean.is_empty() { "workspace.track".into() } else { format!("{clean}.track") }
}

/// The server version from a `GET api/health` body (`{"status":"ok","version":"0.1.0"}`), or
/// `None` when the body isn't a TrackCraft server's health answer.
pub fn parse_health(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    if v.get("status").and_then(Value::as_str) != Some("ok") {
        return None;
    }
    let version: String = v.get("version").and_then(Value::as_str).unwrap_or_default().chars().take(40).collect();
    Some(version)
}

/// The note shown when the page is served by a TrackCraft server.
pub fn server_note(version: &str) -> String {
    if version.is_empty() {
        "Served by a TrackCraft server. Server workspaces: open with a token (coming with sync); this tab edits a local copy.".into()
    } else {
        format!("Served by TrackCraft server {version}. Server workspaces: open with a token (coming with sync); this tab edits a local copy.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_sample() {
        let s = Session::sample();
        let text = to_text(&s).unwrap();
        let back = session_from_text(&text).unwrap();
        assert_eq!(back.ws.name, s.ws.name);
        assert!(!back.dirty);
    }

    #[test]
    fn hostile_text_is_an_error() {
        for junk in ["", "{", "[]", "{\"format\":\"other\"}", "{\"format\":\"trackcraft\",\"schema\":99999}", "\u{0}"] {
            assert!(session_from_text(junk).is_err(), "{junk:?}");
        }
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Lumen Labs"), "Lumen Labs.track");
        assert_eq!(file_name("a/b\\c:d"), "abcd.track");
        assert_eq!(file_name(""), "workspace.track");
        assert_eq!(file_name(".."), "workspace.track");
        assert_eq!(file_name("\u{7}\n"), "workspace.track");
        assert!(file_name(&"é".repeat(500)).chars().count() <= MAX_NAME_CHARS + 6);
    }

    #[test]
    fn health_bodies() {
        assert_eq!(parse_health(r#"{"status":"ok","version":"0.1.0"}"#).as_deref(), Some("0.1.0"));
        assert_eq!(parse_health(r#"{"status":"ok"}"#).as_deref(), Some(""));
        assert_eq!(parse_health(r#"{"status":"down"}"#), None);
        assert_eq!(parse_health("<!DOCTYPE html>"), None);
        assert!(server_note("0.1.0").contains("Server workspaces: open with a token"));
        assert!(server_note("").contains("Server workspaces: open with a token"));
    }
}
