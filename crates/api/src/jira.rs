//! Jira Cloud REST v3 subset (`/rest/api/3/…`), shaped after Atlassian's public reference and
//! mapped onto engine commands. Reads build the public shapes from the workspace; every change
//! goes through `Session::run`.

use std::collections::HashSet;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get, routing::post, routing::put};
use serde_json::{Map, Value, json};
use trackcraft_engine::model::{Comment, Issue, LinkKind, PRIORITIES, Project, ProjectKind, StatusCategory, Timestamp, Workspace, priority_rank};

use crate::ApiState;
use crate::ctx::{Body, Ctx, P, Q};
use crate::error::{ApiError, Flavor};
use crate::ids;

/// Most issues one search page returns (the public API caps pages at 100 when fields are asked for).
pub const MAX_SEARCH: usize = 100;
/// Most comments / changelog entries one page returns.
pub const MAX_PAGE: usize = 1000;
const J: Flavor = Flavor::Jira;

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/rest/api/3/myself", get(myself))
        .route("/rest/api/3/project", get(projects))
        .route("/rest/api/3/project/{key}", get(project))
        .route("/rest/api/3/search/jql", get(search_get).post(search_post))
        .route("/rest/api/3/issue", post(issue_create))
        .route("/rest/api/3/issue/{key}", get(issue_get).put(issue_edit).delete(issue_delete))
        .route("/rest/api/3/issue/{key}/transitions", get(transitions).post(transition))
        .route("/rest/api/3/issue/{key}/comment", get(comments).post(comment_add))
        .route("/rest/api/3/issue/{key}/comment/{id}", get(comment_get).put(comment_edit).delete(comment_delete))
        .route("/rest/api/3/issue/{key}/assignee", put(assign))
        .route("/rest/api/3/issue/{key}/changelog", get(changelog))
        .route("/rest/api/3/issueLink", post(issue_link))
}

// ── shapes ────────────────────────────────────────────────────────────────────────────────────

/// Jira's date-time format: `2026-10-31T09:30:00.000+0000`.
pub fn jdate(t: &Timestamp) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string()
}

pub fn user(ws: &Workspace, base: &str, id: &str) -> Value {
    let u = ws.users.get(id);
    let mut v = json!({
        "self": format!("{base}/rest/api/3/user?accountId={}", crate::ctx::pct(id)),
        "accountId": id,
        "accountType": "atlassian",
        "displayName": u.map(|u| u.display_name.as_str()).unwrap_or(id),
        "active": u.is_some(),
        "timeZone": "UTC",
    });
    if let (Some(o), Some(e)) = (v.as_object_mut(), u.and_then(|u| u.email.as_deref())) {
        o.insert("emailAddress".into(), json!(e));
    }
    v
}

fn opt_user(ws: &Workspace, base: &str, id: Option<&str>) -> Value {
    id.map(|i| user(ws, base, i)).unwrap_or(Value::Null)
}

pub fn status_category(c: StatusCategory) -> Value {
    match c {
        StatusCategory::ToDo => json!({"id": 2, "key": "new", "colorName": "blue-gray", "name": "To Do"}),
        StatusCategory::InProgress => json!({"id": 4, "key": "indeterminate", "colorName": "yellow", "name": "In Progress"}),
        StatusCategory::Done => json!({"id": 3, "key": "done", "colorName": "green", "name": "Done"}),
    }
}

fn status(p: Option<&Project>, name: &str, cat: StatusCategory) -> Value {
    let id = p.and_then(|p| p.workflow.statuses.iter().position(|s| s.name.eq_ignore_ascii_case(name))).map(|i| i.saturating_add(1));
    json!({"id": id.map(|i| i.to_string()), "name": name, "statusCategory": status_category(cat)})
}

fn issue_type(p: Option<&Project>, name: &str) -> Value {
    let pos = p.and_then(|p| p.issue_types.iter().position(|t| t.name.eq_ignore_ascii_case(name)));
    let subtask = p.and_then(|p| pos.and_then(|i| p.issue_types.get(i))).is_some_and(|t| t.subtask);
    let level = if subtask {
        -1
    } else if name.eq_ignore_ascii_case("Epic") {
        1
    } else {
        0
    };
    json!({"id": pos.map(|i| i.saturating_add(1).to_string()), "name": name, "subtask": subtask, "hierarchyLevel": level})
}

fn priority(name: &str) -> Value {
    json!({"id": priority_rank(name).map(|r| r.to_string()), "name": name})
}

