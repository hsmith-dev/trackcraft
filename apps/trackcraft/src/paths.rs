//! Where TrackCraft keeps its per-user files (control keys, recent files).
//!
//! Shared with `trackcraft-cli` (included there with `#[path]`), so both agree on the key file.
//! `TRACKCRAFT_CONFIG_DIR` overrides the whole folder (isolated runs, tests).

use std::path::PathBuf;

fn var(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(target_os = "windows")]
fn platform_dir() -> Option<PathBuf> {
    var("APPDATA").map(|d| d.join("trackcraft"))
}

#[cfg(target_os = "macos")]
fn platform_dir() -> Option<PathBuf> {
    var("HOME").map(|h| h.join("Library").join("Application Support").join("trackcraft"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_dir() -> Option<PathBuf> {
    // The XDG spec ignores a relative XDG_CONFIG_HOME.
    var("XDG_CONFIG_HOME").filter(|p| p.is_absolute()).or_else(|| var("HOME").map(|h| h.join(".config"))).map(|d| d.join("trackcraft"))
}

/// `<config dir>/trackcraft`, or `None` when the environment names no home folder.
pub fn config_dir() -> Option<PathBuf> {
    var("TRACKCRAFT_CONFIG_DIR").or_else(platform_dir)
}

/// The control key file for a port: `<config dir>/trackcraft/control-key.<port>`.
pub fn key_path(port: u16) -> Option<PathBuf> {
    config_dir().map(|d| d.join(format!("control-key.{port}")))
}

/// The recent-files list: `<config dir>/trackcraft/recent.json`.
pub fn recent_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("recent.json"))
}
