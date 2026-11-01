//! Reports (M2.5 / UI-24): sprint burndown, velocity, cumulative flow, created vs resolved, and
//! a pie of issue counts by field. Read-only queries; the exact rules are in `docs/reports.md`.
//!
//! Every report is rebuilt from the issues' change history. The current value of a field is
//! known; walking the history backwards, before each entry the field held the entry's `from`
//! value. That gives every field's value at any moment without trusting older entries' `to`
//! values to agree with today's data (sprint renames, imports).
//!
//! Work is bounded: at most [`MAX_DAYS`] days per series, [`MAX_ISSUES`] issues per report
//! (beyond that the report says `"truncated": true`), [`MAX_EVENTS`] scope events. Time comes from
//! the session clock and days are UTC calendar days, so the same workspace and clock give the
//! same report everywhere. Under enforcement (servers) the session runs queries on the acting
//! user's filtered view, so reports only count what that user can browse.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Duration, NaiveDate};
use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_model::{Change, Issue, Sprint, SprintState, StatusCategory, Timestamp, Workspace};

use super::issue::search;
use crate::{CmdResult, CommandSpec, Session, json as view, params};

/// Most days in one series (a year and a day).
pub const MAX_DAYS: usize = 366;
/// Most issues one report reads; the rest are left out and the report says `"truncated": true`.
pub const MAX_ISSUES: usize = 20_000;
/// Most scope-change events a burndown lists.
pub const MAX_EVENTS: usize = 2_000;
/// Most closed sprints a velocity report shows.
pub const MAX_VELOCITY_SPRINTS: usize = 50;
/// Most slices a pie shows; the rest are summed as "Other".
pub const MAX_SLICES: usize = 50;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("report.burndown", "Burndown Chart", "Project › Reports", burndown)
            .params(r#"{"sprint": 7 | "active", "project": "LUME" (with "active"), "unit": "points" | "issues"} → remaining work per day from the sprint's start to its completion (or now), the ideal line and scope changes"#)
            .query(),
        CommandSpec::new("report.velocity", "Velocity Chart", "Project › Reports", velocity)
            .params(r#"{"project": "LUME", "sprints": 6 (last n closed sprints, at most 50), "unit": "points" | "issues"} → per closed sprint: committed at its start vs completed at its end"#)
            .query(),
        CommandSpec::new("report.cfd", "Cumulative Flow Diagram", "Project › Reports", cfd)
            .params(r#"{"project": "LUME", "from": "2026-10-01", "to": "2026-10-31" (default: today), "days": 30 (when "from" is left out; at most 366)} → issues per status category at the end of each day"#)
            .query(),
        CommandSpec::new("report.createdResolved", "Created vs Resolved", "Project › Reports", created_resolved)
            .params(r#"{"project": "LUME" | "jql": "…" ("" = every issue you can see), "days": 30 (1–366)} → issues created and resolved per day"#)
            .query(),
        CommandSpec::new("report.pie", "Pie Chart", "Project › Reports", pie)
            .params(r#"{"jql": "project = LUME", "field": "status" | "assignee" | "priority" | "type" | "labels"} → issue counts per value, largest first"#)
            .query(),
    ]
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    Points,
    Issues,
}

impl Unit {
    fn parse(p: &Value) -> Result<Unit, String> {
        match params::opt_str(p, "unit")?.map(str::to_ascii_lowercase).as_deref() {
            None | Some("points" | "storypoints" | "story points") => Ok(Unit::Points),
            Some("issues" | "count" | "issuecount") => Ok(Unit::Issues),
            Some(_) => Err("\"unit\" is \"points\" or \"issues\"".into()),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Unit::Points => "points",
            Unit::Issues => "issues",
        }
    }
}

// ---- timelines -----------------------------------------------------------------------------

/// One field's value over time: `initial` until the first step, then each step's value from its
/// time on. Steps are sorted by time.
struct Timeline<T> {
    initial: T,
    steps: Vec<(Timestamp, T)>,
}

impl<T> Timeline<T> {
    /// The value at `t` (entries recorded at exactly `t` have happened).
    fn at(&self, t: Timestamp) -> &T {
        let n = self.steps.partition_point(|(a, _)| *a <= t);
        match n.checked_sub(1).and_then(|i| self.steps.get(i)) {
            Some((_, v)) => v,
            None => &self.initial,
        }
    }
    fn times(&self) -> impl Iterator<Item = Timestamp> + '_ {
        self.steps.iter().map(|(a, _)| *a)
    }
}

/// Rebuilds a timeline backwards from the `current` value: before each entry (oldest first) the
/// field held `before(entry)`; after the last entry it holds `current`.
fn backward<T: Clone>(entries: &[&Change], current: T, before: impl Fn(&Change) -> T) -> Timeline<T> {
    let mut steps = Vec::with_capacity(entries.len());
    for (k, e) in entries.iter().enumerate() {
        let after = match entries.get(k.saturating_add(1)) {
            Some(next) => before(next),
            None => current.clone(),
        };
        steps.push((e.at, after));
    }
    let initial = match entries.first() {
        Some(e) => before(e),
        None => current,
    };
    Timeline { initial, steps }
}

/// An issue's history entries for `field`, sorted by time (stable, so same-time entries keep
/// their recorded order).
fn entries<'a>(i: &'a Issue, field: &str) -> Vec<&'a Change> {
    let mut v: Vec<&Change> = i.history.iter().filter(|c| c.field == field).collect();
    v.sort_by_key(|c| c.at);
    v
}