pub fn project_json(ws: &Workspace, base: &str, p: &Project, full: bool) -> Value {
    let id = ids::key_id(p.key.as_str()).to_string();
    let mut v = json!({
        "self": format!("{base}/rest/api/3/project/{id}"),
        "id": id,
        "key": p.key.as_str(),
        "name": p.name,
        "projectTypeKey": match p.kind { ProjectKind::Software => "software", ProjectKind::Business => "business" },
        "simplified": false,
        "style": "classic",
        "isPrivate": false,
    });
    if full && let Some(o) = v.as_object_mut() {
        o.insert("description".into(), json!(p.description.plain_text()));
        o.insert("lead".into(), opt_user(ws, base, p.lead.as_deref()));
        o.insert("issueTypes".into(), Value::Array(p.issue_types.iter().map(|t| issue_type(Some(p), &t.name)).collect()));
        o.insert("statuses".into(), Value::Array(p.workflow.statuses.iter().map(|s| status(Some(p), &s.name, s.category)).collect()));
    }
    v
}

fn issue_ref(ws: &Workspace, base: &str, i: &Issue) -> Value {
    let p = ws.projects.get(&i.key.project);
    let id = ids::issue_id(i.key.project.as_str(), i.key.number);
    json!({
        "id": id,
        "key": i.key.to_string(),
        "self": format!("{base}/rest/api/3/issue/{id}"),
        "fields": {
            "summary": i.summary,
            "status": status(p, &i.status, ws.status_category(i)),
            "priority": priority(&i.priority),
            "issuetype": issue_type(p, &i.issue_type),
        },
    })
}

fn link_type(k: LinkKind) -> Value {
    let (name, inward, outward) = match k {
        LinkKind::Blocks => ("Blocks", "is blocked by", "blocks"),
        LinkKind::Relates => ("Relates", "relates to", "relates to"),
        LinkKind::Duplicates => ("Duplicate", "is duplicated by", "duplicates"),
        LinkKind::Clones => ("Cloners", "is cloned by", "clones"),
    };
    json!({"name": name, "inward": inward, "outward": outward})
}

/// A link type from a public name (`Blocks`, `Relates`, `Duplicate`, `Cloners`) or our own.
fn parse_link_type(name: &str) -> Option<LinkKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "blocks" | "is blocked by" => Some(LinkKind::Blocks),
        "relates" | "relates to" => Some(LinkKind::Relates),
        "duplicate" | "duplicates" | "is duplicated by" => Some(LinkKind::Duplicates),
        "cloners" | "clones" | "is cloned by" => Some(LinkKind::Clones),
        _ => None,
    }
}

pub fn comment_json(ws: &Workspace, base: &str, issue: &Issue, c: &Comment) -> Value {
    let id = ids::issue_id(issue.key.project.as_str(), issue.key.number);
    json!({
        "self": format!("{base}/rest/api/3/issue/{id}/comment/{}", c.id.0),
        "id": c.id.0.to_string(),
        "author": user(ws, base, &c.author),
        "updateAuthor": user(ws, base, &c.author),
        "body": c.body.to_adf(),
        "created": jdate(&c.created),
        "updated": jdate(&c.updated),
    })
}

/// Which fields a caller asked for (`fields=summary,status`, `*all`, `*navigable`, `-comment`).
pub struct Fields {
    all: bool,
    include: HashSet<String>,
    exclude: HashSet<String>,
}

impl Fields {
    pub fn parse(list: &[String], default_all: bool) -> Fields {
        let mut f = Fields { all: false, include: HashSet::new(), exclude: HashSet::new() };
        let mut any = false;
        for name in list.iter().take(500) {
            match name.as_str() {
                "*all" | "*navigable" => {
                    f.all = true;
                    any = true;
                }
                "id" | "key" => any = true,
                n => {
                    if let Some(x) = n.strip_prefix('-') {
                        f.exclude.insert(x.to_string());
                    } else {
                        f.include.insert(n.to_string());
                        any = true;
                    }
                }
            }
        }
        if !any {
            f.all = default_all;
        }
        f
    }
    fn wants(&self, k: &str) -> bool {
        !self.exclude.contains(k) && (self.all || self.include.contains(k))
    }
}

