//! Jira's "export issues to CSV" file (the column names Atlassian documents for the export;
//! several columns repeat — `Labels`, `Sprint`, `Comment`, issue links — one per value).

use trackcraft_model::{ProjectKind, StatusCategory, Workspace};

use crate::jira::{RawComment, RawIssue, RawLink, RawSprint, apply, parse_date};
use crate::people::{Person, clean};
use crate::{Actor, ImportError, ImportReport, MAX_FIELD_BYTES, Result, check_size, clip, clip_bytes, transact, wiki};

/// Imports a Jira CSV export (the text of the file).
pub fn jira_csv(ws: &mut Workspace, text: &str, actor: Actor<'_>) -> Result<ImportReport> {
    check_size(text.len(), "the CSV file")?;
    transact(ws, actor, |w, report| {
        let raws = read(text, report)?;
        apply(w, raws, actor, report);
        Ok(())
    })
}

#[derive(Default)]
struct Columns {
    summary: Option<usize>,
    key: Option<usize>,
    id: Option<usize>,
    issue_type: Option<usize>,
    status: Option<usize>,
    status_category: Option<usize>,
    project_name: Option<usize>,
    project_type: Option<usize>,
    priority: Option<usize>,
    assignee: Option<usize>,
    assignee_id: Option<usize>,
    reporter: Option<usize>,
    reporter_id: Option<usize>,
    created: Option<usize>,
    updated: Option<usize>,
    resolved: Option<usize>,
    description: Option<usize>,
    labels: Vec<usize>,
    sprints: Vec<usize>,
    parent: Vec<usize>,
    epic_link: Option<usize>,
    points: Vec<usize>,
    comments: Vec<usize>,
    /// (column, link type name, outward)
    links: Vec<(usize, String, bool)>,
}

fn columns(header: &[String]) -> Columns {
    let mut c = Columns::default();
    for (i, h) in header.iter().enumerate() {
        let l = h.trim().to_lowercase();
        let one = |slot: &mut Option<usize>| {
            if slot.is_none() {
                *slot = Some(i);
            }
        };
        match l.as_str() {
            "summary" => one(&mut c.summary),
            "issue key" | "key" => one(&mut c.key),
            "issue id" => one(&mut c.id),
            "issue type" => one(&mut c.issue_type),
            "status" => one(&mut c.status),
            "status category" => one(&mut c.status_category),
            "project name" => one(&mut c.project_name),
            "project type" => one(&mut c.project_type),
            "priority" => one(&mut c.priority),
            "assignee" => one(&mut c.assignee),
            "assignee id" => one(&mut c.assignee_id),
            "reporter" => one(&mut c.reporter),
            "reporter id" => one(&mut c.reporter_id),
            "created" => one(&mut c.created),
            "updated" => one(&mut c.updated),
            "resolved" => one(&mut c.resolved),
            "description" => one(&mut c.description),
            "labels" => c.labels.push(i),
            "sprint" => c.sprints.push(i),
            "parent" | "parent id" | "parent key" | "parent issue key" => c.parent.push(i),
            "custom field (epic link)" => one(&mut c.epic_link),
            "custom field (story points)" | "custom field (story point estimate)" | "story points" | "story point estimate" => c.points.push(i),
            "comment" | "comments" => c.comments.push(i),
            _ => {
                for (prefix, outward) in [("inward issue link (", false), ("outward issue link (", true)] {
                    if let Some(kind) = l.strip_prefix(prefix).and_then(|r| r.strip_suffix(')')) {
                        let original = h.trim().get(prefix.len()..h.trim().len().saturating_sub(1)).unwrap_or(kind);
                        c.links.push((i, original.to_string(), outward));
                    }
                }
            }
        }
    }
    c
}

fn category(s: &str) -> Option<StatusCategory> {
    match s.trim().to_lowercase().as_str() {
        "to do" | "todo" | "new" => Some(StatusCategory::ToDo),
        "in progress" | "indeterminate" => Some(StatusCategory::InProgress),
        "done" | "complete" => Some(StatusCategory::Done),
        _ => None,
    }
}