fn parse_points(s: Option<&String>) -> Option<f64> {
    s.and_then(|s| s.trim().parse::<f64>().ok()).filter(|p| p.is_finite() && *p >= 0.0)
}

/// What a report needs of one issue at any moment: its status category and estimate.
struct Track {
    created: Timestamp,
    status: Timeline<StatusCategory>,
    points: Timeline<Option<f64>>,
}

fn track(ws: &Workspace, i: &Issue) -> Track {
    let wf = ws.projects.get(&i.key.project).map(|p| &p.workflow);
    // A status the workflow no longer has counts as To Do (as `statusCategory` does today).
    let category = |name: Option<&String>| name.and_then(|n| wf.and_then(|w| w.status(n))).map_or(StatusCategory::ToDo, |s| s.category);
    let status = backward(&entries(i, "status"), ws.status_category(i), |c| category(c.from.as_ref()));
    let points = backward(&entries(i, "storyPoints"), i.story_points, |c| parse_points(c.from.as_ref()));
    Track { created: i.created, status, points }
}

// ---- dates ---------------------------------------------------------------------------------

fn midnight(d: NaiveDate) -> Option<Timestamp> {
    d.and_hms_opt(0, 0, 0).map(|d| d.and_utc())
}

/// The last instant of UTC day `d`.
fn end_of_day(d: NaiveDate) -> Option<Timestamp> {
    midnight(d.succ_opt()?)?.checked_sub_signed(Duration::nanoseconds(1))
}

/// Calendar days `from..=to`, at most [`MAX_DAYS`]; the flag says the range was cut.
fn days(from: NaiveDate, to: NaiveDate) -> (Vec<NaiveDate>, bool) {
    let mut out = Vec::new();
    let mut d = from;
    while d <= to {
        if out.len() >= MAX_DAYS {
            return (out, true);
        }
        out.push(d);
        match d.succ_opt() {
            Some(n) => d = n,
            None => break,
        }
    }
    (out, false)
}

