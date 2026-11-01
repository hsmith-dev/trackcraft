//! M2.4: custom fields, components and versions (releases) of a project, and the params
//! `issue.create` / `issue.edit` take for their values.
//!
//! Definitions need a project administrator (`authz`); values need edit rights on the issue.

use serde_json::{Value, json};
use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::{
    ComponentEdit, FieldDef, FieldEdit, FieldKind, FieldValue, IssueFields, MAX_FIELDS, MAX_OPTIONS, MAX_PER_ISSUE, NewVersion, ProjectFields,
    VersionEdit, VersionList, Workspace, parse_day,
};

use crate::{CmdResult, CommandSpec, Session, json as view, params};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("field.list", "Custom Fields", "Project › Fields", field_list).params(r#"{"project": "LUME"}"#).query(),
        CommandSpec::new("field.create", "Create Custom Field", "Project › Fields", field_create).params(
            r#"{"project": "LUME", "name": "Customer", "type": "text"|"number"|"date"|"select"|"multiSelect"|"user"|"url", "options": ["a", "b"] (select kinds), "required": false, "description": "…"}"#,
        ),
        CommandSpec::new("field.edit", "Edit Custom Field", "Project › Fields", field_edit).params(
            r#"{"id": 12, "name": "…", "options": [...] (values using a removed option are cleared), "required": true, "description": "…"} (each optional)"#,
        ),
        CommandSpec::new("field.delete", "Delete Custom Field", "Project › Fields", field_delete)
            .params(r#"{"id": 12} (removes its values from every issue)"#),
        CommandSpec::new("component.list", "Components", "Project › Components", component_list).params(r#"{"project": "LUME"}"#).query(),
        CommandSpec::new("component.create", "Create Component", "Project › Components", component_create)
            .params(r#"{"project": "LUME", "name": "Docs", "lead": "ann", "description": "…"} → the component"#),
        CommandSpec::new("component.edit", "Edit Component", "Project › Components", component_edit)
            .params(r#"{"id": 13, "name": "…", "lead": "ann" | null, "description": "…"} (each optional)"#),
        CommandSpec::new("component.delete", "Delete Component", "Project › Components", component_delete).params(r#"{"id": 13}"#),
        CommandSpec::new("version.list", "Versions", "Project › Releases", version_list).params(r#"{"project": "LUME"}"#).query(),
        CommandSpec::new("version.create", "Create Version", "Project › Releases", version_create).params(
            r#"{"project": "LUME", "name": "1.2", "description": "…", "startDate": "2026-11-01", "releaseDate": "2026-12-01", "released": false} → the version"#,
        ),
        CommandSpec::new("version.edit", "Edit Version", "Project › Releases", version_edit).params(
            r#"{"id": 14, "name": "…", "description": "…", "startDate": "…" | null, "releaseDate": "…" | null, "released": true, "archived": true} (each optional)"#,
        ),
        CommandSpec::new("version.delete", "Delete Version", "Project › Releases", version_delete).params(r#"{"id": 14}"#),
        CommandSpec::new("version.release", "Release Version", "Project › Releases", version_release)
            .params(r#"{"id": 14, "date": "2026-12-01" (default today), "moveUnresolvedTo": 15 (optional: another unreleased version)}"#),
    ]
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn project_key(s: &Session, p: &Value) -> Result<ProjectKey, String> {
    Ok(s.ws.project(params::str(p, "project")?).map_err(err)?.key.clone())
}

fn defs(ws: &Workspace, k: &ProjectKey) -> ProjectFields {
    ws.project_fields(k).cloned().unwrap_or_default()
}

/// A list of strings, capped before anything is copied.
fn options(p: &Value, k: &str) -> Result<Option<Vec<String>>, String> {
    if let Some(Value::Array(a)) = p.get(k)
        && a.len() > MAX_OPTIONS
    {
        return Err(format!("\"{k}\" takes at most {MAX_OPTIONS} options"));
    }
    match p.get(k) {
        Some(Value::String(_)) => Err(format!("\"{k}\" must be a list of strings")),
        _ => params::strings(p, k),
    }
}

fn opt_bool(p: &Value, k: &str) -> Result<Option<bool>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("\"{k}\" must be true or false")),
    }
}

/// `Some(None)` when the key is present as null.
fn nullable_day(p: &Value, k: &str) -> Result<Option<Option<chrono::NaiveDate>>, String> {
    match p.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => parse_day(s).map(|d| Some(Some(d))).map_err(|e| format!("\"{k}\": {e}")),
        Some(_) => Err(format!("\"{k}\" must be a date (YYYY-MM-DD) or null")),
    }
}

fn day(p: &Value, k: &str) -> Result<Option<chrono::NaiveDate>, String> {
    nullable_day(p, k).map(Option::flatten)
}

// ---- fields ----

fn field_list(s: &mut Session, p: &Value) -> CmdResult {
    let k = project_key(s, p)?;
    Ok(Value::Array(defs(&s.ws, &k).fields.iter().map(view::field_def).collect()))
}

fn field_create(s: &mut Session, p: &Value) -> CmdResult {
    let k = project_key(s, p)?;
    let ty =
        params::opt_str(p, "type")?.or(params::opt_str(p, "kind")?).ok_or("missing \"type\" (text, number, date, select, multiSelect, user, url)")?;
    let kind = FieldKind::parse(ty, &options(p, "options")?.unwrap_or_default()).map_err(err)?;
    let required = opt_bool(p, "required")?.unwrap_or(false);
    let desc = params::opt_str(p, "description")?.unwrap_or("");
    let id = s.ws.create_field(k.as_str(), params::str(p, "name")?, kind, required, desc).map_err(err)?;
    Ok(json!({"id": id.0, "key": format!("customfield_{id}")}))
}

fn field_edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let edit = FieldEdit {
        name: params::opt_str(p, "name")?.map(str::to_string),
        options: options(p, "options")?,
        required: opt_bool(p, "required")?,
        description: params::opt_str(p, "description")?.map(str::to_string),
    };
    let (now, me) = (s.now(), s.user.clone());
    s.ws.edit_field(id, edit, &me, now).map_err(err)?;
    let f = s.ws.find_field(id).map(|(_, f)| view::field_def(f)).unwrap_or(Value::Null);
    Ok(f)
}

fn field_delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let (now, me) = (s.now(), s.user.clone());
    let n = s.ws.delete_field(id, &me, now).map_err(err)?;
    Ok(json!({"deleted": id.0, "issues": n}))
}

// ---- components ----

fn component_list(s: &mut Session, p: &Value) -> CmdResult {
    let k = project_key(s, p)?;
    Ok(Value::Array(defs(&s.ws, &k).components.iter().map(|c| view::component(&s.ws, &k, c)).collect()))
}

fn component_create(s: &mut Session, p: &Value) -> CmdResult {
    let k = project_key(s, p)?;
    let lead = params::opt_str(p, "lead")?;
    let desc = params::opt_str(p, "description")?.unwrap_or("");
    let id = s.ws.create_component(k.as_str(), params::str(p, "name")?, lead, desc).map_err(err)?;
    Ok(s.ws.find_component(id).map(|(k, c)| view::component(&s.ws, k, c)).unwrap_or(Value::Null))
}

fn component_edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let lead = match p.get("lead") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(u)) => Some(Some(u.clone())),
        Some(_) => return Err("\"lead\" must be a user id or null".into()),
    };
    let edit = ComponentEdit {
        name: params::opt_str(p, "name")?.map(str::to_string),
        lead,
        description: params::opt_str(p, "description")?.map(str::to_string),
    };
    s.ws.edit_component(id, edit).map_err(err)?;
    Ok(s.ws.find_component(id).map(|(k, c)| view::component(&s.ws, k, c)).unwrap_or(Value::Null))
}

