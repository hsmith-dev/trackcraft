//! Jira Software Agile REST 1.0 subset (`/rest/agile/1.0/…`): one board per project (scrum for
//! software projects, kanban for business ones), its sprints and backlog, and sprint issues.

use std::collections::HashSet;

use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use trackcraft_engine::model::{Issue, IssueRecord, Project, ProjectKind, Sprint, Timestamp, Workspace};
use trackcraft_jql::{Context, matches, parse};

use crate::ApiState;
use crate::ctx::{Ctx, P, Q};
use crate::error::{ApiError, Flavor};
use crate::ids;
use crate::jira::{Fields, issue_json};

/// Most values one page returns (the public API's default and cap is 50).
pub const MAX_VALUES: usize = 50;
const A: Flavor = Flavor::Jira;

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/rest/agile/1.0/board", get(boards))
        .route("/rest/agile/1.0/board/{id}", get(board))
        .route("/rest/agile/1.0/board/{id}/sprint", get(board_sprints))
        .route("/rest/agile/1.0/board/{id}/backlog", get(board_backlog))
        .route("/rest/agile/1.0/board/{id}/issue", get(board_issues))
        .route("/rest/agile/1.0/sprint/{id}", get(sprint))
        .route("/rest/agile/1.0/sprint/{id}/issue", get(sprint_issues))
}

/// Agile dates: `2026-10-31T09:30:00.000Z`.
fn adate(t: &Timestamp) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn sprint_json(base: &str, s: &Sprint) -> Value {
    let mut v = json!({
        "id": s.id.0,
        "self": format!("{base}/rest/agile/1.0/sprint/{}", s.id.0),
        "state": s.state.name(),
        "name": s.name,
        "originBoardId": ids::key_id(s.project.as_str()),
        "goal": s.goal,
    });
    if let Some(o) = v.as_object_mut() {
        for (k, t) in [("startDate", s.start), ("endDate", s.end), ("completeDate", s.completed)] {
            if let Some(t) = t {
                o.insert(k.into(), json!(adate(&t)));
            }
        }
    }
    v
}

fn board_json(base: &str, p: &Project) -> Value {
    let id = ids::key_id(p.key.as_str());
    json!({
        "id": id,
        "self": format!("{base}/rest/agile/1.0/board/{id}"),
        "name": format!("{} board", p.key.as_str()),
        "type": match p.kind { ProjectKind::Software => "scrum", ProjectKind::Business => "kanban" },
        "location": {
            "projectId": id,
            "projectKey": p.key.as_str(),
            "projectName": p.name,
            "displayName": format!("{} ({})", p.name, p.key.as_str()),
            "projectTypeKey": match p.kind { ProjectKind::Software => "software", ProjectKind::Business => "business" },
            "name": p.name,
        },
    })
}

fn ok(v: Value) -> Response {
    Json(v).into_response()
}

fn values(start: usize, max: usize, total: usize, values: Vec<Value>) -> Value {
    let is_last = start.saturating_add(values.len()) >= total;
    json!({"maxResults": max, "startAt": start, "total": total, "isLast": is_last, "values": values})
}

fn board_project(ws: &Workspace, id: &str) -> Result<String, ApiError> {
    ids::project_key(ws, id).ok_or_else(|| {
        ApiError::not_found(A, format!("Board {} does not exist or you do not have permission to see it.", id.chars().take(40).collect::<String>()))
    })
}

/// Keys matching `jql` (the caller's extra filter), or `None` when there is none.
fn jql_filter(ws: &Workspace, user: &str, now: Timestamp, jql: Option<&str>) -> Result<Option<HashSet<String>>, ApiError> {
    let Some(q) = jql.filter(|q| !q.trim().is_empty()) else { return Ok(None) };
    let q = parse(q).map_err(|e| ApiError::bad(A, format!("JQL: {e}")))?;
    let cx = Context { current_user: Some(user.to_string()), now };
    let mut keys = HashSet::new();
    for issue in ws.issues.values() {
        if matches(&q, &IssueRecord { ws, issue }, &cx).map_err(|e| ApiError::bad(A, format!("JQL: {e}")))? {
            keys.insert(issue.key.to_string());
        }
    }
    Ok(Some(keys))
}

/// The public "search results" page shape used by board and sprint issue lists.
fn issue_page(ws: &Workspace, base: &str, q: &Q, issues: &[&Issue]) -> Result<Value, ApiError> {
    let start = q.num("startAt", 0, usize::MAX, A)?;
    let max = q.num("maxResults", MAX_VALUES, MAX_VALUES, A)?;
    let f = Fields::parse(&q.list("fields"), true);
    let page: Vec<Value> = issues.iter().skip(start).take(max).map(|i| issue_json(ws, base, i, &f)).collect();
    Ok(json!({"expand": "schema,names", "startAt": start, "maxResults": max, "total": issues.len(), "issues": page}))
}

