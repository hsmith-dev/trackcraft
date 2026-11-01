//! M2.4: project components and versions, the field list, and components / versions /
//! `customfield_*` in issue reads and writes — Jira Cloud REST v3 shapes (public reference).

use axum::Json;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use trackcraft_engine::model::{FieldKind, Issue, Project, Workspace, field_id_of};

use crate::ctx::{Ctx, P};
use crate::error::{ApiError, Flavor};
use crate::ids;

const J: Flavor = Flavor::Jira;

/// System fields `GET /rest/api/3/field` lists: (id, name, schema type, JQL clause names).
const SYSTEM: &[(&str, &str, &str, &[&str])] = &[
    ("summary", "Summary", "string", &["summary"]),
    ("description", "Description", "string", &["description"]),
    ("issuetype", "Issue Type", "issuetype", &["issuetype", "type"]),
    ("status", "Status", "status", &["status"]),
    ("statusCategory", "Status Category", "statusCategory", &["statusCategory"]),
    ("priority", "Priority", "priority", &["priority"]),
    ("assignee", "Assignee", "user", &["assignee"]),
    ("reporter", "Reporter", "user", &["reporter"]),
    ("creator", "Creator", "user", &["creator"]),
    ("labels", "Labels", "array", &["labels"]),
    ("created", "Created", "datetime", &["created", "createdDate"]),
    ("updated", "Updated", "datetime", &["updated", "updatedDate"]),
    ("resolutiondate", "Resolved", "datetime", &["resolved", "resolutiondate"]),
    ("resolution", "Resolution", "resolution", &["resolution"]),
    ("project", "Project", "project", &["project"]),
    ("parent", "Parent", "issuelink", &["parent"]),
    ("subtasks", "Sub-tasks", "array", &["subtasks"]),
    ("issuelinks", "Linked Issues", "array", &["issueLink"]),
    ("comment", "Comment", "comments-page", &["comment"]),
    ("components", "Components", "array", &["component"]),
    ("fixVersions", "Fix versions", "array", &["fixVersion"]),
    ("versions", "Affects versions", "array", &["affectedVersion"]),
    ("storyPoints", "Story Points", "number", &["\"Story Points\""]),
    ("sprint", "Sprint", "array", &["sprint"]),
];

fn kind_schema(k: &FieldKind) -> Value {
    match k {
        FieldKind::Text | FieldKind::Url => json!({"type": "string"}),
        FieldKind::Number => json!({"type": "number"}),
        FieldKind::Date => json!({"type": "date"}),
        FieldKind::Select { .. } => json!({"type": "option"}),
        FieldKind::MultiSelect { .. } => json!({"type": "array", "items": "option"}),
        FieldKind::User => json!({"type": "user"}),
    }
}

fn component_ref(base: &str, id: u64, name: &str) -> Value {
    json!({"self": format!("{base}/rest/api/3/component/{id}"), "id": id.to_string(), "name": name})
}

fn version_json(base: &str, project: &str, v: &trackcraft_engine::model::Version) -> Value {
    let mut o = json!({
        "self": format!("{base}/rest/api/3/version/{}", v.id.0),
        "id": v.id.0.to_string(),
        "name": v.name,
        "archived": v.archived,
        "released": v.released,
        "projectId": ids::key_id(project),
    });
    if let Some(m) = o.as_object_mut() {
        if !v.description.is_empty() {
            m.insert("description".into(), json!(v.description));
        }
        if let Some(d) = v.start {
            m.insert("startDate".into(), json!(d.format("%Y-%m-%d").to_string()));
        }
        if let Some(d) = v.release_date {
            m.insert("releaseDate".into(), json!(d.format("%Y-%m-%d").to_string()));
        }
    }
    o
}

/// `components`, `fixVersions`, `versions` and one `customfield_<id>` per field of the issue's
/// project, in the public shapes (select `{"value"}`, user objects, dates `YYYY-MM-DD`).
pub fn issue_values(ws: &Workspace, base: &str, i: &Issue) -> Vec<(String, Value)> {
    let Some(d) = ws.project_fields(&i.key.project) else {
        return vec![("components".into(), json!([])), ("fixVersions".into(), json!([])), ("versions".into(), json!([]))];
    };
    let pk = i.key.project.as_str();
    let versions =
        |ids: &[trackcraft_core::Id]| Value::Array(ids.iter().filter_map(|id| d.version(*id)).map(|v| version_json(base, pk, v)).collect());
    let mut out = vec![
        (
            "components".to_string(),
            Value::Array(i.components.iter().filter_map(|id| d.component(*id)).map(|c| component_ref(base, c.id.0, &c.name)).collect()),
        ),
        ("fixVersions".to_string(), versions(&i.fix_versions)),
        ("versions".to_string(), versions(&i.affects_versions)),
    ];
    for f in &d.fields {
        let v = match i.custom.get(&f.id) {
            None => Value::Null,
            Some(trackcraft_engine::model::FieldValue::User(u)) => crate::jira::user(ws, base, u),
            Some(trackcraft_engine::model::FieldValue::Select(s)) => json!({"value": s}),
            Some(trackcraft_engine::model::FieldValue::MultiSelect(list)) => Value::Array(list.iter().map(|s| json!({"value": s})).collect()),
            Some(trackcraft_engine::model::FieldValue::Number(n)) => json!(n),
            Some(other) => json!(other.display()),
        };
        out.push((format!("customfield_{}", f.id), v));
    }
    out
}

