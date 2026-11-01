//! M2.4: components, versions and custom fields from an export (`docs/import.md`).
//!
//! Components and versions are created by name in the issue's project. A custom field column
//! (`Custom field (Customer)`) becomes a field of that name unless the project has one already
//! (then its kind is kept). The kind is guessed from every value the export holds for it: all
//! numbers → number; all dates → date; several values on one issue (a repeated column) →
//! multi-select over the values seen (at most [`MAX_OPTIONS`]); otherwise text. Values that
//! don't fit are warnings, never failures.

use std::collections::BTreeMap;

use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::{FieldKind, FieldValue, IssueFields, MAX_OPTIONS, MAX_PER_ISSUE, NewVersion, Workspace};

use crate::jira::{RawIssue, parse_date};
use crate::{ImportReport, clip};

/// Custom fields the importer reads elsewhere, or that are Jira internals.
const SKIP: &[&str] =
    &["epic link", "story points", "story point estimate", "sprint", "rank", "epic name", "epic status", "epic colour", "epic color"];

/// Values seen for one field of one project.
#[derive(Default)]
struct Seen {
    values: Vec<String>,
    multi: bool,
}

fn is_number(s: &str) -> bool {
    s.trim().replace(',', "").parse::<f64>().is_ok_and(f64::is_finite)
}

fn guess(seen: &Seen) -> FieldKind {
    if seen.values.iter().all(|v| is_number(v)) {
        return FieldKind::Number;
    }
    if seen.values.iter().all(|v| parse_date(v).is_some()) {
        return FieldKind::Date;
    }
    if seen.multi {
        let mut options: Vec<String> = Vec::new();
        for v in &seen.values {
            let v = clip(v.trim(), trackcraft_model::MAX_TITLE);
            if !v.is_empty() && !options.iter().any(|o| o.eq_ignore_ascii_case(&v)) {
                options.push(v);
            }
        }
        if !options.is_empty() && options.len() <= MAX_OPTIONS && options.iter().all(|o| !o.chars().any(char::is_control)) {
            return FieldKind::MultiSelect { options };
        }
    }
    FieldKind::Text
}

/// Creates the components, versions and custom fields the issues use.
pub fn ensure<'a>(ws: &mut Workspace, items: impl Iterator<Item = (&'a ProjectKey, &'a RawIssue)>, report: &mut ImportReport) {
    let mut fields: BTreeMap<(ProjectKey, String), Seen> = BTreeMap::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for (pk, r) in items {
        for c in r.components.iter().take(MAX_PER_ISSUE) {
            if ws.project_fields(pk).and_then(|d| d.find_component(c)).is_none()
                && let Err(e) = ws.create_component(pk.as_str(), c, None, "")
            {
                report.warn(format!("{}: component '{}': {e}", r.at, clip(c, 40)));
            }
        }
        for v in r.fix_versions.iter().chain(&r.affects_versions).take(MAX_PER_ISSUE.saturating_mul(2)) {
            if ws.project_fields(pk).and_then(|d| d.find_version(v)).is_none()
                && let Err(e) = ws.create_version(pk.as_str(), NewVersion { name: v.clone(), ..NewVersion::default() })
            {
                report.warn(format!("{}: version '{}': {e}", r.at, clip(v, 40)));
            }
        }
        let mut per_issue: BTreeMap<String, usize> = BTreeMap::new();
        for (name, value) in &r.custom {
            let lower = name.trim().to_lowercase();
            if SKIP.contains(&lower.as_str()) {
                continue;
            }
            names.entry(lower.clone()).or_insert_with(|| name.trim().to_string());
            let seen = fields.entry((pk.clone(), lower.clone())).or_default();
            let n = per_issue.entry(lower).or_default();
            *n += 1;
            seen.multi |= *n > 1;
            if seen.values.len() < 50_000 {
                seen.values.push(value.clone());
            }
        }
    }
    for ((pk, lower), seen) in &fields {
        let name = names.get(lower).cloned().unwrap_or_else(|| lower.clone());
        if ws.project_fields(pk).and_then(|d| d.find_field(&name)).is_some() {
            continue;
        }
        if let Err(e) = ws.create_field(pk.as_str(), &name, guess(seen), false, "") {
            report.warn(format!("custom field '{}' in {pk}: {e}; its values were not imported", clip(&name, 40)));
        }
    }
}

/// Sets the issue's components, versions and custom values (no history: first values). Values
/// that don't fit their field are left out with a warning.
pub fn set(ws: &mut Workspace, key: &str, r: &RawIssue, report: &mut ImportReport) {
    let Ok(pk) = ws.issue(key).map(|i| i.key.project.clone()) else { return };
    let Some(d) = ws.project_fields(&pk).cloned() else { return };
    let ids = |list: &[String], find: &dyn Fn(&str) -> Option<Id>| -> Vec<Id> {
        let mut out: Vec<Id> = list.iter().filter_map(|n| find(n)).collect();
        out.dedup();
        out.truncate(MAX_PER_ISSUE);
        out
    };
    let mut values = IssueFields {
        components: ids(&r.components, &|n| d.find_component(n).map(|c| c.id)),
        fix_versions: ids(&r.fix_versions, &|n| d.find_version(n).map(|v| v.id)),
        affects_versions: ids(&r.affects_versions, &|n| d.find_version(n).map(|v| v.id)),
        custom: vec![],
    };
    // Group repeated columns; a single-valued field keeps the first value.
    let mut grouped: BTreeMap<Id, Vec<String>> = BTreeMap::new();
    for (name, value) in &r.custom {
        if SKIP.contains(&name.trim().to_lowercase().as_str()) {
            continue;
        }
        if let Some(f) = d.find_field(name) {
            grouped.entry(f.id).or_default().push(value.clone());
        }
    }
    for (id, vals) in grouped {
        let Some(def) = d.field(id) else { continue };
        let value = match &def.kind {
            // Each repeated column is one option (names may hold commas).
            FieldKind::MultiSelect { .. } => {
                let mut list: Vec<String> = vals.iter().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
                list.truncate(MAX_PER_ISSUE);
                Ok((!list.is_empty()).then_some(FieldValue::MultiSelect(list)))
            }
            FieldKind::Date => {
                let day = vals.first().and_then(|v| parse_date(v)).map(|t| t.date_naive().format("%Y-%m-%d").to_string());
                FieldValue::from_text(&def.kind, &day.unwrap_or_else(|| vals.first().cloned().unwrap_or_default()))
            }
            _ => FieldValue::from_text(&def.kind, vals.first().map_or("", String::as_str)),
        };
        let parsed = value.and_then(|v| v.map(|v| ws.check_value(def, v)).transpose());
        match parsed {
            Ok(Some(v)) => values.custom.push((id, v)),
            Ok(None) => {}
            Err(e) => report.warn(format!("{}: {key}: {e}", r.at)),
        }
    }
    if values.is_empty() {
        return;
    }
    if let Err(e) = ws.init_issue_fields(key, values) {
        report.warn(format!("{}: {key}: {e}", r.at));
    }
}
