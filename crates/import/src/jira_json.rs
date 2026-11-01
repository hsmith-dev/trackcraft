//! Jira Cloud REST API v3 JSON: the response of `GET/POST /rest/api/3/search/jql` (one page
//! `{"issues":[…]}`), an array of such pages, an array of issues, or one issue.

use serde_json::Value;
use trackcraft_markup::Doc;
use trackcraft_model::{ProjectKind, SprintState, StatusCategory, Workspace};

use crate::jira::{RawComment, RawIssue, RawLink, RawSprint, apply, parse_date};
use crate::people::{Person, clean};
use crate::{Actor, ImportError, ImportReport, MAX_FIELD_BYTES, MAX_ISSUES, MAX_PER_ISSUE, Result, check_size, clip_bytes, transact, wiki};

/// Imports Jira REST v3 JSON (the text of the file).
pub fn jira_json(ws: &mut Workspace, text: &str, actor: Actor<'_>) -> Result<ImportReport> {
    check_size(text.len(), "the JSON file")?;
    // serde_json refuses nesting deeper than 128 levels, so hostile depth is an error, not a crash.
    let v: Value = serde_json::from_str(text).map_err(|e| ImportError::Malformed(format!("not JSON: {e}")))?;
    transact(ws, actor, |w, report| {
        let raws = read(&v, report)?;
        apply(w, raws, actor, report);
        Ok(())
    })
}

/// (issue, `names` map of its page) pairs.
fn issues(v: &Value) -> Result<Vec<(&Value, Option<&Value>)>> {
    fn page<'v>(p: &'v Value, out: &mut Vec<(&'v Value, Option<&'v Value>)>) -> bool {
        let Some(list) = p.get("issues").and_then(Value::as_array) else { return false };
        out.extend(list.iter().map(|i| (i, p.get("names"))));
        true
    }
    let mut out = Vec::new();
    match v {
        Value::Object(_) if page(v, &mut out) => {}
        Value::Object(_) if v.get("key").is_some() && v.get("fields").is_some() => out.push((v, None)),
        Value::Array(items) => {
            for it in items {
                if !page(it, &mut out) && it.get("key").is_some() {
                    out.push((it, None));
                }
            }
        }
        _ => return Err(ImportError::Malformed("expected a search/jql page ({\"issues\":[…]}), an array of pages, or issues".into())),
    }
    Ok(out)
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).and_then(clean)
}

fn person(v: Option<&Value>) -> Person {
    let Some(v) = v.filter(|v| v.is_object()) else { return Person::default() };
    Person {
        account: s(v.get("accountId")).or_else(|| s(v.get("name"))).or_else(|| s(v.get("key"))),
        name: s(v.get("displayName")),
        email: s(v.get("emailAddress")),
    }
}

/// A rich-text field: ADF (v3) or wiki markup (v2 strings).
fn rich(v: Option<&Value>, at: &str, report: &mut ImportReport) -> Doc {
    match v {
        Some(Value::String(t)) => wiki::to_doc(clip_bytes(t, MAX_FIELD_BYTES)),
        Some(o @ Value::Object(_)) => Doc::from_adf(o).unwrap_or_else(|e| {
            report.warn(format!("{at}: rich text left out: {e}"));
            Doc::default()
        }),
        _ => Doc::default(),
    }
}

fn date(v: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    v.and_then(Value::as_str).and_then(parse_date)
}

/// Sprint objects (Jira Software's sprint field: `[{"id","name","state","startDate",…}]`).
fn sprint_list(v: &Value) -> Option<Vec<RawSprint>> {
    let arr = v.as_array()?;
    let mut out = Vec::new();
    for sp in arr.iter().take(MAX_PER_ISSUE) {
        let state = match sp.get("state").and_then(Value::as_str)?.to_ascii_lowercase().as_str() {
            "active" => SprintState::Active,
            "closed" => SprintState::Closed,
            "future" => SprintState::Future,
            _ => return None,
        };
        out.push(RawSprint {
            name: s(sp.get("name"))?,
            state: Some(state),
            start: date(sp.get("startDate")),
            end: date(sp.get("endDate")),
            completed: date(sp.get("completeDate")),
        });
    }
    (!out.is_empty()).then_some(out)
}