fn component_delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let (now, me) = (s.now(), s.user.clone());
    let n = s.ws.delete_component(id, &me, now).map_err(err)?;
    Ok(json!({"deleted": id.0, "issues": n}))
}

// ---- versions ----

fn version_list(s: &mut Session, p: &Value) -> CmdResult {
    let k = project_key(s, p)?;
    Ok(Value::Array(defs(&s.ws, &k).versions.iter().map(|v| view::version(&s.ws, &k, v)).collect()))
}

fn version_create(s: &mut Session, p: &Value) -> CmdResult {
    let k = project_key(s, p)?;
    let new = NewVersion {
        name: params::str(p, "name")?.to_string(),
        description: params::opt_str(p, "description")?.unwrap_or("").to_string(),
        start: day(p, "startDate")?,
        release_date: day(p, "releaseDate")?,
        released: opt_bool(p, "released")?.unwrap_or(false),
    };
    let id = s.ws.create_version(k.as_str(), new).map_err(err)?;
    Ok(s.ws.find_version(id).map(|(k, v)| view::version(&s.ws, k, v)).unwrap_or(Value::Null))
}

fn version_edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let edit = VersionEdit {
        name: params::opt_str(p, "name")?.map(str::to_string),
        description: params::opt_str(p, "description")?.map(str::to_string),
        start: nullable_day(p, "startDate")?,
        release_date: nullable_day(p, "releaseDate")?,
        released: opt_bool(p, "released")?,
        archived: opt_bool(p, "archived")?,
    };
    s.ws.edit_version(id, edit).map_err(err)?;
    Ok(s.ws.find_version(id).map(|(k, v)| view::version(&s.ws, k, v)).unwrap_or(Value::Null))
}

