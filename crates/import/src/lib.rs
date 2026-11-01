//! TrackCraft importers (L3): Jira issue exports (CSV, REST v3 JSON) and Confluence space
//! exports (HTML, single-page storage format) into a [`Workspace`].
//!
//! Formats come from Atlassian's **public** documentation only (the documented CSV export
//! columns, the REST API v3 `search/jql` response, the Confluence HTML export layout and
//! storage format). Test fixtures are written in code; no exported file is ever committed.
//!
//! Every input is hostile: sizes, rows, columns, nesting and archive entries are capped; a
//! malformed row or page becomes a warning in the [`ImportReport`], never a panic. An import
//! that fails outright (not CSV, too big, a zip with unsafe names) leaves the workspace
//! untouched: the work happens on a copy that replaces the workspace only on success.
//!
//! What maps where, and the guessing rules, are documented in `docs/import.md`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod confluence;
mod csv;
mod html;
mod jira;
mod jira_csv;
mod jira_json;
mod people;
pub mod wiki;

use serde_json::{Value, json};
use trackcraft_model::{Timestamp, Workspace};

pub use confluence::{ConfluenceSource, confluence_html, confluence_path};
pub use jira_csv::jira_csv;
pub use jira_json::jira_json;

/// Converts an HTML fragment (or Confluence storage-format XHTML) into rich text.
pub fn html_to_doc(src: &str) -> trackcraft_markup::Doc {
    html::to_doc(clip_bytes(src, MAX_FILE_BYTES)).0
}

/// Largest input file read (CSV, JSON, one HTML page, or a whole .zip).
pub const MAX_FILE_BYTES: usize = 64 << 20;
/// Most issues one import brings in.
pub const MAX_ISSUES: usize = 20_000;
/// Most CSV columns accepted in the header.
pub const MAX_COLUMNS: usize = 2_000;
/// Longest single CSV field / description / comment body, in bytes (longer ones are clipped).
pub const MAX_FIELD_BYTES: usize = 1 << 20;
/// Most comments / links / sprints kept per issue.
pub const MAX_PER_ISSUE: usize = 1_000;
/// Most pages one Confluence import brings in.
pub const MAX_PAGES: usize = 10_000;
/// Most entries read from a .zip or a directory.
pub const MAX_ENTRIES: usize = 50_000;
/// Most bytes decompressed from a .zip in total.
pub const MAX_UNZIPPED_BYTES: u64 = 256 << 20;
/// Deepest HTML element nesting kept (deeper tags are flattened into their ancestor).
pub const MAX_HTML_DEPTH: usize = 128;
/// Most warnings listed; the rest are counted.
pub const MAX_WARNINGS: usize = 500;

/// Why an import failed as a whole (nothing was changed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    TooLarge(String),
    Malformed(String),
    Unsafe(String),
    Io(String),
    Model(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::TooLarge(w) => write!(f, "too large: {w}"),
            ImportError::Malformed(w) => write!(f, "not a valid export: {w}"),
            ImportError::Unsafe(w) => write!(f, "refused: {w}"),
            ImportError::Io(w) => write!(f, "can't read the export: {w}"),
            ImportError::Model(w) => f.write_str(w),
        }
    }
}
impl std::error::Error for ImportError {}

pub type Result<T> = std::result::Result<T, ImportError>;

/// What an import created, and everything it skipped or guessed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub projects: Vec<String>,
    pub issues: usize,
    pub users: Vec<String>,
    pub sprints: usize,
    pub comments: usize,
    pub links: usize,
    pub spaces: Vec<String>,
    pub pages: usize,
    pub warnings: Vec<String>,
    /// Warnings beyond [`MAX_WARNINGS`], counted but not listed.
    pub more_warnings: usize,
}

impl ImportReport {
    pub fn warn(&mut self, w: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(clip(&w.into(), 400));
        } else {
            self.more_warnings = self.more_warnings.saturating_add(1);
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "projects": self.projects,
            "issues": self.issues,
            "users": self.users,
            "sprints": self.sprints,
            "comments": self.comments,
            "links": self.links,
            "spaces": self.spaces,
            "pages": self.pages,
            "warnings": self.warnings,
            "moreWarnings": self.more_warnings,
        })
    }
}

/// Who performs the import and when (history entries and fallbacks use these).
#[derive(Debug, Clone, Copy)]
pub struct Actor<'a> {
    pub user: &'a str,
    pub now: Timestamp,
}

/// Runs `f` on a copy of the workspace and keeps the copy only when it succeeds. Makes sure the
/// acting user exists first.
pub(crate) fn transact(
    ws: &mut Workspace,
    actor: Actor<'_>,
    f: impl FnOnce(&mut Workspace, &mut ImportReport) -> Result<()>,
) -> Result<ImportReport> {
    let mut w = ws.clone();
    let mut report = ImportReport::default();
    if w.user(actor.user).is_err() {
        w.add_user(actor.user, actor.user).map_err(|e| ImportError::Model(format!("the importing user: {e}")))?;
        report.users.push(actor.user.trim().to_string());
    }
    f(&mut w, &mut report)?;
    *ws = w;
    Ok(report)
}

/// Clips `s` to at most `max` characters.
pub(crate) fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Clips `s` to at most `max` bytes, at a char boundary.
pub(crate) fn clip_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default()
}

/// Checks the size of an input before parsing it.
pub(crate) fn check_size(len: usize, what: &str) -> Result<()> {
    if len > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge(format!("{what} is {len} bytes; the limit is {MAX_FILE_BYTES}")));
    }
    Ok(())
}

/// Reads a file, refusing anything over [`MAX_FILE_BYTES`].
pub fn read_file(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| ImportError::Io(format!("{}: {e}", path.display())))?;
    let mut buf = Vec::new();
    f.take(u64::try_from(MAX_FILE_BYTES).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| ImportError::Io(format!("{}: {e}", path.display())))?;
    check_size(buf.len(), &path.display().to_string())?;
    Ok(buf)
}

/// Reads a text file (UTF-8, a leading byte-order mark dropped; invalid bytes replaced).
pub fn read_text(path: &std::path::Path) -> Result<String> {
    let bytes = read_file(path)?;
    Ok(text_of(&bytes))
}

pub(crate) fn text_of(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Which importer `import_path` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    JiraCsv,
    JiraJson,
    Confluence,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s.trim().to_ascii_lowercase().replace(['-', '_'], "").as_str() {
            "jiracsv" | "csv" => Kind::JiraCsv,
            "jirajson" | "json" => Kind::JiraJson,
            "confluence" | "confluencehtml" | "html" => Kind::Confluence,
            _ => return None,
        })
    }
}

/// Imports a file (or, for Confluence, a directory or .zip) of the given kind.
pub fn import_path(ws: &mut Workspace, kind: Kind, path: &std::path::Path, space: Option<&str>, actor: Actor<'_>) -> Result<ImportReport> {
    match kind {
        Kind::JiraCsv => jira_csv(ws, &read_text(path)?, actor),
        Kind::JiraJson => jira_json(ws, &read_text(path)?, actor),
        Kind::Confluence => confluence_path(ws, path, space, actor),
    }
}

#[cfg(test)]
mod tests;
