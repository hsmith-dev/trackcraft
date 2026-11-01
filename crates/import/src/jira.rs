//! What the two Jira readers (CSV, JSON) produce, and how it lands in a workspace.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use trackcraft_core::{IssueKey, ProjectKey};
use trackcraft_markup::Doc;
use trackcraft_model::{LinkKind, MAX_LABELS, MAX_TITLE, NewIssue, PRIORITIES, ProjectKind, SprintState, StatusCategory, Timestamp, Workspace};

use crate::people::{People, Person};
use crate::{Actor, ImportReport, MAX_ISSUES, MAX_PER_ISSUE, clip};

#[derive(Debug, Clone, Default)]
pub struct RawSprint {
    pub name: String,
    pub state: Option<SprintState>,
    pub start: Option<Timestamp>,
    pub end: Option<Timestamp>,
    pub completed: Option<Timestamp>,
}

#[derive(Debug, Clone, Default)]
pub struct RawComment {
    pub author: Person,
    pub at: Option<Timestamp>,
    pub body: Doc,
}

#[derive(Debug, Clone)]
pub struct RawLink {
    /// The link type's name (`Blocks`, `Relates`, `Duplicate`, `Cloners`…).
    pub kind: String,
    /// `true`: this issue → other ("blocks"); `false`: other → this issue ("is blocked by").
    pub outward: bool,
    pub other: String,
}

#[derive(Debug, Clone, Default)]
pub struct RawIssue {
    /// Where it came from, for warnings ("row 7", "issues[3]").
    pub at: String,
    pub key: String,
    pub id: Option<String>,
    pub project_name: Option<String>,
    pub project_kind: Option<ProjectKind>,
    pub summary: String,
    pub issue_type: Option<String>,
    pub subtask: Option<bool>,
    pub status: Option<String>,
    pub status_category: Option<StatusCategory>,
    pub priority: Option<String>,
    pub assignee: Person,
    pub reporter: Person,
    pub created: Option<Timestamp>,
    pub updated: Option<Timestamp>,
    pub resolved: Option<Timestamp>,
    pub description: Doc,
    pub labels: Vec<String>,
    pub sprints: Vec<RawSprint>,
    /// The parent's key, or its numeric issue id.
    pub parent: Option<String>,
    pub story_points: Option<f64>,
    pub comments: Vec<RawComment>,
    pub links: Vec<RawLink>,
}

/// Parses the date formats Jira exports use: ISO 8601 / RFC 3339 (REST: `2024-03-21T15:07:00.000+0000`),
/// the CSV default `21/Mar/24 3:07 PM`, and `2024-03-21 15:07`. Dates without a zone are UTC.
pub fn parse_date(s: &str) -> Option<Timestamp> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
    let s = s.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc));
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%dT%H:%M:%S%z", "%Y-%m-%d %H:%M:%S%.f%z"] {
        if let Ok(d) = DateTime::parse_from_str(s, f) {
            return Some(d.with_timezone(&Utc));
        }
    }
    for f in [
        "%d/%b/%y %I:%M %p",
        "%d/%b/%Y %I:%M %p",
        "%d/%b/%y %H:%M",
        "%d/%b/%Y %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%b %d, %Y %I:%M %p",
    ] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Some(d.and_utc());
        }
    }
    for f in ["%Y-%m-%d", "%d/%b/%y", "%d/%b/%Y", "%b %d, %Y", "%d %b %Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, f) {
            return d.and_hms_opt(0, 0, 0).map(|d| d.and_utc());
        }
    }
    None
}

/// The status category a status name suggests (`docs/import.md`): Done words, then
/// in-progress words, else To Do.
pub fn guess_category(name: &str) -> StatusCategory {
    let n = name.to_lowercase();
    const DONE: [&str; 13] =
        ["done", "closed", "resolved", "complete", "fixed", "released", "shipped", "cancel", "won't", "wont", "declined", "rejected", "duplicate"];
    const DOING: [&str; 12] =
        ["progress", "review", "doing", "develop", "test", "qa", "verif", "implement", "active", "started", "working", "blocked"];
    if DONE.iter().any(|w| n.contains(w)) {
        StatusCategory::Done
    } else if DOING.iter().any(|w| n.contains(w)) {
        StatusCategory::InProgress
    } else {
        StatusCategory::ToDo
    }
}

