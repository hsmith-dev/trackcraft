//! JSON views of model objects returned by commands.
//!
//! Field names follow the public Jira/Confluence REST shapes where one exists (`key`, `summary`,
//! `status.name`, `statusCategory`, `version.number`) so agents that know those APIs read ours.

use serde_json::{Value, json};
use trackcraft_model::{Issue, Page, Project, Space, Workspace};

pub fn issue(ws: &Workspace, i: &Issue) -> Value {
    let cat = ws.status_category(i);
    json!({
        "key": i.key.to_string(),
        "project": i.key.project.as_str(),
        "issuetype": i.issue_type,
        "summary": i.summary,
        "description": i.description.to_markdown(),
        "status": {"name": i.status, "statusCategory": cat.name()},
        "priority": i.priority,
        "assignee": i.assignee,
        "reporter": i.reporter,
        "labels": i.labels,
        "parent": i.parent.as_ref().map(ToString::to_string),
        "storyPoints": i.story_points,
        "created": i.created.to_rfc3339(),
        "updated": i.updated.to_rfc3339(),
        "resolved": i.resolved.map(|t| t.to_rfc3339()),
        "comments": i.comments.iter().map(|c| json!({"id": c.id.0, "author": c.author, "body": c.body.to_markdown(), "created": c.created.to_rfc3339()})).collect::<Vec<_>>(),
        "links": i.links.iter().map(|l| json!({"type": l.kind.outward(), "key": l.target.to_string()})).collect::<Vec<_>>(),
    })
}

/// One line per issue, for lists and search results.
pub fn issue_brief(ws: &Workspace, i: &Issue) -> Value {
    json!({
        "key": i.key.to_string(),
        "issuetype": i.issue_type,
        "summary": i.summary,
        "status": i.status,
        "statusCategory": ws.status_category(i).name(),
        "priority": i.priority,
        "assignee": i.assignee,
    })
}

pub fn project(p: &Project, issue_count: usize) -> Value {
    json!({
        "key": p.key.as_str(),
        "name": p.name,
        "kind": format!("{:?}", p.kind).to_lowercase(),
        "lead": p.lead,
        "issueTypes": p.issue_types.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
        "statuses": p.workflow.statuses.iter().map(|s| json!({"name": s.name, "statusCategory": s.category.name()})).collect::<Vec<_>>(),
        "issueCount": issue_count,
    })
}

pub fn space(s: &Space) -> Value {
    json!({"key": s.key.as_str(), "name": s.name, "homepageId": s.homepage.0})
}

pub fn page(p: &Page) -> Value {
    json!({
        "id": p.id.0,
        "space": p.space.as_str(),
        "parentId": p.parent.map(|i| i.0),
        "title": p.title,
        "body": p.body.to_markdown(),
        "labels": p.labels,
        "version": {"number": p.version, "by": p.author, "when": p.updated.to_rfc3339(), "message": p.message},
        "created": p.created.to_rfc3339(),
    })
}

/// A page tree as nested `{id, title, children}`.
pub fn tree(ws: &Workspace, root: trackcraft_core::Id, depth: usize) -> Value {
    let title = ws.pages.get(&root).map(|p| p.title.clone()).unwrap_or_default();
    let children: Vec<Value> = if depth >= trackcraft_model::MAX_PAGE_DEPTH {
        vec![]
    } else {
        ws.children(root).iter().map(|c| tree(ws, c.id, depth.saturating_add(1))).collect()
    };
    json!({"id": root.0, "title": title, "children": children})
}
