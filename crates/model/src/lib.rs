//! TrackCraft model (L2): one workspace holding both the tracker (projects, issues, workflows —
//! Jira's domain) and the wiki (spaces and page trees — Confluence's domain), and the links
//! between them.
//!
//! Edit primitives live here and validate everything; the engine wraps them in commands, undo
//! and permissions. No method panics on bad input — each returns a [`ModelError`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod access;
mod agile;
mod fields; // M2.4
mod import;
mod notify;
mod rank;
mod record;
mod tracker;
mod wiki;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, IdGen, IssueKey, ProjectKey};

pub use access::{Access, AccessLevel, Grants, MAX_RESTRICTION_USERS, ProjectRole, Restriction, SpaceRole, WorkspaceRole};
pub use agile::{Anchor, Completion, MAX_GOAL, MAX_MOVE, MAX_SPRINTS, MoveOpenTo, Sprint, SprintEdit, SprintState};
pub use fields::{
    Component, ComponentEdit, FIELD_KINDS, FieldDef, FieldEdit, FieldKind, FieldValue, HISTORY_AFFECTS_VERSION, HISTORY_COMPONENT,
    HISTORY_FIX_VERSION, IssueFields, MAX_COMPONENTS, MAX_DEF_DESCRIPTION, MAX_FIELDS, MAX_OPTIONS, MAX_PER_ISSUE, MAX_TEXT_VALUE, MAX_URL,
    MAX_VERSIONS, NewVersion, ProjectFields, RESERVED_FIELD_NAMES, Version, VersionEdit, VersionList, field_id_of, parse_day,
}; // M2.4
pub use import::MAX_IMPORTED_KINDS;
pub use notify::{Inbox, MAX_NOTIFICATIONS, MAX_WATCHES, NOTIFY_EXCERPT, Notification, NotifyKind, Target, Watches};
pub use rank::{MAX_RANK_LEN, is_valid_rank, rank_between, ranks_between, rebalanced};
pub use record::IssueRecord;
pub use trackcraft_core::Timestamp;
pub use trackcraft_markup::Doc;
pub use tracker::{
    Change, Comment, HISTORY_PREVIEW, Issue, IssueLink, IssueType, LinkKind, MAX_HISTORY, NewIssue, PRIORITIES, Project, ProjectKind,
    RESOLUTION_DONE, Status, StatusCategory, Transition, Workflow, priority_rank,
};
pub use wiki::{
    AnchorLocation, BUILTIN_TEMPLATES, BuiltinTemplate, CopyPage, InlineAnchor, MAX_COMMENT_CHARS, MAX_COPY, MAX_PAGE_COMMENTS, MAX_QUOTE_CHARS,
    MAX_TEMPLATE_BYTES, MAX_TEMPLATES, NewPage, NewPageComment, NewTemplate, Page, PageComment, PageEdit, PageVersion, Placement, ResolvedTemplate,
    Space, Template, builtin_template, expand_doc, expand_placeholders, locate_anchor,
};

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

/// M2.4: a custom field's id (`cf[ID]` in JQL, `customfield_ID` over REST).
pub type FieldId = Id;

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
    /// Sprints of every project (added in schema 1; older files load with none).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sprints: BTreeMap<Id, Sprint>,
    /// Saved page templates, workspace-wide or per space (older files load with none).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub templates: BTreeMap<Id, Template>,
    /// M4.5: notifications and watch lists, per user (older files load with none).
    #[serde(default, skip_serializing_if = "Inbox::is_empty")]
    pub inbox: Inbox,
    pub ids: IdGen,
    /// Roles, access levels and page restrictions (M4.3; older files load with none: everyone is an admin).
    #[serde(default, skip_serializing_if = "Access::is_empty")]
    pub access: Access,
    /// M2.4: custom fields, components and versions per project (older files load with none).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<ProjectKey, ProjectFields>,
    /// Transient (never saved): whether history entries are being grouped into one change.
    #[serde(skip)]
    batch: Batch,
}

/// History grouping state: outside a batch each primitive gets its own change id; inside one,
/// every entry shares the id allocated for the first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Batch {
    #[default]
    Off,
    Open(Option<Id>),
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
            sprints: BTreeMap::new(),
            templates: BTreeMap::new(),
            inbox: Inbox::default(),
            ids: IdGen::default(),
            access: Access::default(),
            fields: BTreeMap::new(),
            batch: Batch::Off,
        }
    }

    /// Starts grouping history: every change recorded until [`Workspace::end_batch`] shares one
    /// id, so one command (an edit of several fields) reads as one activity entry. The engine
    /// wraps each mutating command in a batch.
    pub fn begin_batch(&mut self) {
        self.batch = Batch::Open(None);
    }

    pub fn end_batch(&mut self) {
        self.batch = Batch::Off;
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

    /// Renames the workspace (a title: 1–255 characters, one line).
    pub fn rename(&mut self, name: &str) -> Result<()> {
        self.name = title(name, "the workspace name")?;
        Ok(())
    }

    /// Changes a user's display name and/or email. `email: Some(None)` (or an empty string)
    /// clears it. The id never changes: history and JQL refer to it.
    pub fn edit_user(&mut self, id: &str, display_name: Option<&str>, email: Option<Option<&str>>) -> Result<()> {
        let name = display_name.map(|n| title(n, "the user's name")).transpose()?;
        let email = match email {
            None => None,
            Some(e) => Some(match e.map(str::trim).filter(|e| !e.is_empty()) {
                None => None,
                Some(e) => Some(check_email(e)?),
            }),
        };
        let u = self.users.get_mut(id).ok_or_else(|| ModelError::NoSuchUser(clip(id, 128)))?;
        if let Some(n) = name {
            u.display_name = n;
        }
        if let Some(e) = email {
            u.email = e;
        }
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

/// Longest email address accepted (RFC 5321's path limit).
pub const MAX_EMAIL: usize = 254;

/// A plausible email address: one `@` with text on both sides, a dot inside the domain, no
/// spaces or control characters, at most [`MAX_EMAIL`] characters. Deliverability isn't checked.
fn check_email(e: &str) -> Result<String> {
    let bad = || ModelError::Invalid(format!("'{}' is not an email address", clip(e, 40)));
    if e.chars().count() > MAX_EMAIL || e.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    match e.split_once('@') {
        Some((local, domain))
            if !local.is_empty() && !domain.contains('@') && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') =>
        {
            Ok(e.to_string())
        }
        _ => Err(bad()),
    }
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests;