/// Our priority for a Jira priority name (`docs/import.md`), `None` when unknown.
pub fn map_priority(p: &str) -> Option<&'static str> {
    let l = p.trim().to_lowercase();
    if let Some(c) = PRIORITIES.iter().find(|q| q.eq_ignore_ascii_case(&l)) {
        return Some(c);
    }
    Some(match l.as_str() {
        "blocker" | "critical" | "urgent" | "p0" | "p1 - critical" => "Highest",
        "major" | "p1" | "p2 - high" => "High",
        "normal" | "p2" | "p3 - medium" | "" => "Medium",
        "minor" | "p3" | "p4 - low" => "Low",
        "trivial" | "p4" | "p5" => "Lowest",
        _ => return None,
    })
}

/// Our link kind for a Jira link type name.
pub fn map_link(kind: &str) -> Option<LinkKind> {
    let l = kind.trim().to_lowercase();
    Some(match l.as_str() {
        "blocks" | "block" | "blocker" => LinkKind::Blocks,
        "relates" | "relate" | "related" | "relates to" => LinkKind::Relates,
        "duplicate" | "duplicates" => LinkKind::Duplicates,
        "cloners" | "clones" | "cloner" | "clone" => LinkKind::Clones,
        _ => return None,
    })
}

fn summary(s: &str) -> Option<String> {
    let s: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then(|| clip(&s, MAX_TITLE))
}

fn sanitize_labels(raw: &[String], at: &str, report: &mut ImportReport) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in raw.iter().flat_map(|l| l.split_whitespace()) {
        if l.chars().count() > MAX_TITLE {
            report.warn(format!("{at}: a label longer than {MAX_TITLE} characters was left out"));
        } else if !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
    }
    if out.len() > MAX_LABELS {
        report.warn(format!("{at}: only the first {MAX_LABELS} labels were kept"));
        out.truncate(MAX_LABELS);
    }
    out
}

/// A sprint's trailing number ("Team Sprint 12" → 12), for ordering open sprints.
fn sprint_number(name: &str) -> u64 {
    let digits: String = name.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().unwrap_or(u64::MAX)
}

struct Prepared {
    raw: RawIssue,
    key: IssueKey,
    parent: Option<IssueKey>,
}

/// Lands parsed issues in the workspace (see `docs/import.md` for every rule applied here).
pub fn apply(ws: &mut Workspace, raws: Vec<RawIssue>, actor: Actor<'_>, report: &mut ImportReport) {
    let mut people = People::default();
    for r in &raws {
        people.learn(&r.assignee);
        people.learn(&r.reporter);
        for c in &r.comments {
            people.learn(&c.author);
        }
    }
    let (mut items, by_id) = prepare(ws, raws, report);
    ensure_projects(ws, &items, report);
    ensure_kinds(ws, &mut items, report);
    let order = creation_order(ws, &mut items, &by_id, report);
    let mut created: Vec<usize> = Vec::new();
    for n in order {
        if let Some(it) = items.get(n)
            && create(ws, it, &mut people, actor, report)
        {
            created.push(n);
        }
    }
    links(ws, &items, &created, actor, report);
    sprints(ws, &items, &created, actor, report);
    // Final state: status, dates (after comments and links, which touch `updated`).
    for &n in &created {
        let Some(it) = items.get(n) else { continue };
        let r = &it.raw;
        let key = it.key.to_string();
        let updated = r.updated.or(r.created).unwrap_or(actor.now);
        if let Err(e) = ws.set_imported_state(&key, r.status.as_deref(), Some(updated), Some(r.resolved)) {
            report.warn(format!("{}: {key}: {e}", r.at));
        }
    }
}