/// An issue in the public shape, with the requested fields.
pub fn issue_json(ws: &Workspace, base: &str, i: &Issue, f: &Fields) -> Value {
    let p = ws.projects.get(&i.key.project);
    let id = ids::issue_id(i.key.project.as_str(), i.key.number);
    let mut fields = Map::new();
    let mut put = |k: &str, v: &dyn Fn() -> Value| {
        if f.wants(k) {
            fields.insert(k.to_string(), v());
        }
    };
    put("summary", &|| json!(i.summary));
    put("description", &|| if i.description.is_empty() { Value::Null } else { i.description.to_adf() });
    put("issuetype", &|| issue_type(p, &i.issue_type));
    put("status", &|| status(p, &i.status, ws.status_category(i)));
    put("statusCategory", &|| status_category(ws.status_category(i)));
    put("priority", &|| priority(&i.priority));
    put("assignee", &|| opt_user(ws, base, i.assignee.as_deref()));
    put("reporter", &|| user(ws, base, &i.reporter));
    put("creator", &|| user(ws, base, &i.reporter));
    put("labels", &|| json!(i.labels));
    put("created", &|| json!(jdate(&i.created)));
    put("updated", &|| json!(jdate(&i.updated)));
    put("resolutiondate", &|| i.resolved.map(|t| json!(jdate(&t))).unwrap_or(Value::Null));
    put("resolution", &|| if i.resolved.is_some() { json!({"name": "Done"}) } else { Value::Null });
    put("project", &|| p.map(|p| project_json(ws, base, p, false)).unwrap_or(Value::Null));
    put("parent", &|| i.parent.as_ref().and_then(|k| ws.issues.get(k)).map(|pi| issue_ref(ws, base, pi)).unwrap_or(Value::Null));
    put("subtasks", &|| {
        Value::Array(ws.issues.values().filter(|c| c.parent.as_ref() == Some(&i.key) && ws.is_subtask(c)).map(|c| issue_ref(ws, base, c)).collect())
    });
    put("issuelinks", &|| {
        let mut links: Vec<Value> = i
            .links
            .iter()
            .filter_map(|l| ws.issues.get(&l.target).map(|t| json!({"type": link_type(l.kind), "outwardIssue": issue_ref(ws, base, t)})))
            .collect();
        for other in ws.issues.values() {
            for l in other.links.iter().filter(|l| l.target == i.key) {
                links.push(json!({"type": link_type(l.kind), "inwardIssue": issue_ref(ws, base, other)}));
            }
        }
        Value::Array(links)
    });
    put("comment", &|| {
        let cs: Vec<Value> = i.comments.iter().map(|c| comment_json(ws, base, i, c)).collect();
        json!({"comments": cs, "startAt": 0, "maxResults": cs.len(), "total": cs.len()})
    });
    put("storyPoints", &|| json!(i.story_points));
    put("sprint", &|| i.sprint.and_then(|id| ws.sprints.get(&id)).map(|s| crate::agile::sprint_json(base, s)).unwrap_or(Value::Null));
    put("closedSprints", &|| {
        Value::Array(i.past_sprints.iter().filter_map(|id| ws.sprints.get(id)).map(|s| crate::agile::sprint_json(base, s)).collect())
    });
    let mut v = json!({"id": id, "key": i.key.to_string(), "self": format!("{base}/rest/api/3/issue/{id}")});
    if !fields.is_empty()
        && let Some(o) = v.as_object_mut()
    {
        o.insert("fields".into(), Value::Object(fields));
    }
    v
}

fn ok(v: Value) -> Response {
    Json(v).into_response()
}

fn created(v: Value) -> Response {
    (StatusCode::CREATED, Json(v)).into_response()
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn find_issue<'a>(ws: &'a Workspace, id_or_key: &str) -> Result<&'a Issue, ApiError> {
    let key = ids::issue_key(ws, id_or_key);
    ws.issue(&key).map_err(|_| {
        ApiError::not_found(J, format!("Issue does not exist or you do not have permission to see it: {}", key.chars().take(80).collect::<String>()))
    })
}

// ── field input ───────────────────────────────────────────────────────────────────────────────

/// `{"key": …}`, `{"id": …}`, `{"name": …}`, `{"accountId": …}` or a bare string.
fn ref_str(v: &Value, keys: &[&str]) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(o) => keys.iter().find_map(|k| match o.get(*k) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }),
        _ => None,
    }
}

fn field_err(name: &str, what: &str) -> ApiError {
    ApiError::bad(J, format!("field '{name}': {what}"))
}

