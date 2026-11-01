//! Where the central area can be: [`View`], and the `ui.navigate` request that names one.
//!
//! Parallel work items each add one `View` variant and one arm in [`parse`]; append, don't
//! reorder.

use serde_json::Value;
use trackcraft_core::{Id, IssueKey, ProjectKey};
use trackcraft_model::Workspace;

/// What the central area shows.
#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Home,
    Issues,
    Board(ProjectKey),
    Issue(IssueKey),
    Page(Id),
    PageHistory(Id),
}

/// The view a `ui.navigate` request names:
/// `{"view": "home"|"issues"|"board"|"issue"|"page"|"history", "project"|"key"|"id": …}`.
/// Pages must exist; issue and board keys must parse.
pub(crate) fn parse(ws: &Workspace, p: &Value) -> Result<View, String> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(match s("view").as_str() {
        "home" => View::Home,
        "issues" => View::Issues,
        "board" => View::Board(ProjectKey::parse(&s("project").to_ascii_uppercase()).map_err(|e| e.to_string())?),
        "issue" => View::Issue(IssueKey::parse(&s("key")).map_err(|e| e.to_string())?),
        "page" | "history" => {
            let id = Id(p.get("id").and_then(Value::as_u64).ok_or("missing \"id\"")?);
            ws.page(id).map_err(|e| e.to_string())?;
            if s("view") == "page" { View::Page(id) } else { View::PageHistory(id) }
        }
        other => return Err(format!("unknown view {other}")),
    })
}