fn date_str(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn ts(t: Timestamp) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Rounds for output (estimates are decimals; sums shouldn't show float noise).
fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// ---- sprints -------------------------------------------------------------------------------

/// One issue's part in a sprint: its fields over time and whether it was in the sprint.
struct Member {
    key: String,
    track: Track,
    member: Timeline<bool>,
}

struct SprintData {
    members: Vec<Member>,
    truncated: bool,
}

/// One issue's state at a moment, as a sprint report sees it.
#[derive(Clone, Copy, PartialEq)]
struct State {
    in_sprint: bool,
    done: bool,
    value: f64,
}

impl Member {
    fn state(&self, t: Timestamp, unit: Unit) -> State {
        if t < self.track.created {
            return State { in_sprint: false, done: false, value: 0.0 };
        }
        let value = match unit {
            Unit::Points => self.track.points.at(t).unwrap_or(0.0),
            Unit::Issues => 1.0,
        };
        State { in_sprint: *self.member.at(t), done: *self.track.status.at(t) == StatusCategory::Done, value }
    }
}

#[derive(Default, Clone, Copy)]
struct Totals {
    /// Everything in the sprint.
    scope: f64,
    /// In the sprint and not in a Done-category status.
    remaining: f64,
    /// In the sprint and done.
    done: f64,
}

fn totals(members: &[Member], t: Timestamp, unit: Unit) -> Totals {
    let mut out = Totals::default();
    for m in members {
        let s = m.state(t, unit);
        if s.in_sprint {
            out.scope += s.value;
            if s.done {
                out.done += s.value;
            } else {
                out.remaining += s.value;
            }
        }
    }
    out
}

/// The moment a sprint's membership is known exactly: its completion for a closed sprint (every
/// issue in it then keeps it in `past_sprints`), otherwise now (`issue.sprint`).
fn anchor(s: &Sprint) -> Option<Timestamp> {
    if s.state == SprintState::Closed { s.completed.or(s.end) } else { None }
}

fn anchored_in(i: &Issue, s: &Sprint) -> bool {
    if s.state == SprintState::Closed { i.past_sprints.contains(&s.id) || i.sprint == Some(s.id) } else { i.sprint == Some(s.id) }
}

/// `sprint` history entries before the anchor (entries at a closed sprint's completion are the
/// completion moving open issues on: they happen after its end).
fn sprint_entries(i: &Issue, anchor: Option<Timestamp>) -> Vec<&Change> {
    let mut v = entries(i, "sprint");
    if let Some(a) = anchor {
        v.retain(|c| c.at < a);
    }
    v
}

/// The sprint's issues over time (sub-tasks left out: they follow their parent and carry its
/// work). History stores sprint *names*; a sprint renamed since keeps its old names as aliases,
/// learnt from entries that moved an issue into it.
fn sprint_data(ws: &Workspace, s: &Sprint) -> SprintData {
    let a = anchor(s);
    let issues: Vec<&Issue> = ws.issues.values().filter(|i| i.key.project == s.project && !ws.is_subtask(i)).collect();
    let mut aliases: BTreeSet<String> = BTreeSet::from([s.name.clone()]);
    // Two passes: an alias learnt from one issue can explain another's older entries.
    for _ in 0..2 {
        for i in &issues {
            let mut known = anchored_in(i, s);
            for e in sprint_entries(i, a).iter().rev() {
                if known && let Some(to) = &e.to {
                    aliases.insert(to.clone());
                }
                known = e.from.as_ref().is_some_and(|f| aliases.contains(f));
            }
        }
    }
    let touches = |i: &Issue| {
        anchored_in(i, s) || sprint_entries(i, a).iter().any(|e| [&e.from, &e.to].iter().any(|v| v.as_ref().is_some_and(|n| aliases.contains(n))))
    };
    let mut members = Vec::new();
    let mut truncated = false;
    for i in issues.into_iter().filter(|i| touches(i)) {
        if members.len() >= MAX_ISSUES {
            truncated = true;
            break;
        }
        let member = backward(&sprint_entries(i, a), anchored_in(i, s), |e| e.from.as_ref().is_some_and(|f| aliases.contains(f)));
        members.push(Member { key: i.key.to_string(), track: track(ws, i), member });
    }
    SprintData { members, truncated }
}

fn pick_sprint<'a>(s: &'a Session, p: &Value) -> Result<&'a Sprint, String> {
    match p.get("sprint") {
        Some(Value::String(v)) if v.trim().eq_ignore_ascii_case("active") => {
            let key = s.ws.project(params::str(p, "project")?).map_err(err)?.key.clone();
            s.ws.active_sprint(&key).ok_or_else(|| format!("project {key} has no active sprint"))
        }
        None | Some(Value::Null) => Err("missing \"sprint\" (a sprint id, or \"active\" with \"project\")".into()),
        Some(_) => {
            let id: Id = params::id(p, "sprint").map_err(|_| "\"sprint\" is a sprint id or \"active\"".to_string())?;
            s.ws.sprint(id).map_err(err)
        }
    }
}