/// Maps Jira `fields` onto `issue.create` / `issue.edit` params. `project` is resolved by the caller.
fn issue_params(ws: &Workspace, project: Option<&Project>, fields: &Map<String, Value>, out: &mut Map<String, Value>) -> Result<(), ApiError> {
    for (k, v) in fields {
        match k.as_str() {
            "summary" => {
                out.insert("summary".into(), json!(v.as_str().ok_or_else(|| field_err(k, "must be a string"))?));
            }
            "description" => {
                if !(v.is_null() || v.is_string() || v.is_object()) {
                    return Err(field_err(k, "must be an ADF document"));
                }
                out.insert("description".into(), if v.is_null() { json!("") } else { v.clone() });
            }
            "issuetype" => {
                let name = match (v.get("name").and_then(Value::as_str), ref_str(v, &["id"])) {
                    (Some(n), _) => n.to_string(),
                    (None, Some(id)) => id
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| project.and_then(|p| p.issue_types.get(i.checked_sub(1)?)))
                        .map(|t| t.name.clone())
                        .unwrap_or(id),
                    (None, None) => return Err(field_err(k, "needs a name or id")),
                };
                out.insert("type".into(), json!(name));
            }
            "priority" => {
                let name = match (v.get("name").and_then(Value::as_str), ref_str(v, &["id"])) {
                    (Some(n), _) => n.to_string(),
                    (None, Some(id)) => id.parse::<usize>().ok().and_then(|i| PRIORITIES.get(i.checked_sub(1)?)).map(|s| s.to_string()).unwrap_or(id),
                    (None, None) => return Err(field_err(k, "needs a name or id")),
                };
                out.insert("priority".into(), json!(name));
            }
            "labels" => {
                out.insert("labels".into(), v.clone());
            }
            "parent" => {
                let parent = if v.is_null() {
                    Value::Null
                } else {
                    json!(ids::issue_key(ws, &ref_str(v, &["key", "id"]).ok_or_else(|| field_err(k, "needs a key or id"))?))
                };
                out.insert("parent".into(), parent);
            }
            "assignee" => {
                let who = if v.is_null() {
                    Value::Null
                } else {
                    json!(ref_str(v, &["accountId", "id", "name"]).ok_or_else(|| field_err(k, "needs an accountId"))?)
                };
                out.insert("assignee".into(), who);
            }
            "storyPoints" => {
                out.insert("storyPoints".into(), v.clone());
            }
            "sprint" => {
                out.insert("sprint".into(), json!(ref_str(v, &["id"]).ok_or_else(|| field_err(k, "needs a sprint id"))?));
            }
            // Accepted and ignored: set by the server, or not modelled yet.
            "project" | "reporter" | "environment" | "duedate" | "components" | "fixVersions" | "versions" => {}
            other if other.starts_with("customfield_") => {}
            other => return Err(field_err(other, "is not supported by this server")),
        }
    }
    Ok(())
}

// ── handlers ──────────────────────────────────────────────────────────────────────────────────

async fn myself(cx: Ctx) -> Result<Response, ApiError> {
    let base = cx.base.clone();
    cx.run(move |tx| {
        let mut v = user(tx.ws(), &base, tx.user());
        if let Some(o) = v.as_object_mut() {
            o.insert("locale".into(), json!("en_US"));
        }
        Ok(ok(v))
    })
    .await
}

async fn projects(cx: Ctx) -> Result<Response, ApiError> {
    let base = cx.base.clone();
    cx.run(move |tx| {
        let ws = tx.ws();
        Ok(ok(Value::Array(ws.projects.values().map(|p| project_json(ws, &base, p, false)).collect())))
    })
    .await
}

async fn project(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let (base, key) = (cx.base.clone(), path.get("key"));
    cx.run(move |tx| {
        let ws = tx.ws();
        let k = ids::project_key(ws, &key)
            .ok_or_else(|| ApiError::not_found(J, format!("No project could be found with key '{}'.", key.chars().take(80).collect::<String>())))?;
        let p = ws.project(&k).map_err(|e| ApiError::not_found(J, e.to_string()))?;
        Ok(ok(project_json(ws, &base, p, true)))
    })
    .await
}

struct SearchReq {
    jql: String,
    start: usize,
    max: usize,
    fields: Vec<String>,
}

/// `nextPageToken` is opaque to clients; ours is the next offset.
fn page_token(t: &str) -> Result<usize, ApiError> {
    t.strip_prefix('o').and_then(|n| n.parse().ok()).ok_or_else(|| ApiError::bad(J, "invalid nextPageToken"))
}

async fn search_get(cx: Ctx, q: Q) -> Result<Response, ApiError> {
    let start = match q.get("nextPageToken").filter(|t| !t.is_empty()) {
        Some(t) => page_token(t)?,
        None => q.num("startAt", 0, usize::MAX, J)?,
    };
    let req = SearchReq {
        jql: q.get("jql").unwrap_or_default().to_string(),
        start,
        max: q.num("maxResults", 50, MAX_SEARCH, J)?,
        fields: q.list("fields"),
    };
    search(cx, req).await
}