fn read(v: &Value, report: &mut ImportReport) -> Result<Vec<RawIssue>> {
    let list = issues(v)?;
    if list.len() > MAX_ISSUES {
        report.warn(format!("more than {MAX_ISSUES} issues: the rest were not imported"));
    }
    let mut out = Vec::new();
    for (n, (issue, names)) in list.into_iter().take(MAX_ISSUES).enumerate() {
        let at = format!("issues[{n}]");
        let Some(key) = issue.get("key").and_then(Value::as_str) else {
            report.warn(format!("{at}: no key; skipped"));
            continue;
        };
        let Some(f) = issue.get("fields").filter(|f| f.is_object()) else {
            report.warn(format!("{at} ({}): no fields object; skipped", crate::clip(key, 40)));
            continue;
        };
        let field_named = |want: &[&str]| -> Option<&Value> {
            let names = names?.as_object()?;
            names
                .iter()
                .find(|(_, name)| name.as_str().is_some_and(|nm| want.iter().any(|w| nm.eq_ignore_ascii_case(w))))
                .and_then(|(id, _)| f.get(id))
        };
        let status = f.get("status");
        let category = status.and_then(|s| s.get("statusCategory")).and_then(|c| c.get("key")).and_then(Value::as_str).and_then(|k| match k {
            "new" => Some(StatusCategory::ToDo),
            "indeterminate" => Some(StatusCategory::InProgress),
            "done" => Some(StatusCategory::Done),
            _ => None,
        });
        let mut r = RawIssue {
            at: format!("{at} ({})", crate::clip(key, 40)),
            key: key.to_string(),
            id: issue.get("id").and_then(|i| i.as_str().map(str::to_string).or_else(|| i.as_u64().map(|n| n.to_string()))),
            project_name: s(f.get("project").and_then(|p| p.get("name"))),
            project_kind: match f.get("project").and_then(|p| p.get("projectTypeKey")).and_then(Value::as_str) {
                Some("business") => Some(ProjectKind::Business),
                Some("software") => Some(ProjectKind::Software),
                _ => None,
            },
            summary: f.get("summary").and_then(Value::as_str).unwrap_or_default().to_string(),
            issue_type: s(f.get("issuetype").and_then(|t| t.get("name"))),
            subtask: f.get("issuetype").and_then(|t| t.get("subtask")).and_then(Value::as_bool),
            status: s(status.and_then(|s| s.get("name"))),
            status_category: category,
            priority: s(f.get("priority").and_then(|p| p.get("name"))),
            assignee: person(f.get("assignee")),
            reporter: person(f.get("reporter")),
            created: date(f.get("created")),
            updated: date(f.get("updated")),
            resolved: date(f.get("resolutiondate")),
            description: rich(f.get("description"), &at, report),
            labels: f.get("labels").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
            parent: f.get("parent").and_then(|p| s(p.get("key")).or_else(|| s(p.get("id")))),
            story_points: field_named(&["Story Points", "Story point estimate"]).and_then(Value::as_f64),
            ..RawIssue::default()
        };
        // M2.4: components and versions by name.
        let named = |k: &str| -> Vec<String> {
            f.get(k)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| x.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        };
        r.components = named("components");
        r.fix_versions = named("fixVersions");
        r.affects_versions = named("versions");
        r.sprints = field_named(&["Sprint"])
            .and_then(sprint_list)
            .or_else(|| f.as_object().and_then(|o| o.iter().filter(|(k, _)| k.starts_with("customfield_")).find_map(|(_, v)| sprint_list(v))))
            .unwrap_or_default();
        let comments = f.get("comment").and_then(|c| c.get("comments")).and_then(Value::as_array);
        for c in comments.into_iter().flatten().take(MAX_PER_ISSUE) {
            r.comments.push(RawComment { author: person(c.get("author")), at: date(c.get("created")), body: rich(c.get("body"), &at, report) });
        }
        for l in f.get("issuelinks").and_then(Value::as_array).into_iter().flatten().take(MAX_PER_ISSUE) {
            let kind = l.get("type").and_then(|t| t.get("name")).and_then(Value::as_str).unwrap_or("Relates").to_string();
            if let Some(k) = l.get("outwardIssue").and_then(|i| i.get("key")).and_then(Value::as_str) {
                r.links.push(RawLink { kind: kind.clone(), outward: true, other: k.to_string() });
            }
            if let Some(k) = l.get("inwardIssue").and_then(|i| i.get("key")).and_then(Value::as_str) {
                r.links.push(RawLink { kind, outward: false, other: k.to_string() });
            }
        }
        out.push(r);
    }
    Ok(out)
}