/// Maps `components`, `fixVersions`, `versions` and known `customfield_<id>` entries of a
/// request's `fields` onto `issue.create` / `issue.edit` params. Returns `false` for anything
/// else (unknown `customfield_*` stay accepted and ignored, as before).
pub fn map_input(project: Option<&Project>, ws: &Workspace, k: &str, v: &Value, out: &mut Map<String, Value>) -> Result<bool, ApiError> {
    let list = |v: &Value| if v.is_null() { json!([]) } else { v.clone() };
    match k {
        "components" => out.insert("components".into(), list(v)),
        "fixVersions" => out.insert("fixVersions".into(), list(v)),
        "versions" => out.insert("affectsVersions".into(), list(v)),
        _ if k.starts_with("customfield_") => {
            let known = field_id_of(k).is_some_and(|id| project.and_then(|p| ws.project_fields(&p.key)).and_then(|d| d.field(id)).is_some());
            if !known {
                return Ok(false);
            }
            let custom = out.entry("custom").or_insert_with(|| json!({}));
            match custom.as_object_mut() {
                Some(m) => m.insert(k.to_string(), v.clone()),
                None => return Err(ApiError::bad(J, "custom fields could not be read")),
            };
            None
        }
        _ => return Ok(false),
    };
    Ok(true)
}

fn not_found(key: &str) -> ApiError {
    ApiError::not_found(J, format!("No project could be found with key '{}'.", key.chars().take(80).collect::<String>()))
}

/// `GET /rest/api/3/project/{key}/components`.
pub async fn project_components(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let (base, key) = (cx.base.clone(), path.get("key"));
    cx.run(move |tx| {
        let ws = tx.ws();
        let k = ids::project_key(ws, &key).ok_or_else(|| not_found(&key))?;
        let p = ws.project(&k).map_err(|_| not_found(&key))?;
        let list: Vec<Value> = ws
            .project_fields(&p.key)
            .map(|d| d.components.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|c| {
                let mut v = component_ref(&base, c.id.0, &c.name);
                if let Some(o) = v.as_object_mut() {
                    o.insert("description".into(), json!(c.description));
                    if let Some(l) = &c.lead {
                        o.insert("lead".into(), crate::jira::user(ws, &base, l));
                    }
                    o.insert("project".into(), json!(p.key.as_str()));
                    o.insert("projectId".into(), json!(ids::key_id(p.key.as_str())));
                }
                v
            })
            .collect();
        Ok(Json(Value::Array(list)).into_response())
    })
    .await
}

/// `GET /rest/api/3/project/{key}/versions`.
pub async fn project_versions(cx: Ctx, path: P) -> Result<Response, ApiError> {
    let (base, key) = (cx.base.clone(), path.get("key"));
    cx.run(move |tx| {
        let ws = tx.ws();
        let k = ids::project_key(ws, &key).ok_or_else(|| not_found(&key))?;
        let p = ws.project(&k).map_err(|_| not_found(&key))?;
        let list: Vec<Value> = ws
            .project_fields(&p.key)
            .map(|d| d.versions.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|v| version_json(&base, p.key.as_str(), v))
            .collect();
        Ok(Json(Value::Array(list)).into_response())
    })
    .await
}

/// `GET /rest/api/3/field`: system fields, then the custom fields of every project the caller
/// can see.
pub async fn field_list(cx: Ctx) -> Result<Response, ApiError> {
    cx.run(move |tx| {
        let ws = tx.ws();
        let mut out: Vec<Value> = SYSTEM
            .iter()
            .map(|(id, name, ty, clauses)| {
                json!({"id": id, "key": id, "name": name, "custom": false, "orderable": true, "navigable": true, "searchable": true,
                       "clauseNames": clauses, "schema": {"type": ty, "system": id}})
            })
            .collect();
        for (pk, d) in &ws.fields {
            for f in &d.fields {
                let key = format!("customfield_{}", f.id);
                let mut schema = kind_schema(&f.kind);
                if let Some(o) = schema.as_object_mut() {
                    o.insert("customId".into(), json!(f.id.0));
                }
                out.push(json!({
                    "id": key, "key": key, "name": f.name, "custom": true, "orderable": true, "navigable": true, "searchable": true,
                    "clauseNames": [format!("cf[{}]", f.id), f.name.clone()],
                    "schema": schema,
                    "description": f.description,
                    "scope": {"type": "PROJECT", "project": {"id": ids::key_id(pk.as_str()).to_string(), "key": pk.as_str()}},
                }));
            }
        }
        Ok(Json(Value::Array(out)).into_response())
    })
    .await
}