/// Account ids look opaque (`557058:…`, 24 hex digits); usernames don't.
fn author(s: &str) -> Person {
    let s = s.trim();
    let opaque = s.contains(':') || (s.len() >= 20 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if opaque { Person { account: clean(s), ..Person::default() } } else { Person { account: clean(s), name: clean(s), ..Person::default() } }
}

/// A `Comment` cell: `date;author;body` (the body may hold more `;`).
fn comment(cell: &str, at: &str, report: &mut ImportReport) -> Option<RawComment> {
    let cell = cell.trim();
    if cell.is_empty() {
        return None;
    }
    let mut parts = cell.splitn(3, ';');
    let (d, a, b) = (parts.next().unwrap_or_default(), parts.next(), parts.next());
    match (parse_date(d), a, b) {
        (Some(date), Some(a), Some(b)) => Some(RawComment { author: author(a), at: Some(date), body: wiki::to_doc(clip_bytes(b, MAX_FIELD_BYTES)) }),
        _ => {
            report.warn(format!("{at}: a comment isn't 'date;author;body'; imported as written, by the importing user"));
            Some(RawComment { author: Person::default(), at: None, body: wiki::to_doc(clip_bytes(cell, MAX_FIELD_BYTES)) })
        }
    }
}

fn read(text: &str, report: &mut ImportReport) -> Result<Vec<RawIssue>> {
    let table = crate::csv::parse(text, report)?;
    let c = columns(&table.header);
    if c.summary.is_none() || c.key.is_none() {
        return Err(ImportError::Malformed("a Jira CSV export needs 'Summary' and 'Issue key' columns".into()));
    }
    let mut out = Vec::with_capacity(table.rows.len());
    for (line, row) in &table.rows {
        let at = format!("line {line}");
        let cell = |i: Option<usize>| i.and_then(|i| row.get(i)).map(|s| s.trim()).unwrap_or_default();
        let cells = |is: &[usize]| -> Vec<String> {
            is.iter().filter_map(|&i| row.get(i)).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_string).collect()
        };
        let date = |i: Option<usize>, what: &str, report: &mut ImportReport| {
            let s = cell(i);
            if s.is_empty() {
                return None;
            }
            let d = parse_date(s);
            if d.is_none() {
                report.warn(format!("{at}: {what} date '{}' not understood", clip(s, 40)));
            }
            d
        };
        let mut r = RawIssue {
            at: at.clone(),
            key: cell(c.key).to_string(),
            id: Some(cell(c.id).to_string()).filter(|s| !s.is_empty()),
            project_name: clean(cell(c.project_name)),
            project_kind: match cell(c.project_type).to_lowercase().as_str() {
                "business" => Some(ProjectKind::Business),
                "software" => Some(ProjectKind::Software),
                _ => None,
            },
            summary: cell(c.summary).to_string(),
            issue_type: clean(cell(c.issue_type)),
            status: clean(cell(c.status)),
            status_category: category(cell(c.status_category)),
            priority: clean(cell(c.priority)),
            assignee: Person { account: clean(cell(c.assignee_id)), name: clean(cell(c.assignee)), email: None },
            reporter: Person { account: clean(cell(c.reporter_id)), name: clean(cell(c.reporter)), email: None },
            created: date(c.created, "created", report),
            updated: date(c.updated, "updated", report),
            resolved: date(c.resolved, "resolved", report),
            description: wiki::to_doc(clip_bytes(cell(c.description), MAX_FIELD_BYTES)),
            labels: cells(&c.labels),
            sprints: cells(&c.sprints).into_iter().map(|name| RawSprint { name, ..RawSprint::default() }).collect(),
            parent: cells(&c.parent).into_iter().next().or_else(|| clean(cell(c.epic_link))),
            ..RawIssue::default()
        };
        for p in cells(&c.points) {
            match p.replace(',', ".").parse::<f64>() {
                Ok(v) => r.story_points = Some(v),
                Err(_) => report.warn(format!("{at}: story points '{}' is not a number", clip(&p, 20))),
            }
        }
        for &i in &c.comments {
            if let Some(cm) = row.get(i).and_then(|s| comment(s, &at, report)) {
                r.comments.push(cm);
            }
        }
        for (i, kind, outward) in &c.links {
            for other in row.get(*i).map(String::as_str).unwrap_or_default().split([',', ' ']).filter(|s| !s.trim().is_empty()) {
                r.links.push(RawLink { kind: kind.clone(), outward: *outward, other: other.trim().to_string() });
            }
        }
        out.push(r);
    }
    Ok(out)
}