/// Keys: valid, unique in the file, not already in the workspace. Also maps issue ids to keys.
fn prepare(ws: &Workspace, raws: Vec<RawIssue>, report: &mut ImportReport) -> (Vec<Prepared>, HashMap<String, IssueKey>) {
    let mut seen: BTreeSet<IssueKey> = BTreeSet::new();
    let mut by_id: HashMap<String, IssueKey> = HashMap::new();
    let mut items: Vec<Prepared> = Vec::new();
    for r in raws.into_iter().take(MAX_ISSUES) {
        let Ok(key) = IssueKey::parse(&r.key) else {
            report.warn(format!("{}: '{}' is not an issue key; skipped", r.at, clip(&r.key, 40)));
            continue;
        };
        if ws.issues.contains_key(&key) {
            report.warn(format!("{}: {key} already exists in the workspace; skipped", r.at));
            continue;
        }
        if !seen.insert(key.clone()) {
            report.warn(format!("{}: {key} appears twice; the second was skipped", r.at));
            continue;
        }
        if let Some(id) = &r.id {
            by_id.insert(id.clone(), key.clone());
        }
        items.push(Prepared { raw: r, key, parent: None });
    }
    (items, by_id)
}

fn ensure_projects(ws: &mut Workspace, items: &[Prepared], report: &mut ImportReport) {
    for it in items {
        let pk = it.key.project.clone();
        if !ws.projects.contains_key(&pk) {
            let name = it.raw.project_name.as_deref().and_then(summary).unwrap_or_else(|| pk.to_string());
            match ws.create_project(pk.as_str(), &name, it.raw.project_kind.unwrap_or(ProjectKind::Software), None) {
                Ok(_) => report.projects.push(pk.to_string()),
                Err(e) => report.warn(format!("project {pk}: {e}")),
            }
        }
    }
}

/// Issue types and statuses the projects lack are added; priorities are mapped.
fn ensure_kinds(ws: &mut Workspace, items: &mut [Prepared], report: &mut ImportReport) {
    let mut warned_priorities: BTreeSet<String> = BTreeSet::new();
    for it in items {
        let pk = it.key.project.to_string();
        let r = &mut it.raw;
        if let Some(t) = r.issue_type.as_deref().and_then(summary) {
            let subtask = r.subtask.unwrap_or_else(|| t.to_lowercase().replace(['-', ' ', '_'], "").contains("subtask"));
            match ws.add_issue_type(&pk, &t, subtask) {
                Ok(_) => r.issue_type = Some(t),
                Err(e) => {
                    report.warn(format!("{}: issue type '{}': {e}; imported as Task", r.at, clip(&t, 40)));
                    r.issue_type = None;
                }
            }
        }
        r.status = r.status.as_deref().and_then(summary).and_then(|s| {
            let category = r.status_category.unwrap_or_else(|| match guess_category(&s) {
                StatusCategory::ToDo if r.resolved.is_some() => StatusCategory::Done,
                g => g,
            });
            match ws.add_status(&pk, &s, category) {
                Ok(_) => Some(s),
                Err(e) => {
                    report.warn(format!("{}: status '{}': {e}", r.at, clip(&s, 40)));
                    None
                }
            }
        });
        if let Some(p) = r.priority.take() {
            let mapped = map_priority(&p).unwrap_or_else(|| {
                if warned_priorities.insert(p.to_lowercase()) {
                    report.warn(format!("priority '{}' has no match; imported as Medium", clip(&p, 40)));
                }
                "Medium"
            });
            r.priority = Some(mapped.to_string());
        }
    }
}

/// Resolves parents (by key or id) and returns the creation order: parents before children.
fn creation_order(ws: &Workspace, items: &mut [Prepared], by_id: &HashMap<String, IssueKey>, report: &mut ImportReport) -> Vec<usize> {
    let keys: BTreeSet<IssueKey> = items.iter().map(|i| i.key.clone()).collect();
    for it in items.iter_mut() {
        let Some(p) = it.raw.parent.as_deref().map(str::trim).filter(|p| !p.is_empty()) else { continue };
        match IssueKey::parse(p).ok().or_else(|| by_id.get(p).cloned()) {
            Some(pk) if pk == it.key => report.warn(format!("{}: {} is its own parent; parent left out", it.raw.at, it.key)),
            Some(pk) if keys.contains(&pk) || ws.issues.contains_key(&pk) => it.parent = Some(pk),
            _ => report.warn(format!("{}: parent '{}' of {} is not in the export; parent left out", it.raw.at, clip(p, 40), it.key)),
        }
    }
    let parent_of: HashMap<IssueKey, IssueKey> = items.iter().filter_map(|i| i.parent.clone().map(|p| (i.key.clone(), p))).collect();
    let depth = |k: &IssueKey| -> usize {
        let mut d = 0usize;
        let mut at = k;
        while let Some(p) = parent_of.get(at) {
            d += 1;
            if d > 8 {
                break;
            }
            at = p;
        }
        d
    };
    let mut order: Vec<(usize, usize)> = items.iter().enumerate().map(|(n, i)| (depth(&i.key), n)).collect();
    order.sort();
    order.into_iter().map(|(_, n)| n).collect()
}