fn version_delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let (now, me) = (s.now(), s.user.clone());
    let n = s.ws.delete_version(id, &me, now).map_err(err)?;
    Ok(json!({"deleted": id.0, "issues": n}))
}

fn version_release(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let date = day(p, "date")?;
    let to = params::opt_id(p, "moveUnresolvedTo").map_err(|_| "\"moveUnresolvedTo\" must be a version id".to_string())?;
    let (now, me) = (s.now(), s.user.clone());
    let moved = s.ws.release_version(id, date, to, &me, now).map_err(err)?;
    let v = s.ws.find_version(id).map(|(k, v)| view::version(&s.ws, k, v)).unwrap_or(Value::Null);
    Ok(json!({"version": v, "moved": moved}))
}

// ---- values in issue.create / issue.edit ----

/// What `issue.create` / `issue.edit` asked for, resolved against the issue's project.
#[derive(Default)]
pub(crate) struct Requested {
    pub custom: Vec<(Id, Option<FieldValue>)>,
    pub components: Option<Vec<Id>>,
    pub fix_versions: Option<Vec<Id>>,
    pub affects_versions: Option<Vec<Id>>,
}

impl Requested {
    pub fn is_empty(&self) -> bool {
        self.custom.is_empty() && self.components.is_none() && self.fix_versions.is_none() && self.affects_versions.is_none()
    }

    /// First values of a new issue (empty values are skipped).
    pub fn into_initial(self) -> IssueFields {
        IssueFields {
            custom: self.custom.into_iter().filter_map(|(id, v)| v.map(|v| (id, v))).collect(),
            components: self.components.unwrap_or_default(),
            fix_versions: self.fix_versions.unwrap_or_default(),
            affects_versions: self.affects_versions.unwrap_or_default(),
        }
    }

    /// Applies the changes to an existing issue (with history); returns what changed.
    pub fn apply(self, s: &mut Session, key: &str) -> Result<Vec<&'static str>, String> {
        let (now, me) = (s.now(), s.user.clone());
        let mut changed = Vec::new();
        if !self.custom.is_empty() {
            for (id, v) in self.custom {
                s.ws.set_custom(key, id, v, &me, now).map_err(err)?;
            }
            changed.push("custom");
        }
        if let Some(c) = self.components {
            s.ws.set_components(key, &c, &me, now).map_err(err)?;
            changed.push("components");
        }
        if let Some(v) = self.fix_versions {
            s.ws.set_versions(key, VersionList::Fix, &v, &me, now).map_err(err)?;
            changed.push("fixVersions");
        }
        if let Some(v) = self.affects_versions {
            s.ws.set_versions(key, VersionList::Affects, &v, &me, now).map_err(err)?;
            changed.push("affectsVersions");
        }
        Ok(changed)
    }
}

/// Reads `custom`, `components`, `fixVersions` and `affectsVersions` (alias `versions`).
pub(crate) fn requested(ws: &Workspace, project: &ProjectKey, p: &Value) -> Result<Requested, String> {
    let d = defs(ws, project);
    let mut r = Requested::default();
    match p.get("custom") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            if m.len() > MAX_FIELDS {
                return Err(format!("\"custom\" takes at most {MAX_FIELDS} fields"));
            }
            for (name, v) in m {
                let def = d.find_field(name).ok_or_else(|| format!("project {project} has no field '{}'", short(name)))?;
                r.custom.push((def.id, value_of(ws, def, v)?));
            }
        }
        Some(_) => return Err("\"custom\" must be an object of field name (or id) → value".into()),
    }
    r.components = refs(p, "components", "component", |s| d.find_component(s).map(|c| c.id), project)?;
    r.fix_versions = refs(p, "fixVersions", "version", |s| d.find_version(s).map(|v| v.id), project)?;
    r.affects_versions = match p.get("affectsVersions") {
        Some(_) => refs(p, "affectsVersions", "version", |s| d.find_version(s).map(|v| v.id), project)?,
        None => refs(p, "versions", "version", |s| d.find_version(s).map(|v| v.id), project)?,
    };
    Ok(r)
}