async fn search_post(cx: Ctx, Body(b): Body) -> Result<Response, ApiError> {
    if !(b.is_object() || b.is_null()) {
        return Err(ApiError::bad(J, "the request body must be a JSON object"));
    }
    let start = match b.get("nextPageToken").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        Some(t) => page_token(t)?,
        None => num(&b, "startAt", 0, usize::MAX)?,
    };
    let fields = match b.get("fields") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_string()).collect(),
        Some(_) => return Err(ApiError::bad(J, "'fields' must be a list of field names")),
    };
    let jql = match b.get("jql") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(ApiError::bad(J, "'jql' must be a string")),
    };
    search(cx, SearchReq { jql, start, max: num(&b, "maxResults", 50, MAX_SEARCH)?, fields }).await
}

fn num(b: &Value, k: &str, default: usize, max: usize) -> Result<usize, ApiError> {
    match b.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(|n| n.min(max))
            .ok_or_else(|| ApiError::bad(J, format!("'{k}' must be a whole number"))),
    }
}

async fn search(cx: Ctx, req: SearchReq) -> Result<Response, ApiError> {
    let base = cx.base.clone();
    cx.run(move |tx| {
        let r = tx.run("search.jql", json!({"jql": req.jql, "startAt": req.start, "maxResults": req.max}))?;
        let total = r.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
        let f = Fields::parse(&req.fields, false);
        let ws = tx.ws();
        let issues: Vec<Value> = r
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|b| b.get("key").and_then(Value::as_str))
            .filter_map(|k| ws.issue(k).ok())
            .map(|i| issue_json(ws, &base, i, &f))
            .collect();
        let next = req.start.saturating_add(issues.len());
        let is_last = next >= total || issues.is_empty();
        let mut v = json!({"issues": issues, "isLast": is_last});
        if !is_last && let Some(o) = v.as_object_mut() {
            o.insert("nextPageToken".into(), json!(format!("o{next}")));
        }
        Ok(ok(v))
    })
    .await
}

async fn issue_create(cx: Ctx, Body(b): Body) -> Result<Response, ApiError> {
    let base = cx.base.clone();
    cx.run(move |tx| {
        let fields = b.get("fields").and_then(Value::as_object).ok_or_else(|| ApiError::bad(J, "the body needs a \"fields\" object"))?;
        let pref = fields.get("project").and_then(|p| ref_str(p, &["key", "id"])).ok_or_else(|| field_err("project", "is required (a key or id)"))?;
        let ws = tx.ws();
        let key = ids::project_key(ws, &pref).ok_or_else(|| field_err("project", "no such project"))?;
        if fields.get("summary").is_none() {
            return Err(field_err("summary", "is required"));
        }
        let mut params = Map::new();
        issue_params(ws, ws.projects.values().find(|p| p.key.as_str() == key), fields, &mut params)?;
        params.insert("project".into(), json!(key));
        if params.get("assignee").is_some_and(Value::is_null) {
            params.remove("assignee");
        }
        let r = tx.run("issue.create", Value::Object(params))?;
        let k = r.get("key").and_then(Value::as_str).unwrap_or_default();
        let i = find_issue(tx.ws(), k)?;
        let id = ids::issue_id(i.key.project.as_str(), i.key.number);
        Ok(created(json!({"id": id, "key": k, "self": format!("{base}/rest/api/3/issue/{id}")})))
    })
    .await
}

async fn issue_get(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, key, fields) = (cx.base.clone(), path.get("key"), q.list("fields"));
    cx.run(move |tx| {
        let ws = tx.ws();
        let i = find_issue(ws, &key)?;
        Ok(ok(issue_json(ws, &base, i, &Fields::parse(&fields, true))))
    })
    .await
}

/// `update` verbs: `[{"add": x}, {"remove": y}, {"set": z}]`.
fn verbs(v: &Value, name: &str) -> Result<Vec<(String, Value)>, ApiError> {
    let ops = v.as_array().ok_or_else(|| field_err(name, "update must be a list of operations"))?;
    let mut out = Vec::new();
    for op in ops.iter().take(1000) {
        let o = op.as_object().ok_or_else(|| field_err(name, "each operation is an object such as {\"add\": …}"))?;
        for (verb, val) in o {
            out.push((verb.clone(), val.clone()));
        }
    }
    Ok(out)
}

