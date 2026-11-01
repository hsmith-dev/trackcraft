//! TrackCraft model (L2): one workspace holding both the tracker (projects, issues, workflows —
//! Jira's domain) and the wiki (spaces and page trees — Confluence's domain), and the links
//! between them.
//!
//! Edit primitives live here and validate everything; the engine wraps them in commands, undo
//! and permissions. No method panics on bad input — each returns a [`ModelError`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod record;
mod tracker;
mod wiki;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, IdGen, IssueKey, ProjectKey};

pub use record::IssueRecord;
pub use trackcraft_core::Timestamp;
pub use trackcraft_markup::Doc;
pub use tracker::{
    Comment, Issue, IssueLink, IssueType, LinkKind, NewIssue, PRIORITIES, Project, ProjectKind, Status, StatusCategory, Transition, Workflow,
    priority_rank,
};
pub use wiki::{NewPage, Page, PageEdit, PageVersion, Space};

/// Bumped whenever the saved shape changes; `store` migrates older files.
pub const SCHEMA_VERSION: u32 = 1;
/// Longest summary/title accepted (Jira documents 255 for summaries).
pub const MAX_TITLE: usize = 255;
/// Most labels on one item.
pub const MAX_LABELS: usize = 100;
/// Deepest page tree accepted.
pub const MAX_PAGE_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    NoSuchProject(String),
    NoSuchIssue(String),
    NoSuchSpace(String),
    NoSuchPage(Id),
    NoSuchUser(String),
    NoSuchComment(Id),
    Duplicate(String),
    Invalid(String),
    NoTransition { from: String, to: String },
    StaleVersion { expected: u32, actual: u32 },
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::NoSuchProject(k) => write!(f, "no project with key {k}"),
            ModelError::NoSuchIssue(k) => write!(f, "no issue {k}"),
            ModelError::NoSuchSpace(k) => write!(f, "no space with key {k}"),
            ModelError::NoSuchPage(id) => write!(f, "no page with id {id}"),
            ModelError::NoSuchUser(u) => write!(f, "no user {u}"),
            ModelError::NoSuchComment(id) => write!(f, "no comment with id {id}"),
            ModelError::Duplicate(what) => write!(f, "{what} already exists"),
            ModelError::Invalid(why) => f.write_str(why),
            ModelError::NoTransition { from, to } => write!(f, "the workflow has no transition from {from} to {to}"),
            ModelError::StaleVersion { expected, actual } => {
                write!(f, "the page changed: you edited version {expected} but it is now at version {actual}")
            }
        }
    }
}
impl std::error::Error for ModelError {}

pub type Result<T> = std::result::Result<T, ModelError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Stable account name (`ann`), used in JQL and mentions.
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub schema_version: u32,
    pub name: String,
    pub users: BTreeMap<String, User>,
    pub projects: BTreeMap<ProjectKey, Project>,
    pub issues: BTreeMap<IssueKey, Issue>,
    pub spaces: BTreeMap<ProjectKey, Space>,
    pub pages: BTreeMap<Id, Page>,
    pub ids: IdGen,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace::new("Untitled workspace")
    }
}

impl Workspace {
    pub fn new(name: &str) -> Self {
        Workspace {
            schema_version: SCHEMA_VERSION,
            name: name.to_string(),
            users: BTreeMap::new(),
            projects: BTreeMap::new(),
            issues: BTreeMap::new(),
            spaces: BTreeMap::new(),
            pages: BTreeMap::new(),
            ids: IdGen::default(),
        }
    }

    pub fn add_user(&mut self, id: &str, display_name: &str) -> Result<()> {
        let id = id.trim();
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_whitespace) {
            return Err(ModelError::Invalid("a user id is 1–128 characters with no spaces".into()));
        }
        if self.users.contains_key(id) {
            return Err(ModelError::Duplicate(format!("user {id}")));
        }
        let display_name = if display_name.trim().is_empty() { id } else { display_name.trim() };
        self.users.insert(id.to_string(), User { id: id.to_string(), display_name: clip(display_name, MAX_TITLE), email: None });
        Ok(())
    }

    pub fn user(&self, id: &str) -> Result<&User> {
        self.users.get(id).ok_or_else(|| ModelError::NoSuchUser(id.to_string()))
    }
}

/// Validates a title/summary: trimmed, non-empty, at most [`MAX_TITLE`] characters.
pub fn title(s: &str, what: &str) -> Result<String> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ModelError::Invalid(format!("{what} can't be empty")));
    }
    if s.chars().count() > MAX_TITLE {
        return Err(ModelError::Invalid(format!("{what} is longer than {MAX_TITLE} characters")));
    }
    if s.chars().any(char::is_control) {
        return Err(ModelError::Invalid(format!("{what} can't contain line breaks or control characters")));
    }
    Ok(s.to_string())
}

/// Labels: no spaces (as in Jira), deduplicated, capped.
pub fn labels(input: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for l in input {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        if l.chars().any(char::is_whitespace) || l.chars().count() > MAX_TITLE {
            return Err(ModelError::Invalid(format!("label '{}' must be one word of at most {MAX_TITLE} characters", clip(l, 40))));
        }
        if !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
    }
    if out.len() > MAX_LABELS {
        return Err(ModelError::Invalid(format!("at most {MAX_LABELS} labels")));
    }
    Ok(out)
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests;