fn short(s: &str) -> String {
    s.chars().take(60).collect()
}

/// A list of components or versions: names or ids (strings or numbers), or REST objects
/// (`{"id": "12"}`, `{"name": "Docs"}`).
fn refs(p: &Value, k: &str, what: &str, find: impl Fn(&str) -> Option<Id>, project: &ProjectKey) -> Result<Option<Vec<Id>>, String> {
    let items: Vec<&Value> = match p.get(k) {
        None => return Ok(None),
        Some(Value::Null) => return Ok(Some(vec![])),
        Some(Value::Array(a)) if a.len() > MAX_PER_ISSUE => return Err(format!("\"{k}\" takes at most {MAX_PER_ISSUE} values")),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(v @ (Value::String(_) | Value::Number(_) | Value::Object(_))) => vec![v],
        Some(_) => return Err(format!("\"{k}\" must be a list of {what} names or ids")),
    };
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let text = match it {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Object(o) => match (o.get("id"), o.get("name")) {
                (Some(Value::String(s)), _) => s.clone(),
                (Some(Value::Number(n)), _) => n.to_string(),
                (_, Some(Value::String(s))) => s.clone(),
                _ => return Err(format!("\"{k}\": an object needs \"id\" or \"name\"")),
            },
            _ => return Err(format!("\"{k}\" must be a list of {what} names or ids")),
        };
        if text.trim().is_empty() {
            continue;
        }
        out.push(find(&text).ok_or_else(|| format!("project {project} has no {what} '{}'", short(&text)))?);
    }
    Ok(Some(out))
}

/// A JSON value for field `def`: `null` clears; strings are parsed for the kind; numbers for
/// number fields; lists for multi-selects; REST shapes (`{"value": …}`, `{"accountId": …}`)
/// are accepted too.
pub(crate) fn value_of(ws: &Workspace, def: &FieldDef, v: &Value) -> Result<Option<FieldValue>, String> {
    let name = short(&def.name);
    let text_of = |v: &Value| -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => ["value", "accountId", "id", "name"].iter().find_map(|k| o.get(*k).and_then(Value::as_str)).map(str::to_string),
            _ => None,
        }
    };
    let value = match (&def.kind, v) {
        (_, Value::Null) => None,
        (FieldKind::Number, Value::Number(n)) => Some(FieldValue::Number(n.as_f64().ok_or_else(|| format!("{name}: not a number"))?)),
        (FieldKind::MultiSelect { .. }, Value::Array(a)) => {
            if a.len() > MAX_PER_ISSUE {
                return Err(format!("{name}: at most {MAX_PER_ISSUE} values"));
            }
            let vals: Vec<String> =
                a.iter().map(|x| text_of(x).ok_or_else(|| format!("{name}: values are option names"))).collect::<Result<_, _>>()?;
            if vals.is_empty() { None } else { Some(FieldValue::MultiSelect(vals)) }
        }
        (FieldKind::Text | FieldKind::Url, Value::String(s)) => FieldValue::from_text(&def.kind, s).map_err(err)?,
        (kind, other) => {
            let s = text_of(other).ok_or_else(|| format!("{name}: a {} field takes {}", kind.name(), expects(kind)))?;
            FieldValue::from_text(kind, &s).map_err(|e| format!("{name}: {e}"))?
        }
    };
    value.map(|v| ws.check_value(def, v).map_err(err)).transpose()
}

fn expects(k: &FieldKind) -> &'static str {
    match k {
        FieldKind::Text | FieldKind::Url => "a string",
        FieldKind::Number => "a number",
        FieldKind::Date => "a date (YYYY-MM-DD)",
        FieldKind::Select { .. } => "an option name",
        FieldKind::MultiSelect { .. } => "a list of option names",
        FieldKind::User => "a user id",
    }
}
