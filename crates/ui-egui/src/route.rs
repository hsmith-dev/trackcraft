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
    /// A project's backlog: open sprints, then the Backlog section (UI-11).
    Backlog(ProjectKey),
    /// A project's board showing only one sprint's issues (UI-12).
    SprintBoard(ProjectKey, Id),
    /// The searchable table of every project (UI-8).
    Projects,
    /// The searchable table of every space (UI-8).
    Spaces,
    /// Settings, on one of its tabs (UI-7).
    Settings(crate::settings::Tab),
    /// Two versions of a page compared: (page, from, to) (UI-28).
    PageDiff(Id, u32, u32),
    /// The acting user's notifications (M4.5).
    Inbox,
    /// A project's settings, on one of its tabs (UI-37).
    ProjectSettings(ProjectKey, crate::project_settings::ProjectTab),
    /// A space's settings (UI-37).
    SpaceSettings(ProjectKey),
    /// A project's reports: burndown, velocity, cumulative flow, created vs resolved (UI-24).
    Reports(ProjectKey),
    /// A project's custom fields, components and versions (M2.4; becomes a project settings tab).
    ProjectFields(ProjectKey),
    /// A project's versions with progress and Release (M2.4).
    Releases(ProjectKey),
}

/// The view a `ui.navigate` request names:
/// `{"view": "home"|"issues"|"board"|"issue"|"page"|"history"|"backlog", "project"|"key"|"id": …}`.
/// Pages must exist; issue and board keys must parse. A board takes `"sprint": "active" | 7`
/// (the active sprint falls back to the whole board when none is running).
pub(crate) fn parse(ws: &Workspace, p: &Value) -> Result<View, String> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let v = match s("view").as_str() {
        "home" => View::Home,
        "issues" => View::Issues,
        "board" => View::Board(ProjectKey::parse(&s("project").to_ascii_uppercase()).map_err(|e| e.to_string())?),
        "issue" => View::Issue(IssueKey::parse(&s("key")).map_err(|e| e.to_string())?),
        "page" | "history" => {
            let id = Id(p.get("id").and_then(Value::as_u64).ok_or("missing \"id\"")?);
            ws.page(id).map_err(|e| e.to_string())?;
            if s("view") == "page" { View::Page(id) } else { View::PageHistory(id) }
        }
        "backlog" => View::Backlog(ProjectKey::parse(&s("project").to_ascii_uppercase()).map_err(|e| e.to_string())?), // UI-11
        "projects" => View::Projects,                                                                                  // UI-8
        "spaces" => View::Spaces,                                                                                      // UI-8
        "settings" => View::Settings(crate::settings::Tab::parse(&s("tab")).ok_or("\"tab\" is appearance, workspace, people or shortcuts")?), // UI-7
        // UI-28: `{"view": "diff", "id": 14, "from": 1, "to": 2}`; `to` defaults to the current
        // version, `from` to the one before `to`.
        "diff" => {
            let id = Id(p.get("id").and_then(Value::as_u64).ok_or("missing \"id\"")?);
            let page = ws.page(id).map_err(|e| e.to_string())?;
            let num = |k: &str| p.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
            let to = num("to").unwrap_or(page.version);
            let from = num("from").unwrap_or_else(|| to.saturating_sub(1).max(1));
            for v in [from, to] {
                if v == 0 || v > page.version {
                    return Err(format!("page {id} has no version {v}"));
                }
            }
            View::PageDiff(id, from, to)
        }
        "inbox" => View::Inbox, // M4.5
        // UI-37: `{"view": "projectSettings", "project": "LUME", "tab": "access"}`,
        // `{"view": "spaceSettings", "space": "ENG"}`.
        "projectSettings" => {
            let k = ws.project(&s("project")).map_err(|e| e.to_string())?.key.clone();
            View::ProjectSettings(k, crate::project_settings::ProjectTab::parse(&s("tab")).ok_or("unknown project settings \"tab\"")?)
        }
        "spaceSettings" => View::SpaceSettings(ws.space(&s("space")).map_err(|e| e.to_string())?.key.clone()),
        // UI-24
        "reports" => View::Reports(ProjectKey::parse(&s("project").to_ascii_uppercase()).map_err(|e| e.to_string())?),
        // M2.4: `{"view": "fields"|"releases", "project": "LUME"}`.
        "fields" | "releases" => {
            let k = ProjectKey::parse(&s("project").to_ascii_uppercase()).map_err(|e| e.to_string())?;
            ws.project(k.as_str()).map_err(|e| e.to_string())?;
            // Fields live in project settings (the standalone view stays for old scripts).
            if s("view") == "fields" { View::ProjectSettings(k, crate::project_settings::ProjectTab::Fields) } else { View::Releases(k) }
        }
        other => return Err(format!("unknown view {other}")),
    };
    // UI-12: a board with `"sprint"` shows only that sprint.
    Ok(match v {
        View::Board(k) => match p.get("sprint") {
            None | Some(Value::Null) => View::Board(k),
            Some(Value::String(a)) if a.eq_ignore_ascii_case("active") => match ws.active_sprint(&k) {
                Some(sp) => View::SprintBoard(k, sp.id),
                None => View::Board(k),
            },
            Some(sp) => {
                let id = Id(sp.as_u64().ok_or("\"sprint\" is \"active\" or a sprint id")?);
                let sprint = ws.sprint(id).map_err(|e| e.to_string())?;
                if sprint.project != k {
                    return Err(format!("sprint {id} is not in project {k}"));
                }
                View::SprintBoard(k, id)
            }
        },
        v => v,
    })
}
