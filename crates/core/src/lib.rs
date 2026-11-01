//! TrackCraft core (L0): identifiers, keys and time shared by every crate.
//!
//! Everything here treats its input as hostile: keys and ids come from files, JQL, the CLI, the
//! control channel and MCP, so parsing returns `Result` and never panics.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Timestamps are UTC, through chrono (no hand-rolled calendar code).
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// The longest project key accepted (Jira's documented default limit is 10; we allow a bit more).
pub const PROJECT_KEY_MAX: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("project key is empty")]
    Empty,
    #[error("project key is longer than {PROJECT_KEY_MAX} characters")]
    TooLong,
    #[error("project key must start with an uppercase letter A–Z")]
    BadStart,
    #[error("project key may only contain A–Z, 0–9 and _")]
    BadChar,
    #[error("issue key must look like ABC-123")]
    BadIssueKey,
}

/// A project key such as `ATLAS`: an uppercase ASCII letter, then uppercase letters, digits or `_`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectKey(String);

impl ProjectKey {
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let s = s.trim();
        let mut chars = s.chars();
        let first = chars.next().ok_or(KeyError::Empty)?;
        if s.chars().count() > PROJECT_KEY_MAX {
            return Err(KeyError::TooLong);
        }
        if !first.is_ascii_uppercase() {
            return Err(KeyError::BadStart);
        }
        if !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            return Err(KeyError::BadChar);
        }
        Ok(ProjectKey(s.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProjectKey {
    type Error = KeyError;
    fn try_from(s: String) -> Result<Self, KeyError> {
        ProjectKey::parse(&s)
    }
}
impl From<ProjectKey> for String {
    fn from(k: ProjectKey) -> String {
        k.0
    }
}
impl fmt::Display for ProjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An issue key such as `ATLAS-42`: project key, a hyphen, a positive number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IssueKey {
    pub project: ProjectKey,
    pub number: u64,
}

impl IssueKey {
    pub fn new(project: ProjectKey, number: u64) -> Self {
        IssueKey { project, number }
    }
    /// Parses `abc-12` as well as `ABC-12` (Jira accepts lowercase keys in search and URLs).
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let s = s.trim();
        let (p, n) = s.rsplit_once('-').ok_or(KeyError::BadIssueKey)?;
        let number: u64 = n.parse().map_err(|_| KeyError::BadIssueKey)?;
        if number == 0 {
            return Err(KeyError::BadIssueKey);
        }
        Ok(IssueKey { project: ProjectKey::parse(&p.to_ascii_uppercase())?, number })
    }
}

impl FromStr for IssueKey {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Self, KeyError> {
        IssueKey::parse(s)
    }
}
impl TryFrom<String> for IssueKey {
    type Error = KeyError;
    fn try_from(s: String) -> Result<Self, KeyError> {
        IssueKey::parse(&s)
    }
}
impl From<IssueKey> for String {
    fn from(k: IssueKey) -> String {
        k.to_string()
    }
}
impl fmt::Display for IssueKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.project, self.number)
    }
}

/// An opaque numeric id (pages, spaces, comments, users…). Ids are never reused in a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Id(pub u64);

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Hands out fresh ids; saturates instead of wrapping (a workspace never holds 2^64 objects).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdGen {
    next: u64,
}

impl IdGen {
    pub fn next_id(&mut self) -> Id {
        self.next = self.next.saturating_add(1);
        Id(self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_keys() {
        assert!(ProjectKey::parse("ATLAS").is_ok());
        assert!(ProjectKey::parse("A1_B").is_ok());
        assert_eq!(ProjectKey::parse(""), Err(KeyError::Empty));
        assert_eq!(ProjectKey::parse("1AB"), Err(KeyError::BadStart));
        assert_eq!(ProjectKey::parse("AB-C"), Err(KeyError::BadChar));
        assert_eq!(ProjectKey::parse(&"A".repeat(33)), Err(KeyError::TooLong));
        assert_eq!(ProjectKey::parse("É"), Err(KeyError::BadStart));
    }

    #[test]
    fn issue_keys() {
        let k = IssueKey::parse("atlas-42").unwrap();
        assert_eq!(k.to_string(), "ATLAS-42");
        for bad in ["", "-", "ATLAS-", "ATLAS-0", "ATLAS-x", "-5", "ATLAS-99999999999999999999999", "Ä-1"] {
            assert!(IssueKey::parse(bad).is_err(), "{bad}");
        }
        let json = serde_json::to_string(&k).unwrap_or_default();
        assert_eq!(json, "\"ATLAS-42\"");
    }
}
