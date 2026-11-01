//! JSON views of model objects returned by commands.
//!
//! Field names follow the public Jira/Confluence REST shapes where one exists (`key`, `summary`,
//! `status.name`, `statusCategory`, `version.number`) so agents that know those APIs read ours.

use serde_json::{Value, json};
use trackcraft_model::{Change, Issue, Page, Project, Space, Sprint, StatusCategory, Workspace};

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
        "history": history_groups(i).len(),
        "rank": i.rank,
        "sprint": i.sprint.and_then(|id| ws.sprints.get(&id)).map(|s| json!({"id": s.id.0, "name": s.name, "state": s.state.name()})),
        "closedSprints": i.past_sprints.iter().filter_map(|id| ws.sprints.get(id)).map(|s| json!({"id": s.id.0, "name": s.name})).collect::<Vec<_>>(),
    })
}

/// A sprint in the Jira Agile REST shape (`id`, `name`, `state`, `goal`, `startDate`,
/// `endDate`, `completeDate`), plus its project and issue count.
pub fn sprint(ws: &Workspace, s: &Sprint) -> Value {
    let issues = ws.issues.values().filter(|i| i.sprint == Some(s.id) || i.past_sprints.contains(&s.id)).count();
    json!({
        "id": s.id.0,
        "project": s.project.as_str(),
        "name": s.name,
        "state": s.state.name(),
        "goal": s.goal,
        "startDate": s.start.map(|t| t.to_rfc3339()),
        "endDate": s.end.map(|t| t.to_rfc3339()),
        "completeDate": s.completed.map(|t| t.to_rfc3339()),
        "issueCount": issues,
    })
}

/// A backlog / sprint row: the brief plus what planning needs (points, rank, parent).
pub fn planning_row(ws: &Workspace, i: &Issue) -> Value {
    let mut v = issue_brief(ws, i);
    if let Some(o) = v.as_object_mut() {
        o.insert("storyPoints".into(), json!(i.story_points));
        o.insert("rank".into(), json!(i.rank));
        o.insert("parent".into(), json!(i.parent.as_ref().map(ToString::to_string)));
    }
    v
}

/// Points and per-category counts of a set of issues (sprint headers).
pub fn tally<'a>(ws: &Workspace, issues: impl IntoIterator<Item = &'a Issue>) -> (f64, Value) {
    let (mut points, mut todo, mut doing, mut done) = (0.0, 0u64, 0u64, 0u64);
    for i in issues {
        points += i.story_points.unwrap_or(0.0);
        match ws.status_category(i) {
            StatusCategory::ToDo => todo += 1,
            StatusCategory::InProgress => doing += 1,
            StatusCategory::Done => done += 1,
        }
    }
    (points, json!({"todo": todo, "inProgress": doing, "done": done}))
}

/// History entries grouped by change id (one group per command), oldest first.
fn history_groups(i: &Issue) -> Vec<&[Change]> {
    i.history.chunk_by(|a, b| a.id == b.id).collect()
}

/// An issue's change history, newest first, in the Jira changelog shape:
/// `{"id", "author", "created", "items": [{"field", "fromString", "toString"}]}`.
pub fn history(i: &Issue) -> Vec<Value> {
    history_groups(i)
        .into_iter()
        .rev()
        .filter_map(|g| {
            let first = g.first()?;
            let items: Vec<Value> = g.iter().map(|c| json!({"field": c.field, "fromString": c.from, "toString": c.to})).collect();
            Some(json!({"id": first.id.0, "author": first.author, "created": first.at.to_rfc3339(), "items": items}))
        })
        .collect()
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