async fn boards(cx: Ctx, q: Q) -> Result<Response, ApiError> {
    let base = cx.base.clone();
    cx.run(move |tx| {
        let ws = tx.ws();
        let project = q.get("projectKeyOrId").map(|p| ids::project_key(ws, p));
        let (ty, name) = (q.get("type").map(str::to_ascii_lowercase), q.get("name").map(str::to_ascii_lowercase));
        let all: Vec<Value> = ws
            .projects
            .values()
            .filter(|p| project.as_ref().is_none_or(|k| k.as_deref() == Some(p.key.as_str())))
            .map(|p| board_json(&base, p))
            .filter(|b| ty.as_deref().is_none_or(|t| b["type"].as_str() == Some(t)))
            .filter(|b| name.as_deref().is_none_or(|n| b["name"].as_str().is_some_and(|bn| bn.to_ascii_lowercase().contains(n))))
            .collect();
        let start = q.num("startAt", 0, usize::MAX, A)?;
        let max = q.num("maxResults", MAX_VALUES, MAX_VALUES, A)?;
        let total = all.len();
        Ok(ok(values(start, max, total, all.into_iter().skip(start).take(max).collect())))
    })
    .await
}

async fn board(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), path.get("id"));
    cx.run(move |tx| {
        let ws = tx.ws();
        let k = board_project(ws, &id)?;
        let p = ws.project(&k).map_err(|e| ApiError::not_found(A, e.to_string()))?;
        Ok(ok(board_json(&base, p)))
    })
    .await
}

async fn board_sprints(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), path.get("id"));
    cx.run(move |tx| {
        let k = board_project(tx.ws(), &id)?;
        if tx.ws().project(&k).is_ok_and(|p| p.kind == ProjectKind::Business) {
            return Err(ApiError::bad(A, "The board does not support sprints"));
        }
        let states: Vec<String> = q.list("state").iter().map(|s| s.to_ascii_lowercase()).collect();
        let all: Vec<Value> = tx
            .run("sprint.list", json!({"project": k}))?
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.get("id").and_then(Value::as_u64))
            .filter_map(|sid| tx.ws().sprints.get(&trackcraft_core::Id(sid)))
            .filter(|s| states.is_empty() || states.iter().any(|st| st == s.state.name()))
            .map(|s| sprint_json(&base, s))
            .collect();
        let start = q.num("startAt", 0, usize::MAX, A)?;
        let max = q.num("maxResults", MAX_VALUES, MAX_VALUES, A)?;
        let total = all.len();
        Ok(ok(values(start, max, total, all.into_iter().skip(start).take(max).collect())))
    })
    .await
}

async fn board_backlog(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), path.get("id"));
    cx.run(move |tx| {
        let k = board_project(tx.ws(), &id)?;
        let r = tx.run("board.backlog", json!({"project": k, "jql": q.get("jql")}))?;
        let ws = tx.ws();
        let issues: Vec<&Issue> = r
            .get("backlog")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| row.get("key").and_then(Value::as_str))
            .filter_map(|key| ws.issue(key).ok())
            .collect();
        Ok(ok(issue_page(ws, &base, &q, &issues)?))
    })
    .await
}

async fn board_issues(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), path.get("id"));
    cx.run(move |tx| {
        let k = board_project(tx.ws(), &id)?;
        let r = tx.run("board.view", json!({"project": k, "jql": q.get("jql")}))?;
        let ws = tx.ws();
        let issues: Vec<&Issue> = r
            .get("columns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|c| c.get("issues").and_then(Value::as_array).into_iter().flatten())
            .filter_map(|row| row.get("key").and_then(Value::as_str))
            .filter_map(|key| ws.issue(key).ok())
            .collect();
        Ok(ok(issue_page(ws, &base, &q, &issues)?))
    })
    .await
}

fn sprint_id(path: &P) -> Result<u64, ApiError> {
    path.get("id").trim().parse().map_err(|_| ApiError::bad(A, "the sprint id must be a number"))
}

async fn sprint(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), sprint_id(&path)?);
    cx.run(move |tx| {
        let s = tx.ws().sprints.get(&trackcraft_core::Id(id)).ok_or_else(|| ApiError::not_found(A, format!("Sprint {id} does not exist.")))?;
        Ok(ok(sprint_json(&base, s)))
    })
    .await
}

async fn sprint_issues(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, id) = (cx.base.clone(), sprint_id(&path)?);
    cx.run(move |tx| {
        let sid = trackcraft_core::Id(id);
        let (now, user) = (tx.s.now(), tx.user().to_string());
        let ws = tx.ws();
        if !ws.sprints.contains_key(&sid) {
            return Err(ApiError::not_found(A, format!("Sprint {id} does not exist.")));
        }
        let filter = jql_filter(ws, &user, now, q.get("jql"))?;
        let mut issues: Vec<&Issue> = ws
            .issues
            .values()
            .filter(|i| i.sprint == Some(sid) || i.past_sprints.contains(&sid))
            .filter(|i| filter.as_ref().is_none_or(|f| f.contains(&i.key.to_string())))
            .collect();
        issues.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.key.cmp(&b.key)));
        Ok(ok(issue_page(ws, &base, &q, &issues)?))
    })
    .await
}