async fn issue_edit(cx: Ctx, path: P, q: Q, Body(b): Body) -> Result<Response, ApiError> {
    let (base, key, return_issue) = (cx.base.clone(), path.get("key"), q.flag("returnIssue"));
    cx.run(move |tx| {
        tx.atomic();
        let k = find_issue(tx.ws(), &key)?.key.to_string();
        let mut params = Map::new();
        if let Some(f) = b.get("fields") {
            let f = f.as_object().ok_or_else(|| ApiError::bad(J, "\"fields\" must be an object"))?;
            let ws = tx.ws();
            let proj = ws.projects.values().find(|p| k.starts_with(&format!("{}-", p.key.as_str())));
            issue_params(ws, proj, f, &mut params)?;
        }
        let mut comments = Vec::new();
        if let Some(u) = b.get("update") {
            let u = u.as_object().ok_or_else(|| ApiError::bad(J, "\"update\" must be an object"))?;
            for (field, ops) in u {
                for (verb, val) in verbs(ops, field)? {
                    match (field.as_str(), verb.as_str()) {
                        ("labels", "set") => {
                            params.insert("labels".into(), val);
                        }
                        ("labels", "add" | "remove") => {
                            let mut cur: Vec<Value> = match params.get("labels").and_then(Value::as_array) {
                                Some(a) => a.clone(),
                                None => tx.ws().issue(&k).map(|i| i.labels.iter().map(|l| json!(l)).collect()).unwrap_or_default(),
                            };
                            if verb == "add" {
                                if !cur.contains(&val) {
                                    cur.push(val);
                                }
                            } else {
                                cur.retain(|l| *l != val);
                            }
                            params.insert("labels".into(), Value::Array(cur));
                        }
                        ("summary" | "description", "set") => {
                            let mut one = Map::new();
                            one.insert(field.clone(), val);
                            let ws = tx.ws();
                            issue_params(ws, None, &one, &mut params)?;
                        }
                        ("comment", "add") => comments.push(val.get("body").cloned().ok_or_else(|| field_err("comment", "add needs a body"))?),
                        _ => return Err(field_err(field, &format!("the '{verb}' operation is not supported"))),
                    }
                }
            }
        }
        let assignee = params.remove("assignee");
        if params.is_empty() && assignee.is_none() && comments.is_empty() {
            return Err(ApiError::bad(J, "nothing to change: send \"fields\" or \"update\""));
        }
        if !params.is_empty() {
            params.insert("key".into(), json!(k));
            tx.run("issue.edit", Value::Object(params))?;
        }
        if let Some(a) = assignee {
            tx.run("issue.assign", json!({"key": k, "assignee": a}))?;
        }
        for body in comments {
            tx.run("issue.comment", json!({"key": k, "body": body}))?;
        }
        if return_issue {
            let ws = tx.ws();
            return Ok(ok(issue_json(ws, &base, find_issue(ws, &k)?, &Fields::parse(&[], true))));
        }
        Ok(no_content())
    })
    .await
}

async fn issue_delete(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (key, subtasks) = (path.get("key"), q.flag("deleteSubtasks"));
    cx.run(move |tx| {
        let ws = tx.ws();
        let i = find_issue(ws, &key)?;
        let has_subtasks = ws.issues.values().any(|c| c.parent.as_ref() == Some(&i.key) && ws.is_subtask(c));
        if has_subtasks && !subtasks {
            return Err(ApiError::bad(J, format!("{} has sub-tasks: pass deleteSubtasks=true to delete them too", i.key)));
        }
        let k = i.key.to_string();
        tx.run("issue.delete", json!({"key": k}))?;
        Ok(no_content())
    })
    .await
}

/// Transition ids are positions in the project's workflow (1-based).
fn transition_list(ws: &Workspace, i: &Issue) -> Vec<Value> {
    let Some(p) = ws.projects.get(&i.key.project) else { return vec![] };
    let avail = p.workflow.available(&i.status);
    p.workflow
        .transitions
        .iter()
        .enumerate()
        .filter(|(_, t)| avail.iter().any(|a| std::ptr::eq(*a, *t)))
        .map(|(n, t)| {
            let cat = p.workflow.status(&t.to).map(|s| s.category).unwrap_or(StatusCategory::ToDo);
            json!({
                "id": n.saturating_add(1).to_string(),
                "name": t.name,
                "to": status(Some(p), &t.to, cat),
                "hasScreen": false,
                "isGlobal": t.from.is_empty(),
                "isInitial": false,
                "isAvailable": true,
                "isConditional": false,
            })
        })
        .collect()
}

async fn transitions(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (key, only) = (path.get("key"), q.get("transitionId").map(str::to_string));
    cx.run(move |tx| {
        let ws = tx.ws();
        let i = find_issue(ws, &key)?;
        let mut list = transition_list(ws, i);
        if let Some(id) = only {
            list.retain(|t| t.get("id").and_then(Value::as_str) == Some(id.as_str()));
        }
        Ok(ok(json!({"expand": "transitions", "transitions": list})))
    })
    .await
}