fn burndown(s: &mut Session, p: &Value) -> CmdResult {
    let unit = Unit::parse(p)?;
    let sp = pick_sprint(s, p)?;
    let (Some(start), Some(end)) = (sp.start, sp.end) else {
        return Err(format!("{} hasn't started: a burndown begins at the sprint's start date", sp.name));
    };
    let now = s.now();
    // The series runs to the completion of a closed sprint, else to now (an overdue active
    // sprint keeps burning down until someone completes it).
    let last = match sp.state {
        SprintState::Closed => sp.completed.unwrap_or(end),
        _ => now,
    }
    .max(start);
    let data = sprint_data(&s.ws, sp);
    let m = &data.members;
    let committed = totals(m, start, unit);
    let (dates, cut) = days(start.date_naive(), last.date_naive());
    let mut rows = Vec::with_capacity(dates.len());
    let span = (end - start).num_milliseconds().max(1) as f64;
    let ideal_at = |t: Timestamp| {
        let f = ((t - start).num_milliseconds() as f64 / span).clamp(0.0, 1.0);
        round(committed.scope * (1.0 - f))
    };
    // Days since the start: where a point sits on a time axis.
    let offset = |t: Timestamp| ((t - start).num_milliseconds() as f64 / 86_400_000.0 * 10_000.0).round() / 10_000.0;
    for d in &dates {
        let Some(t) = end_of_day(*d).map(|e| e.min(last)) else { continue };
        let x = totals(m, t, unit);
        rows.push(json!({
            "date": date_str(*d), "at": ts(t), "offset": offset(t),
            "remaining": round(x.remaining), "scope": round(x.scope), "done": round(x.done), "ideal": ideal_at(t),
        }));
    }
    let (ideal_dates, _) = days(start.date_naive(), end.date_naive());
    let ideal: Vec<Value> =
        std::iter::once(json!({"date": date_str(start.date_naive()), "at": ts(start), "offset": 0.0, "remaining": round(committed.scope)}))
            .chain(ideal_dates.iter().filter_map(|d| {
                let t = end_of_day(*d)?.min(end);
                Some(json!({"date": date_str(*d), "at": ts(t), "offset": offset(t), "remaining": ideal_at(t)}))
            }))
            .collect();
    let (events, events_cut) = scope_events(m, start, last, unit);
    let now_t = totals(m, last, unit);
    let mut out = json!({
        "sprint": view::sprint(&s.ws, sp),
        "unit": unit.name(),
        "start": ts(start), "end": ts(end), "last": ts(last),
        "committed": round(committed.scope),
        "scope": round(now_t.scope), "remaining": round(now_t.remaining), "done": round(now_t.done),
        "days": rows, "ideal": ideal, "events": events,
        "truncated": data.truncated || cut || events_cut,
    });
    if let Some(o) = out.as_object_mut()
        && let Some(c) = sp.completed
    {
        o.insert("completed".into(), json!(ts(c)));
    }
    Ok(out)
}