/// Creates one issue with its comments; `false` when it couldn't be imported.
fn create(ws: &mut Workspace, it: &Prepared, people: &mut People, actor: Actor<'_>, report: &mut ImportReport) -> bool {
    let r = &it.raw;
    let pk = it.key.project.to_string();
    let Ok(project) = ws.project(&pk) else { return false };
    let types: Vec<(String, bool)> = project.issue_types.iter().map(|t| (t.name.clone(), t.subtask)).collect();
    let parent = it.parent.clone().filter(|p| ws.issues.contains_key(p));
    if it.parent.is_some() && parent.is_none() {
        report.warn(format!("{}: the parent of {} wasn't imported first; parent left out", r.at, it.key));
    }
    let mut ty = r.issue_type.clone();
    let is_sub = ty.as_ref().is_some_and(|t| types.iter().any(|(n, sub)| *sub && n.eq_ignore_ascii_case(t)));
    if is_sub && parent.is_none() {
        report.warn(format!("{}: {} is a {} without a parent; imported as a Task", r.at, it.key, ty.clone().unwrap_or_default()));
        ty = None;
    }
    if ty.is_none() && !types.iter().any(|(n, _)| n == "Task") {
        ty = types.iter().find(|(_, sub)| !sub).map(|(n, _)| n.clone());
    }
    let reporter = people.resolve(ws, &r.reporter, report).unwrap_or_else(|| actor.user.trim().to_string());
    let assignee = people.resolve(ws, &r.assignee, report);
    let points = r.story_points.filter(|p| {
        let ok = p.is_finite() && (0.0..=1_000_000.0).contains(p);
        if !ok {
            report.warn(format!("{}: story points out of range; left out", r.at));
        }
        ok
    });
    let summary = summary(&r.summary).unwrap_or_else(|| {
        report.warn(format!("{}: {} has no summary", r.at, it.key));
        "(no summary)".to_string()
    });
    let new = NewIssue {
        project: pk,
        issue_type: ty,
        summary,
        description: r.description.clone(),
        priority: r.priority.clone(),
        assignee,
        labels: sanitize_labels(&r.labels, &r.at, report),
        parent: parent.map(|p| p.to_string()),
        story_points: points,
    };
    let created_at = r.created.unwrap_or(actor.now);
    if let Err(e) = ws.create_issue_numbered(new, it.key.number, &reporter, created_at) {
        report.warn(format!("{}: {} not imported: {e}", r.at, it.key));
        return false;
    }
    report.issues += 1;
    let key = it.key.to_string();
    if r.comments.len() > MAX_PER_ISSUE {
        report.warn(format!("{}: only the first {MAX_PER_ISSUE} comments of {key} were kept", r.at));
    }
    for c in r.comments.iter().take(MAX_PER_ISSUE).filter(|c| !c.body.is_empty()) {
        let author = people.resolve(ws, &c.author, report).unwrap_or_else(|| actor.user.trim().to_string());
        match ws.add_comment(&key, &author, c.body.clone(), c.at.unwrap_or(created_at)) {
            Ok(_) => report.comments += 1,
            Err(e) => report.warn(format!("{}: a comment on {key}: {e}", r.at)),
        }
    }
    true
}

