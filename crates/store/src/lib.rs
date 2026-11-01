//! TrackCraft storage (L3): `.track` workspace files.
//!
//! An `.track` file is UTF-8 JSON: `{"format":"trackcraft","schema":N,"workspace":{…}}`. Saves
//! are atomic (write a sibling temp file, then rename) so a crash mid-save never truncates the
//! user's file. Loads are capped in size and check the schema version.
//!
//! The server will add a database-backed store behind the same load/save shape; this crate is
//! the local-first single-file format every build can read.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::Path;

use serde::{Deserialize, Serialize};
use trackcraft_model::{SCHEMA_VERSION, Workspace};

/// Largest file accepted (a workspace with tens of thousands of issues is a few hundred MB).
pub const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const FORMAT: &str = "trackcraft";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("{0} is larger than the 1 GiB limit")]
    TooLarge(String),
    #[error("not an TrackCraft workspace file: {0}")]
    Format(String),
    #[error("this file was saved by a newer TrackCraft (schema {found}; this build reads up to {SCHEMA_VERSION})")]
    Newer { found: u32 },
}

#[derive(Serialize, Deserialize)]
struct Envelope<W> {
    format: String,
    schema: u32,
    workspace: W,
}

/// Serializes a workspace to the `.track` JSON text.
pub fn to_string(ws: &Workspace) -> Result<String, StoreError> {
    serde_json::to_string_pretty(&Envelope { format: FORMAT.into(), schema: SCHEMA_VERSION, workspace: ws })
        .map_err(|e| StoreError::Format(e.to_string()))
}

/// Parses `.track` JSON text.
pub fn from_str(s: &str) -> Result<Workspace, StoreError> {
    let head: serde_json::Value = serde_json::from_str(s).map_err(|e| StoreError::Format(e.to_string()))?;
    if head.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
        return Err(StoreError::Format("missing \"format\": \"trackcraft\"".into()));
    }
    let schema = head.get("schema").and_then(serde_json::Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if schema > SCHEMA_VERSION {
        return Err(StoreError::Newer { found: schema });
    }
    if schema == 0 {
        return Err(StoreError::Format("missing schema version".into()));
    }
    // Migrations from older schemas go here, one step per version.
    let env: Envelope<Workspace> = serde_json::from_value(head).map_err(|e| StoreError::Format(e.to_string()))?;
    let mut ws = env.workspace;
    ws.schema_version = SCHEMA_VERSION;
    Ok(ws)
}

pub fn load(path: &Path) -> Result<Workspace, StoreError> {
    let p = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|source| StoreError::Io { path: p.clone(), source })?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge(p));
    }
    let s = std::fs::read_to_string(path).map_err(|source| StoreError::Io { path: p, source })?;
    from_str(&s)
}

pub fn save(ws: &Workspace, path: &Path) -> Result<(), StoreError> {
    let text = to_string(ws)?;
    let p = path.display().to_string();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "workspace.track".into());
    let tmp = path.with_file_name(format!(".{name}.saving"));
    std::fs::write(&tmp, text).map_err(|source| StoreError::Io { path: tmp.display().to_string(), source })?;
    std::fs::rename(&tmp, path).map_err(|source| {
        let _ = std::fs::remove_file(&tmp);
        StoreError::Io { path: p, source }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejects() {
        let mut ws = Workspace::new("T");
        ws.add_user("ann", "Ann").unwrap();
        let s = to_string(&ws).unwrap();
        assert_eq!(from_str(&s).unwrap(), ws);
        assert!(from_str("{}").is_err());
        assert!(from_str("not json").is_err());
        assert!(matches!(from_str(r#"{"format":"trackcraft","schema":999,"workspace":{}}"#), Err(StoreError::Newer { found: 999 })));
        assert!(from_str(r#"{"format":"trackcraft","schema":1,"workspace":{"name":5}}"#).is_err());
    }

    #[test]
    fn save_is_atomic_and_loads_back() {
        let dir = std::env::temp_dir().join(format!("trackcraft-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.track");
        let ws = Workspace::new("Saved");
        save(&ws, &path).unwrap();
        assert_eq!(load(&path).unwrap().name, "Saved");
        assert!(!dir.join(".w.track.saving").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