async fn transition(cx: Ctx, path: P, Body(b): Body) -> Result<Response, ApiError> {
    let key = path.get("key");
    cx.run(move |tx| {
        tx.atomic();
        let ws = tx.ws();
        let i = find_issue(ws, &key)?;
        let t = b.get("transition").ok_or_else(|| ApiError::bad(J, "the body needs a \"transition\": {\"id\": …}"))?;
        let list = transition_list(ws, i);
        let wanted_id = ref_str(t, &["id"]);
        let wanted_name = t.get("name").and_then(Value::as_str);
        let to = list
            .iter()
            .find(|x| match (&wanted_id, wanted_name) {
                (Some(id), _) => x.get("id").and_then(Value::as_str) == Some(id.as_str()),
                (None, Some(n)) => x.get("name").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case(n)),
                (None, None) => false,
            })
            .and_then(|x| x["to"].get("name").and_then(Value::as_str).map(str::to_string))
            .ok_or_else(|| {
                ApiError::bad(
                    J,
                    format!("Transition id '{}' is not valid for this issue.", wanted_id.unwrap_or_default().chars().take(40).collect::<String>()),
                )
            })?;
        let k = i.key.to_string();
        tx.run("issue.transition", json!({"key": k, "to": to}))?;
        let comments = match b.get("update").and_then(|u| u.get("comment")) {
            Some(ops) => verbs(ops, "comment")?,
            None => vec![],
        };
        for (verb, val) in comments {
            if verb != "add" {
                return Err(field_err("comment", "only 'add' is supported during a transition"));
            }
            let body = val.get("body").cloned().ok_or_else(|| field_err("comment", "add needs a body"))?;
            tx.run("issue.comment", json!({"key": k, "body": body}))?;
        }
        Ok(no_content())
    })
    .await
}

async fn comments(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, key) = (cx.base.clone(), path.get("key"));
    let start = q.num("startAt", 0, usize::MAX, J)?;
    let max = q.num("maxResults", 50, MAX_PAGE, J)?;
    let newest_first = q.get("orderBy").is_some_and(|o| o.trim() == "-created");
    cx.run(move |tx| {
        let ws = tx.ws();
        let i = find_issue(ws, &key)?;
        let mut all: Vec<&Comment> = i.comments.iter().collect();
        if newest_first {
            all.reverse();
        }
        let page: Vec<Value> = all.iter().skip(start).take(max).map(|c| comment_json(ws, &base, i, c)).collect();
        Ok(ok(json!({"startAt": start, "maxResults": max, "total": all.len(), "comments": page})))
    })
    .await
}

async fn comment_add(cx: Ctx, path: P, Body(b): Body) -> Result<Response, ApiError> {
    let (base, key) = (cx.base.clone(), path.get("key"));
    cx.run(move |tx| {
        let k = find_issue(tx.ws(), &key)?.key.to_string();
        let body = b.get("body").cloned().ok_or_else(|| ApiError::bad(J, "the body needs a \"body\" (an ADF document)"))?;
        let r = tx.run("issue.comment", json!({"key": k, "body": body}))?;
        let id = r.get("id").and_then(Value::as_u64).unwrap_or_default();
        let ws = tx.ws();
        let i = find_issue(ws, &k)?;
        let c = i.comments.iter().find(|c| c.id.0 == id).ok_or_else(|| ApiError::internal(J, "the new comment is missing"))?;
        Ok(created(comment_json(ws, &base, i, c)))
    })
    .await
}

fn comment_id(path: &P) -> Result<u64, ApiError> {
    path.get("id").trim().parse().map_err(|_| ApiError::bad(J, "the comment id must be a number"))
}

async fn comment_get(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let (base, key, id) = (cx.base.clone(), path.get("key"), comment_id(&path)?);
    cx.run(move |tx| {
        let ws = tx.ws();
        let i = find_issue(ws, &key)?;
        let c = i.comments.iter().find(|c| c.id.0 == id).ok_or_else(|| ApiError::not_found(J, format!("no comment with id {id}")))?;
        Ok(ok(comment_json(ws, &base, i, c)))
    })
    .await
}

async fn comment_edit(cx: Ctx, path: P, Body(b): Body) -> Result<Response, ApiError> {
    let (base, key, id) = (cx.base.clone(), path.get("key"), comment_id(&path)?);
    cx.run(move |tx| {
        let k = find_issue(tx.ws(), &key)?.key.to_string();
        let body = b.get("body").cloned().ok_or_else(|| ApiError::bad(J, "the body needs a \"body\" (an ADF document)"))?;
        tx.run("issue.editComment", json!({"key": k, "id": id, "body": body}))?;
        let ws = tx.ws();
        let i = find_issue(ws, &k)?;
        let c = i.comments.iter().find(|c| c.id.0 == id).ok_or_else(|| ApiError::not_found(J, format!("no comment with id {id}")))?;
        Ok(ok(comment_json(ws, &base, i, c)))
    })
    .await
}