/// Links between created issues and issues already in the workspace.
fn links(ws: &mut Workspace, items: &[Prepared], created: &[usize], actor: Actor<'_>, report: &mut ImportReport) {
    let mut warned_kinds: BTreeSet<String> = BTreeSet::new();
    for it in created.iter().filter_map(|&n| items.get(n)) {
        for l in it.raw.links.iter().take(MAX_PER_ISSUE) {
            let Ok(other) = IssueKey::parse(&l.other) else {
                report.warn(format!("{}: link target '{}' is not an issue key", it.raw.at, clip(&l.other, 40)));
                continue;
            };
            if !ws.issues.contains_key(&other) {
                report.warn(format!("{}: {} links to {other}, which isn't in the workspace; link left out", it.raw.at, it.key));
                continue;
            }
            let kind = map_link(&l.kind).unwrap_or_else(|| {
                if warned_kinds.insert(l.kind.to_lowercase()) {
                    report.warn(format!("link type '{}' has no match; imported as 'relates to'", clip(&l.kind, 40)));
                }
                LinkKind::Relates
            });
            let (src, dst) = if l.outward { (it.key.clone(), other) } else { (other, it.key.clone()) };
            let holds =
                |w: &Workspace, a: &IssueKey, b: &IssueKey| w.issues.get(a).is_some_and(|i| i.links.iter().any(|x| x.kind == kind && &x.target == b));
            if src == dst || holds(ws, &src, &dst) || (kind == LinkKind::Relates && holds(ws, &dst, &src)) {
                continue;
            }
            match ws.link(&src.to_string(), kind, &dst.to_string(), actor.user, actor.now) {
                Ok(()) => report.links += 1,
                Err(e) => report.warn(format!("{}: link {src} → {dst}: {e}", it.raw.at)),
            }
        }
    }
}

