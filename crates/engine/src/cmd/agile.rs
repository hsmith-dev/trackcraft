//! Planning commands: the backlog, rank and sprints.
//!
//! The backlog follows Atlassian's public description of scrum backlogs: one section per open
//! sprint (the active one first, then future sprints in creation order) and the Backlog section
//! for everything else, each in rank order. The Backlog section leaves out Done-category issues
//! (on a scrum board, done work not in a sprint is no longer shown in the backlog), epics (they
//! are planned through their child issues) and sub-tasks (they follow their parent). Sprint rules
//! are documented on `trackcraft_model::Sprint`'s module.

use std::collections::HashSet;

use serde_json::{Value, json};
use trackcraft_core::{Id, IssueKey};
use trackcraft_model::{Anchor, Issue, MoveOpenTo, SprintEdit, SprintState, StatusCategory};

use super::issue::search;
use crate::{CmdResult, CommandSpec, Session, json as view, params};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("board.backlog", "Backlog", "Project › Backlog", board_backlog)
            .params(r#"{"project": "LUME", "jql": "assignee = currentUser() (optional filter)"} → open sprints (active first) and the backlog, each in rank order; the backlog leaves out Done issues, epics and sub-tasks"#)
            .query(),
        CommandSpec::new("sprint.list", "All Sprints", "Project › Backlog", sprint_list)
            .params(r#"{"project": "LUME"} → every sprint, active first, then future, then closed"#)
            .query(),
        CommandSpec::new("issue.rank", "Rank Issues", "Project › Backlog", issue_rank)
            .params(r#"{"keys": ["LUME-5", "LUME-6"], "before": "LUME-3" | "after": "LUME-3"} (moved issues keep the given order; at most 1000; one project)"#),
        CommandSpec::new("issue.setSprint", "Move to Sprint", "Project › Backlog", issue_set_sprint)
            .params(r#"{"keys": ["LUME-5"], "sprint": 7 | null (the backlog), "before": "LUME-3" | "after": "LUME-3" (optional rank)} (sub-tasks follow their parent)"#),
        CommandSpec::new("sprint.create", "Create Sprint", "Project › Backlog", sprint_create).params(
            r#"{"project": "LUME", "name": "default: LUME Sprint N", "goal": "…", "startDate": "2026-11-03", "endDate": "2026-11-14" (each optional; dates YYYY-MM-DD or RFC 3339)}"#,
        ),
        CommandSpec::new("sprint.edit", "Edit Sprint", "Project › Backlog", sprint_edit)
            .params(r#"{"id": 7, "name": "…", "goal": "…", "startDate": "2026-11-03" | null, "endDate": "2026-11-14" | null (each optional; null clears a future sprint's date; a closed sprint keeps its dates)}"#),
        CommandSpec::new("sprint.delete", "Delete Sprint", "Project › Backlog", sprint_delete)
            .params(r#"{"id": 7} (its issues return to the backlog; a closed sprint can't be deleted)"#),
        CommandSpec::new("sprint.start", "Start Sprint", "Project › Backlog", sprint_start).params(
            r#"{"id": 7, "startDate": "2026-11-03", "endDate": "2026-11-17", "name": "…", "goal": "…" (name and goal optional)} (one active sprint per project; needs an issue that isn't done)"#,
        ),
        CommandSpec::new("sprint.complete", "Complete Sprint", "Project › Backlog", sprint_complete)
            .params(r#"{"id": 7, "moveOpenTo": 8 | "new" | "backlog"} (done issues stay with the closed sprint; open ones move; refused while a done issue has open sub-tasks)"#),
    ]
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn is_subtask(s: &Session, i: &Issue) -> bool {
    s.ws.is_subtask(i)
}

fn board_backlog(s: &mut Session, p: &Value) -> CmdResult {
    let key = s.ws.project(params::str(p, "project")?).map_err(err)?.key.clone();
    let filter: Option<HashSet<IssueKey>> = match params::opt_str(p, "jql")?.filter(|q| !q.trim().is_empty()) {
        Some(q) => Some(search(s, &format!("project = {key} AND ({q})"))?.into_iter().collect()),
        None => None,
    };
    let order: Vec<&Issue> =
        s.ws.rank_order(&key)
            .iter()
            .filter_map(|k| s.ws.issues.get(k))
            .filter(|i| filter.as_ref().is_none_or(|f| f.contains(&i.key)) && !is_subtask(s, i))
            .collect();
    let sprints: Vec<Value> =
        s.ws.project_sprints(&key)
            .into_iter()
            .filter(|sp| sp.state != SprintState::Closed)
            .map(|sp| {
                let issues: Vec<&Issue> = order.iter().copied().filter(|i| i.sprint == Some(sp.id)).collect();
                let (points, counts) = view::tally(&s.ws, issues.iter().copied());
                let mut v = view::sprint(&s.ws, sp);
                if let Some(o) = v.as_object_mut() {
                    o.insert("issues".into(), Value::Array(issues.iter().map(|i| view::planning_row(&s.ws, i)).collect()));
                    o.insert("points".into(), json!(points));
                    o.insert("counts".into(), counts);
                }
                v
            })
            .collect();
    let backlog: Vec<&Issue> = order
        .iter()
        .copied()
        .filter(|i| i.sprint.is_none() && s.ws.status_category(i) != StatusCategory::Done && !i.issue_type.eq_ignore_ascii_case("Epic"))
        .collect();
    let (points, counts) = view::tally(&s.ws, backlog.iter().copied());
    Ok(json!({
        "project": key.as_str(),
        "sprints": sprints,
        "backlog": backlog.iter().map(|i| view::planning_row(&s.ws, i)).collect::<Vec<_>>(),
        "backlogPoints": points,
        "backlogCounts": counts,
    }))
}

fn sprint_list(s: &mut Session, p: &Value) -> CmdResult {
    let key = s.ws.project(params::str(p, "project")?).map_err(err)?.key.clone();
    Ok(Value::Array(s.ws.project_sprints(&key).into_iter().map(|sp| view::sprint(&s.ws, sp)).collect()))
}

/// `before` / `after`: at most one; `None` when neither is given.
fn anchor(p: &Value) -> Result<Option<Anchor<'_>>, String> {
    match (params::opt_str(p, "before")?, params::opt_str(p, "after")?) {
        (Some(_), Some(_)) => Err("pass \"before\" or \"after\", not both".into()),
        (Some(b), None) => Ok(Some(Anchor::Before(b))),
        (None, Some(a)) => Ok(Some(Anchor::After(a))),
        (None, None) => Ok(None),
    }
}

fn key_list(keys: &[IssueKey]) -> Vec<String> {
    keys.iter().map(ToString::to_string).collect()
}

fn issue_rank(s: &mut Session, p: &Value) -> CmdResult {
    let keys = params::keys(p, "keys")?;
    let a = anchor(p)?.ok_or("missing \"before\" or \"after\" (the issue to rank next to)")?;
    let moved = s.ws.rank_issues(&keys, a).map_err(err)?;
    Ok(json!({"keys": key_list(&moved)}))
}

fn issue_set_sprint(s: &mut Session, p: &Value) -> CmdResult {
    let keys = params::keys(p, "keys")?;
    if p.get("sprint").is_none() {
        return Err("missing \"sprint\" (a sprint id, or null for the backlog)".into());
    }
    let sprint: Option<Id> = params::opt_id(p, "sprint").map_err(|_| "\"sprint\" must be a sprint id or null".to_string())?;
    let a = anchor(p)?;
    let (now, me) = (s.now(), s.user.clone());
    let moved = s.ws.set_sprint(&keys, sprint, &me, now).map_err(err)?;
    if let Some(a) = a {
        s.ws.rank_issues(&keys, a).map_err(err)?;
    }
    Ok(json!({"keys": key_list(&moved), "sprint": sprint.map(|i| i.0)}))
}

fn sprint_id(p: &Value) -> Result<Id, String> {
    params::id(p, "id").map_err(|e| if e.starts_with("missing") { e } else { "\"id\" must be a sprint id".into() })
}

fn sprint_create(s: &mut Session, p: &Value) -> CmdResult {
    let (start, end) = (params::opt_date(p, "startDate")?, params::opt_date(p, "endDate")?);
    let id = s.ws.create_sprint(params::str(p, "project")?, params::opt_str(p, "name")?, params::opt_str(p, "goal")?, start, end).map_err(err)?;
    let sp = s.ws.sprint(id).map_err(err)?;
    Ok(view::sprint(&s.ws, sp))
}

fn sprint_edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = sprint_id(p)?;
    let edit = SprintEdit {
        name: params::opt_str(p, "name")?.map(str::to_string),
        goal: params::opt_str(p, "goal")?.map(str::to_string),
        start: clearable_date(p, "startDate")?,
        end: clearable_date(p, "endDate")?,
    };
    if edit.name.is_none() && edit.goal.is_none() && edit.start.is_none() && edit.end.is_none() {
        return Err("nothing to change: pass name, goal, startDate or endDate".into());
    }
    s.ws.edit_sprint(id, edit).map_err(err)?;
    let sp = s.ws.sprint(id).map_err(err)?;
    Ok(view::sprint(&s.ws, sp))
}

/// Absent = keep (`None`), `null` = clear (`Some(None)`), a date = set.
fn clearable_date(p: &Value, k: &str) -> Result<Option<Option<trackcraft_model::Timestamp>>, String> {
    match p.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(_) => params::opt_date(p, k).map(Some),
    }
}

fn sprint_delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = sprint_id(p)?;
    let (now, me) = (s.now(), s.user.clone());
    let back = s.ws.delete_sprint(id, &me, now).map_err(err)?;
    Ok(json!({"deleted": id.0, "backlog": key_list(&back)}))
}

fn sprint_start(s: &mut Session, p: &Value) -> CmdResult {
    let id = sprint_id(p)?;
    let start = params::date(p, "startDate")?;
    let end = params::date(p, "endDate")?;
    s.ws.start_sprint(id, params::opt_str(p, "name")?, start, end, params::opt_str(p, "goal")?).map_err(err)?;
    let sp = s.ws.sprint(id).map_err(err)?;
    Ok(view::sprint(&s.ws, sp))
}

fn sprint_complete(s: &mut Session, p: &Value) -> CmdResult {
    let id = sprint_id(p)?;
    let to = match p.get("moveOpenTo") {
        None | Some(Value::Null) => return Err("missing \"moveOpenTo\" (a sprint id, \"new\" or \"backlog\")".into()),
        Some(Value::String(t)) if t.trim().eq_ignore_ascii_case("backlog") => MoveOpenTo::Backlog,
        Some(Value::String(t)) if t.trim().eq_ignore_ascii_case("new") => MoveOpenTo::New,
        Some(_) => MoveOpenTo::Sprint(params::id(p, "moveOpenTo").map_err(|_| "\"moveOpenTo\" is a sprint id, \"new\" or \"backlog\"".to_string())?),
    };
    let (now, me) = (s.now(), s.user.clone());
    let c = s.ws.complete_sprint(id, to, &me, now).map_err(err)?;
    let sp = s.ws.sprint(id).map_err(err)?;
    Ok(json!({
        "sprint": view::sprint(&s.ws, sp),
        "done": key_list(&c.done),
        "moved": key_list(&c.moved),
        "movedTo": c.to.map(|i| i.0),
    }))
}