/// What changed the burndown after the sprint started: issues added or removed, estimates
/// changed, issues done or reopened. One event per issue per moment; `remainingDelta` and
/// `scopeDelta` are its effect on the two lines.
fn scope_events(members: &[Member], start: Timestamp, last: Timestamp, unit: Unit) -> (Vec<Value>, bool) {
    let mut out: Vec<(Timestamp, String, Value)> = Vec::new();
    let mut cut = false;
    for m in members {
        let mut times: BTreeSet<Timestamp> = m.track.status.times().chain(m.track.points.times()).chain(m.member.times()).collect();
        times.insert(m.track.created);
        let mut before = m.state(start, unit);
        for t in times.into_iter().filter(|t| *t > start && *t <= last) {
            let after = m.state(t, unit);
            if after == before {
                continue;
            }
            let rem = |s: &State| if s.in_sprint && !s.done { s.value } else { 0.0 };
            let scope = |s: &State| if s.in_sprint { s.value } else { 0.0 };
            let kind = match (before.in_sprint, after.in_sprint) {
                (false, true) => Some("added"),
                (true, false) => Some("removed"),
                (true, true) if !before.done && after.done => Some("completed"),
                (true, true) if before.done && !after.done => Some("reopened"),
                (true, true) if before.value != after.value => Some("estimateChanged"),
                _ => None,
            };
            if let Some(kind) = kind {
                if out.len() >= MAX_EVENTS {
                    cut = true;
                } else {
                    let mut e = json!({
                        "at": ts(t), "key": m.key, "kind": kind,
                        "remainingDelta": round(rem(&after) - rem(&before)), "scopeDelta": round(scope(&after) - scope(&before)),
                    });
                    if kind == "estimateChanged"
                        && let Some(o) = e.as_object_mut()
                    {
                        o.insert("from".into(), json!(round(before.value)));
                        o.insert("to".into(), json!(round(after.value)));
                    }
                    out.push((t, m.key.clone(), e));
                }
            }
            before = after;
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    (out.into_iter().map(|(_, _, v)| v).collect(), cut)
}

fn velocity(s: &mut Session, p: &Value) -> CmdResult {
    let unit = Unit::parse(p)?;
    let key = s.ws.project(params::str(p, "project")?).map_err(err)?.key.clone();
    let n = params::usize_or(p, "sprints", 6, MAX_VELOCITY_SPRINTS)?.max(1);
    let mut closed: Vec<&Sprint> = s.ws.sprints.values().filter(|sp| sp.project == key && sp.state == SprintState::Closed).collect();
    closed.sort_by_key(|sp| (sp.completed.or(sp.end), sp.id));
    let skip = closed.len().saturating_sub(n);
    let mut rows = Vec::new();
    let mut truncated = false;
    for sp in closed.into_iter().skip(skip) {
        let (Some(start), Some(done_at)) = (sp.start, sp.completed.or(sp.end)) else { continue };
        let data = sprint_data(&s.ws, sp);
        truncated |= data.truncated;
        let committed = totals(&data.members, start, unit).scope;
        let completed = totals(&data.members, done_at, unit).done;
        rows.push(json!({
            "id": sp.id.0, "name": sp.name, "start": ts(start), "completed": ts(done_at),
            "committed": round(committed), "completedValue": round(completed),
        }));
    }
    let avg = if rows.is_empty() { 0.0 } else { rows.iter().filter_map(|r| r["completedValue"].as_f64()).sum::<f64>() / rows.len() as f64 };
    Ok(json!({"project": key.as_str(), "unit": unit.name(), "sprints": rows, "average": round(avg), "truncated": truncated}))
}

/// A `from`/`to` date range: `to` defaults to today, `from` to `days` (30) days back; at most [`MAX_DAYS`].
fn range(s: &Session, p: &Value) -> Result<(NaiveDate, NaiveDate), String> {
    let today = s.now().date_naive();
    let to = params::opt_date(p, "to")?.map_or(today, |t| t.date_naive());
    let back = i64::try_from(params::usize_or(p, "days", 30, MAX_DAYS)?.max(1)).unwrap_or(30).saturating_sub(1);
    let from = match params::opt_date(p, "from")? {
        Some(f) => f.date_naive(),
        None => to.checked_sub_signed(Duration::days(back)).unwrap_or(to),
    };
    if from > to {
        return Err("\"from\" must be on or before \"to\"".into());
    }
    if (to - from).num_days() >= MAX_DAYS as i64 {
        return Err(format!("a report covers at most {MAX_DAYS} days"));
    }
    Ok((from, to))
}

fn cfd(s: &mut Session, p: &Value) -> CmdResult {
    let key = s.ws.project(params::str(p, "project")?).map_err(err)?.key.clone();
    let (from, to) = range(s, p)?;
    let all: Vec<&Issue> = s.ws.issues.values().filter(|i| i.key.project == key).collect();
    let truncated = all.len() > MAX_ISSUES;
    let tracks: Vec<Track> = all.into_iter().take(MAX_ISSUES).map(|i| track(&s.ws, i)).collect();
    let now = s.now();
    let (dates, _) = days(from, to);
    let mut rows = Vec::with_capacity(dates.len());
    for d in dates {
        let Some(t) = end_of_day(d).map(|e| e.min(now)) else { continue };
        let (mut todo, mut doing, mut done) = (0u64, 0u64, 0u64);
        for tr in tracks.iter().filter(|tr| tr.created <= t) {
            match tr.status.at(t) {
                StatusCategory::ToDo => todo += 1,
                StatusCategory::InProgress => doing += 1,
                StatusCategory::Done => done += 1,
            }
        }
        rows.push(json!({"date": date_str(d), "toDo": todo, "inProgress": doing, "done": done}));
    }
    Ok(json!({
        "project": key.as_str(), "from": date_str(from), "to": date_str(to),
        "categories": ["To Do", "In Progress", "Done"], "days": rows, "truncated": truncated,
    }))
}

/// The issues a project-or-JQL report reads (`project` and `jql` together = both must match;
/// `"jql": ""` alone = every issue).
fn scoped_issues<'a>(s: &'a Session, p: &Value) -> Result<(Vec<&'a Issue>, bool, String), String> {
    let project = params::opt_str(p, "project")?.map(str::trim).filter(|x| !x.is_empty());
    let jql = params::opt_str(p, "jql")?.map(str::trim);
    let (query, label) = match (project, jql) {
        (None, None) => return Err("give \"project\" or \"jql\" (\"\" = every issue)".into()),
        (Some(pk), q) => {
            let q = q.filter(|x| !x.is_empty());
            let key = s.ws.project(pk).map_err(err)?.key.clone();
            match q {
                Some(q) => (format!("project = {key} AND ({q})"), format!("project = {key} AND ({q})")),
                None => (format!("project = {key}"), format!("project = {key}")),
            }
        }
        (None, Some(q)) => (q.to_string(), q.to_string()),
    };
    let keys = search(s, &query)?;
    let truncated = keys.len() > MAX_ISSUES;
    Ok((keys.iter().take(MAX_ISSUES).filter_map(|k| s.ws.issues.get(k)).collect(), truncated, label))
}

fn created_resolved(s: &mut Session, p: &Value) -> CmdResult {
    let n = params::usize_or(p, "days", 30, MAX_DAYS)?.max(1);
    let (issues, truncated, query) = scoped_issues(s, p)?;
    let today = s.now().date_naive();
    let from = today.checked_sub_signed(Duration::days(i64::try_from(n).unwrap_or(1).saturating_sub(1))).unwrap_or(today);
    let (dates, _) = days(from, today);
    let mut created: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    let mut resolved: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    for i in &issues {
        *created.entry(i.created.date_naive()).or_default() += 1;
        if let Some(r) = i.resolved {
            *resolved.entry(r.date_naive()).or_default() += 1;
        }
    }
    let get = |m: &BTreeMap<NaiveDate, u64>, d: &NaiveDate| m.get(d).copied().unwrap_or(0);
    let rows: Vec<Value> = dates.iter().map(|d| json!({"date": date_str(*d), "created": get(&created, d), "resolved": get(&resolved, d)})).collect();
    let sum = |m: &BTreeMap<NaiveDate, u64>| dates.iter().map(|d| get(m, d)).sum::<u64>();
    Ok(json!({
        "jql": query, "from": date_str(from), "to": date_str(today), "days": rows,
        "created": sum(&created), "resolved": sum(&resolved), "truncated": truncated,
    }))
}

fn pie(s: &mut Session, p: &Value) -> CmdResult {
    let field = params::str(p, "field")?.trim().to_ascii_lowercase();
    if !matches!(field.as_str(), "status" | "assignee" | "priority" | "type" | "issuetype" | "labels") {
        return Err("\"field\" is status, assignee, priority, type or labels".into());
    }
    let jql = params::opt_str(p, "jql")?.unwrap_or("");
    let keys = search(s, jql)?;
    let truncated = keys.len() > MAX_ISSUES;
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;
    for i in keys.iter().take(MAX_ISSUES).filter_map(|k| s.ws.issues.get(k)) {
        total += 1;
        let values: Vec<String> = match field.as_str() {
            "status" => vec![i.status.clone()],
            "assignee" => vec![i.assignee.clone().unwrap_or_default()],
            "priority" => vec![i.priority.clone()],
            "labels" if i.labels.is_empty() => vec![String::new()],
            "labels" => i.labels.clone(),
            _ => vec![i.issue_type.clone()],
        };
        for v in values {
            *counts.entry(v).or_default() += 1;
        }
    }
    let mut v: Vec<(String, u64)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let other: u64 = v.iter().skip(MAX_SLICES).map(|(_, n)| n).sum();
    v.truncate(MAX_SLICES);
    let name = |value: &str| -> String {
        match (field.as_str(), value) {
            ("assignee", "") => "Unassigned".into(),
            ("labels", "") => "No labels".into(),
            ("assignee", u) => s.ws.users.get(u).map_or_else(|| u.to_string(), |u| u.display_name.clone()),
            (_, x) => x.to_string(),
        }
    };
    let pct = |n: u64| if total == 0 { 0.0 } else { (n as f64 * 1000.0 / total as f64).round() / 10.0 };
    let mut slices: Vec<Value> = v.iter().map(|(val, n)| json!({"value": val, "name": name(val), "count": n, "percent": pct(*n)})).collect();
    if other > 0 {
        slices.push(json!({"value": "", "name": "Other", "count": other, "percent": pct(other)}));
    }
    Ok(json!({"jql": jql, "field": field, "total": total, "slices": slices, "truncated": truncated}))
}