/// Sprints: created by name per project, states as given (JSON) or guessed (CSV; see
/// `docs/import.md`), then each issue placed in its current and past sprints.
fn sprints(ws: &mut Workspace, items: &[Prepared], created: &[usize], actor: Actor<'_>, report: &mut ImportReport) {
    // Per project: sprint name → (first appearance, info, member indices, closed-by-carry-over).
    struct Info {
        first: usize,
        raw: RawSprint,
        members: Vec<usize>,
        carried: bool,
    }
    let mut by_project: BTreeMap<ProjectKey, BTreeMap<String, Info>> = BTreeMap::new();
    let mut counter = 0usize;
    for &n in created {
        let Some(it) = items.get(n) else { continue };
        let list: Vec<&RawSprint> = it.raw.sprints.iter().filter(|s| !s.name.trim().is_empty()).take(MAX_PER_ISSUE).collect();
        let last = list.len().saturating_sub(1);
        for (pos, s) in list.into_iter().enumerate() {
            let Some(name) = summary(&s.name) else { continue };
            let e = by_project.entry(it.key.project.clone()).or_default().entry(name).or_insert_with(|| {
                counter += 1;
                Info { first: counter, raw: s.clone(), members: vec![], carried: false }
            });
            if e.raw.state.is_none() {
                e.raw.state = s.state;
            }
            e.members.push(n);
            if pos < last {
                e.carried = true;
            }
        }
    }
    let done = |ws: &Workspace, n: usize| {
        items.get(n).and_then(|it| ws.issues.get(&it.key)).is_some_and(|i| {
            let st = items.get(n).and_then(|it| it.raw.status.clone()).unwrap_or_else(|| i.status.clone());
            ws.project(i.key.project.as_str()).ok().and_then(|p| p.workflow.status(&st)).is_some_and(|s| s.category == StatusCategory::Done)
        })
    };
    let fortnight = chrono::Duration::days(14);
    let mut ids: HashMap<(ProjectKey, String), trackcraft_core::Id> = HashMap::new();
    for (project, sprints) in &by_project {
        // Guess states for sprints the export didn't describe.
        let mut states: BTreeMap<String, SprintState> = BTreeMap::new();
        let mut open: Vec<(u64, usize, String)> = Vec::new();
        for (name, info) in sprints {
            match info.raw.state {
                Some(s) => {
                    states.insert(name.clone(), s);
                }
                None if info.carried || info.members.iter().all(|&m| done(ws, m)) => {
                    states.insert(name.clone(), SprintState::Closed);
                }
                None => open.push((sprint_number(name), info.first, name.clone())),
            }
        }
        open.sort();
        let mut has_active = ws.active_sprint(project).is_some() || states.values().any(|s| *s == SprintState::Active);
        for (_, _, name) in open {
            let s = if has_active { SprintState::Future } else { SprintState::Active };
            has_active = true;
            states.insert(name, s);
        }
        // Create (or reuse by name).
        let mut active_made = ws.active_sprint(project).is_some();
        let mut by_first: Vec<(&String, &Info)> = sprints.iter().collect();
        by_first.sort_by_key(|(_, i)| i.first);
        for (name, info) in by_first {
            if let Some(existing) = ws.sprints.values().find(|s| &s.project == project && &s.name == name) {
                ids.insert((project.clone(), name.clone()), existing.id);
                continue;
            }
            let mut state = states.get(name).copied().unwrap_or(SprintState::Future);
            if state == SprintState::Active && active_made {
                report.warn(format!("sprint '{name}': {project} already has an active sprint; imported as a future sprint"));
                state = SprintState::Future;
            }
            let (mut start, mut end) = (info.raw.start, info.raw.end);
            if let (Some(s), Some(e)) = (start, end)
                && e <= s
            {
                end = None;
            }
            if state == SprintState::Active {
                let s = start.unwrap_or(actor.now);
                start = Some(s);
                end = Some(end.unwrap_or(s + fortnight));
            }
            match ws.import_sprint(project.as_str(), name, state, start, end, info.raw.completed) {
                Ok(id) => {
                    if state == SprintState::Active {
                        active_made = true;
                    }
                    report.sprints += 1;
                    ids.insert((project.clone(), name.clone()), id);
                }
                Err(e) => report.warn(format!("sprint '{name}': {e}")),
            }
        }
    }
    // Membership: sub-tasks after their parents, following the parent's open sprint.
    let mut order: Vec<usize> = created.to_vec();
    order.sort_by_key(|&n| items.get(n).is_some_and(|it| it.parent.is_some()));
    for n in order {
        let Some(it) = items.get(n) else { continue };
        let mut current = None;
        let mut past = Vec::new();
        for s in &it.raw.sprints {
            let Some(name) = summary(&s.name) else { continue };
            let Some(&id) = ids.get(&(it.key.project.clone(), name)) else { continue };
            match ws.sprints.get(&id).map(|s| s.state) {
                Some(SprintState::Closed) => past.push(id),
                Some(_) => current = Some(id),
                None => {}
            }
        }
        let issue = ws.issues.get(&it.key);
        if issue.is_some_and(|i| ws.is_subtask(i)) {
            current = issue.and_then(|i| i.parent.as_ref()).and_then(|p| ws.issues.get(p)).and_then(|p| p.sprint);
        }
        if current.is_none() && past.is_empty() {
            continue;
        }
        if let Err(e) = ws.set_imported_sprints(&it.key.to_string(), current, &past) {
            report.warn(format!("{}: sprints of {}: {e}", it.raw.at, it.key));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let want: Timestamp = "2024-03-21T15:07:00Z".parse().unwrap();
        for s in ["21/Mar/24 3:07 PM", "21/mar/24 3:07 pm", "2024-03-21 15:07", "2024-03-21T15:07:00.000+0000", "2024-03-21T16:07:00+01:00"] {
            assert_eq!(parse_date(s), Some(want), "{s}");
        }
        assert!(parse_date("2024-03-21").is_some());
        for bad in ["", "yesterday", "99/Foo/24 3:07 PM", &"9".repeat(1000)] {
            assert_eq!(parse_date(bad), None);
        }
    }

    #[test]
    fn guesses() {
        assert_eq!(guess_category("Closed"), StatusCategory::Done);
        assert_eq!(guess_category("Won't Fix"), StatusCategory::Done);
        assert_eq!(guess_category("Code Review"), StatusCategory::InProgress);
        assert_eq!(guess_category("Backlog"), StatusCategory::ToDo);
        assert_eq!(map_priority("Blocker"), Some("Highest"));
        assert_eq!(map_priority("minor"), Some("Low"));
        assert_eq!(map_priority("Whenever"), None);
        assert_eq!(map_link("Cloners"), Some(LinkKind::Clones));
        assert_eq!(sprint_number("Team Sprint 12"), 12);
    }
}