async fn comment_delete(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let (key, id) = (path.get("key"), comment_id(&path)?);
    cx.run(move |tx| {
        let k = find_issue(tx.ws(), &key)?.key.to_string();
        tx.run("issue.deleteComment", json!({"key": k, "id": id}))?;
        Ok(no_content())
    })
    .await
}

async fn assign(cx: Ctx, path: P, Body(b): Body) -> Result<Response, ApiError> {
    let key = path.get("key");
    cx.run(move |tx| {
        let ws = tx.ws();
        let i = find_issue(ws, &key)?;
        let who = match b.get("accountId").or_else(|| b.get("name")) {
            None if !b.is_object() => return Err(ApiError::bad(J, "the body needs \"accountId\" (a user id, or null to unassign)")),
            None | Some(Value::Null) => Value::Null,
            // "-1": the project's default assignee (its lead).
            Some(Value::String(s)) if s == "-1" => {
                ws.projects.get(&i.key.project).and_then(|p| p.lead.clone()).map(Value::String).unwrap_or(Value::Null)
            }
            Some(Value::String(s)) => json!(s),
            Some(_) => return Err(ApiError::bad(J, "\"accountId\" must be a string or null")),
        };
        let k = i.key.to_string();
        tx.run("issue.assign", json!({"key": k, "assignee": who}))?;
        Ok(no_content())
    })
    .await
}

async fn changelog(cx: Ctx, path: P, q: Q) -> Result<Response, ApiError> {
    let (base, key) = (cx.base.clone(), path.get("key"));
    let start = q.num("startAt", 0, usize::MAX, J)?;
    let max = q.num("maxResults", 100, MAX_PAGE, J)?;
    cx.run(move |tx| {
        let k = find_issue(tx.ws(), &key)?.key.to_string();
        let r = tx.run("issue.history", json!({"key": k}))?;
        let mut groups: Vec<Value> = r.as_array().cloned().unwrap_or_default();
        groups.reverse(); // the public changelog is oldest first
        let ws = tx.ws();
        let total = groups.len();
        let values: Vec<Value> = groups
            .iter()
            .skip(start)
            .take(max)
            .map(|g| {
                let author = g.get("author").and_then(Value::as_str).unwrap_or_default();
                let items: Vec<Value> = g
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|it| json!({"field": it["field"], "fieldtype": "jira", "from": null, "fromString": it["fromString"], "to": null, "toString": it["toString"]}))
                    .collect();
                let when = g.get("created").and_then(Value::as_str).and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| jdate(&t.to_utc()));
                json!({"id": g["id"].to_string(), "author": user(ws, &base, author), "created": when, "items": items})
            })
            .collect();
        let is_last = start.saturating_add(values.len()) >= total;
        Ok(ok(json!({"startAt": start, "maxResults": max, "total": total, "isLast": is_last, "values": values})))
    })
    .await
}

/// `{"type": {"name": "Blocks"}, "inwardIssue": {"key": A}, "outwardIssue": {"key": B}}`: A blocks B
/// (on A the link reads with the type's outward text, on B with the inward text).
async fn issue_link(cx: Ctx, Body(b): Body) -> Result<Response, ApiError> {
    cx.run(move |tx| {
        let ty = b
            .get("type")
            .and_then(|t| ref_str(t, &["name", "outward", "inward"]))
            .ok_or_else(|| field_err("type", "is required ({\"name\": \"Blocks\"})"))?;
        let kind = parse_link_type(&ty).ok_or_else(|| field_err("type", "is one of Blocks, Relates, Duplicate, Cloners"))?;
        let ws = tx.ws();
        let inward = b.get("inwardIssue").and_then(|v| ref_str(v, &["key", "id"])).ok_or_else(|| field_err("inwardIssue", "is required"))?;
        let outward = b.get("outwardIssue").and_then(|v| ref_str(v, &["key", "id"])).ok_or_else(|| field_err("outwardIssue", "is required"))?;
        let (from, to) = (find_issue(ws, &inward)?.key.to_string(), find_issue(ws, &outward)?.key.to_string());
        tx.atomic();
        tx.run("issue.link", json!({"key": from, "type": kind.outward(), "target": to}))?;
        if let Some(body) = b.get("comment").and_then(|c| c.get("body")).cloned() {
            tx.run("issue.comment", json!({"key": from, "body": body}))?;
        }
        Ok(StatusCode::CREATED.into_response())
    })
    .await
}
