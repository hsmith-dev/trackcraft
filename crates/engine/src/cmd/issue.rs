//! Tracker commands: projects, issues, workflow, comments, links, search and boards.

use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_jql::{Context, matches, parse};
use trackcraft_model::{IssueRecord, LinkKind, NewIssue, ProjectKind};

use crate::{CmdResult, CommandSpec, Session, json as view, params};

/// Most results one search returns (Jira caps `maxResults` too).
pub const MAX_RESULTS: usize = 1000;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("project.create", "Create Project", "Projects", project_create)
            .params(r#"{"key": "LUME", "name": "Lume", "kind": "software"|"business", "lead": "ann"}"#),
        CommandSpec::new("project.list", "All Projects", "Projects", project_list).query(),
        CommandSpec::new("project.get", "Project Details", "Projects", project_get).params(r#"{"key": "LUME"}"#).query(),
        CommandSpec::new("issue.create", "Create Issue", "Issue", issue_create).key("C").params(
            r#"{"project": "LUME", "summary": "…", "type": "Task", "description": "Markdown or ADF", "priority": "High", "assignee": "ann", "labels": ["ui"], "parent": "LUME-1", "storyPoints": 3}"#,
        ),
        CommandSpec::new("issue.get", "View Issue", "Issue", issue_get).params(r#"{"key": "LUME-1"}"#).query(),
        CommandSpec::new("issue.history", "Change History", "Issue › Activity", issue_history).params(r#"{"key": "LUME-1"}"#).query(),
        CommandSpec::new("issue.edit", "Edit Issue", "Issue", issue_edit)
            .key("E")
            .params(r#"{"key": "LUME-1", "summary": "…", "description": "…", "priority": "Low", "labels": ["a"], "storyPoints": 5 (each optional)}"#),
        CommandSpec::new("issue.assign", "Assign", "Issue", issue_assign).key("A").params(r#"{"key": "LUME-1", "assignee": "ann" | null}"#),
        CommandSpec::new("issue.assignToMe", "Assign to Me", "Issue", issue_assign_me).key("I").params(r#"{"key": "LUME-1"}"#),
        CommandSpec::new("issue.transitions", "Available Transitions", "Issue › Workflow", issue_transitions).params(r#"{"key": "LUME-1"}"#).query(),
        CommandSpec::new("issue.transition", "Transition", "Issue › Workflow", issue_transition).params(r#"{"key": "LUME-1", "to": "In Progress"}"#),
        CommandSpec::new("issue.comment", "Add Comment", "Issue › Activity", issue_comment).key("M").params(r#"{"key": "LUME-1", "body": "Markdown or ADF"}"#),
        CommandSpec::new("issue.deleteComment", "Delete Comment", "Issue › Activity", issue_delete_comment).params(r#"{"key": "LUME-1", "id": 7}"#),
        CommandSpec::new("issue.link", "Link Issue", "Issue › Links", issue_link)
            .params(r#"{"key": "LUME-1", "type": "blocks"|"relates to"|"duplicates"|"clones", "target": "LUME-2"}"#),
        CommandSpec::new("issue.delete", "Delete Issue", "Issue", issue_delete).params(r#"{"key": "LUME-1"}"#),
        CommandSpec::new("search.jql", "Search Issues (JQL)", "Filters", search_jql)
            .key("/")
            .params(r#"{"jql": "project = LUME AND status != Done ORDER BY priority", "startAt": 0, "maxResults": 50}"#)
            .query(),
        CommandSpec::new("board.view", "Board", "Project › Board", board_view).params(r#"{"project": "LUME", "jql": "assignee = currentUser() (optional filter)"}"#).query(),
    ]
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn project_create(s: &mut Session, p: &Value) -> CmdResult {
    let kind = match params::opt_str(p, "kind")?.map(str::to_ascii_lowercase).as_deref() {
        None | Some("software") => ProjectKind::Software,
        Some("business") => ProjectKind::Business,
        Some(_) => return Err("\"kind\" is \"software\" or \"business\"".into()),
    };
    let lead = params::opt_str(p, "lead")?;
    let key = s.ws.create_project(params::str(p, "key")?, params::str(p, "name")?, kind, lead).map_err(err)?;
    Ok(json!({"key": key.as_str()}))
}

fn project_list(s: &mut Session, _: &Value) -> CmdResult {
    let count = |k| s.ws.issues.keys().filter(|i| &i.project == k).count();
    Ok(Value::Array(s.ws.projects.values().map(|p| view::project(p, count(&p.key))).collect()))
}

fn project_get(s: &mut Session, p: &Value) -> CmdResult {
    let proj = s.ws.project(params::str(p, "key")?).map_err(err)?;
    let n = s.ws.issues.keys().filter(|i| i.project == proj.key).count();
    Ok(view::project(proj, n))
}

fn issue_create(s: &mut Session, p: &Value) -> CmdResult {
    let new = NewIssue {
        project: params::str(p, "project")?.to_string(),
        issue_type: params::opt_str(p, "type")?.or(params::opt_str(p, "issuetype")?).map(str::to_string),
        summary: params::str(p, "summary")?.to_string(),
        description: params::opt_doc(p, "description")?.unwrap_or_default(),
        priority: params::opt_str(p, "priority")?.map(str::to_string),
        assignee: params::opt_str(p, "assignee")?.map(str::to_string),
        labels: params::strings(p, "labels")?.unwrap_or_default(),
        parent: params::opt_str(p, "parent")?.map(str::to_string),
        story_points: params::opt_f64(p, "storyPoints")?,
    };
    let now = s.now();
    let user = s.user.clone();
    let key = s.ws.create_issue(new, &user, now).map_err(err)?;
    Ok(json!({"key": key.to_string()}))
}

fn issue_get(s: &mut Session, p: &Value) -> CmdResult {
    let i = s.ws.issue(params::str(p, "key")?).map_err(err)?;
    Ok(view::issue(&s.ws, i))
}

/// The issue's activity, newest first, one entry per command (Jira changelog shape).
fn issue_history(s: &mut Session, p: &Value) -> CmdResult {
    let i = s.ws.issue(params::str(p, "key")?).map_err(err)?;
    Ok(Value::Array(view::history(i)))
}

fn issue_edit(s: &mut Session, p: &Value) -> CmdResult {
    let key = params::str(p, "key")?;
    s.ws.issue(key).map_err(err)?;
    let (now, me) = (s.now(), s.user.clone());
    let mut changed = Vec::new();
    if let Some(v) = params::opt_str(p, "summary")? {
        s.ws.set_summary(key, v, &me, now).map_err(err)?;
        changed.push("summary");
    }
    if let Some(d) = params::opt_doc(p, "description")? {
        s.ws.set_description(key, d, &me, now).map_err(err)?;
        changed.push("description");
    }
    if let Some(v) = params::opt_str(p, "priority")? {
        s.ws.set_priority(key, v, &me, now).map_err(err)?;
        changed.push("priority");
    }
    if let Some(l) = params::strings(p, "labels")? {
        s.ws.set_labels(key, &l, &me, now).map_err(err)?;
        changed.push("labels");
    }
    if p.get("storyPoints").is_some() {
        s.ws.set_story_points(key, params::opt_f64(p, "storyPoints")?, &me, now).map_err(err)?;
        changed.push("storyPoints");
    }
    if changed.is_empty() {
        return Err("nothing to change: pass summary, description, priority, labels or storyPoints".into());
    }
    Ok(json!({"key": key.trim().to_ascii_uppercase(), "changed": changed}))
}

fn issue_assign(s: &mut Session, p: &Value) -> CmdResult {
    let key = params::str(p, "key")?;
    if p.get("assignee").is_none() {
        return Err("missing \"assignee\" (a user id, or null to unassign)".into());
    }
    let who = params::opt_str(p, "assignee")?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.assign(key, who, &me, now).map_err(err)?;
    Ok(json!({"key": key.trim().to_ascii_uppercase(), "assignee": who}))
}

fn issue_assign_me(s: &mut Session, p: &Value) -> CmdResult {
    let key = params::str(p, "key")?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.assign(key, Some(&me), &me, now).map_err(err)?;
    Ok(json!({"key": key.trim().to_ascii_uppercase(), "assignee": me}))
}

fn issue_transitions(s: &mut Session, p: &Value) -> CmdResult {
    let i = s.ws.issue(params::str(p, "key")?).map_err(err)?;
    let proj = s.ws.project(i.key.project.as_str()).map_err(err)?;
    Ok(Value::Array(proj.workflow.available(&i.status).iter().map(|t| json!({"name": t.name, "to": t.to})).collect()))
}

fn issue_transition(s: &mut Session, p: &Value) -> CmdResult {
    let key = params::str(p, "key")?;
    let to = params::str(p, "to")?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.transition(key, to, &me, now).map_err(err)?;
    let i = s.ws.issue(key).map_err(err)?;
    Ok(json!({"key": i.key.to_string(), "status": i.status}))
}

fn issue_comment(s: &mut Session, p: &Value) -> CmdResult {
    let key = params::str(p, "key")?;
    let body = params::opt_doc(p, "body")?.ok_or("missing \"body\"")?;
    let (now, me) = (s.now(), s.user.clone());
    let id = s.ws.add_comment(key, &me, body, now).map_err(err)?;
    Ok(json!({"id": id.0}))
}

fn issue_delete_comment(s: &mut Session, p: &Value) -> CmdResult {
    let key = params::str(p, "key")?;
    let id: Id = params::id(p, "id")?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.delete_comment(key, id, &me, now).map_err(err)?;
    Ok(json!({"deleted": id.0}))
}

fn issue_link(s: &mut Session, p: &Value) -> CmdResult {
    let ty = params::str(p, "type")?;
    let kind = LinkKind::parse(ty).ok_or("\"type\" is one of: blocks, relates to, duplicates, clones")?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.link(params::str(p, "key")?, kind, params::str(p, "target")?, &me, now).map_err(err)?;
    Ok(json!({"linked": true}))
}

fn issue_delete(s: &mut Session, p: &Value) -> CmdResult {
    let gone = s.ws.delete_issue(params::str(p, "key")?).map_err(err)?;
    Ok(json!({"deleted": gone.iter().map(ToString::to_string).collect::<Vec<_>>()}))
}

/// Runs a JQL query and returns matching keys in result order.
fn search(s: &Session, jql: &str) -> Result<Vec<trackcraft_core::IssueKey>, String> {
    let q = parse(jql).map_err(|e| format!("JQL: {e}"))?;
    let cx = Context { current_user: Some(s.user.clone()), now: s.now() };
    let mut hits: Vec<IssueRecord<'_>> = Vec::new();
    for issue in s.ws.issues.values() {
        let r = IssueRecord { ws: &s.ws, issue };
        if matches(&q, &r, &cx).map_err(|e| format!("JQL: {e}"))? {
            hits.push(r);
        }
    }
    if q.order.is_empty() {
        // Jira's default for unordered queries: newest first.
        hits.sort_by(|a, b| b.issue.created.cmp(&a.issue.created).then_with(|| b.issue.key.number.cmp(&a.issue.key.number)));
    } else {
        trackcraft_jql::sort(&q, &mut hits);
    }
    Ok(hits.into_iter().map(|r| r.issue.key.clone()).collect())
}

fn search_jql(s: &mut Session, p: &Value) -> CmdResult {
    let jql = params::opt_str(p, "jql")?.unwrap_or("");
    let start = params::usize_or(p, "startAt", 0, usize::MAX)?;
    let max = params::usize_or(p, "maxResults", 50, MAX_RESULTS)?;
    let keys = search(s, jql)?;
    let issues: Vec<Value> = keys.iter().skip(start).take(max).filter_map(|k| s.ws.issues.get(k)).map(|i| view::issue_brief(&s.ws, i)).collect();
    Ok(json!({"jql": jql, "startAt": start, "maxResults": max, "total": keys.len(), "issues": issues}))
}

fn board_view(s: &mut Session, p: &Value) -> CmdResult {
    let proj = s.ws.project(params::str(p, "project")?).map_err(err)?;
    let key = proj.key.clone();
    let extra = params::opt_str(p, "jql")?.filter(|q| !q.trim().is_empty());
    let jql = match extra {
        Some(q) => format!("project = {key} AND ({q}) ORDER BY priority, key"),
        None => format!("project = {key} ORDER BY priority, key"),
    };
    let keys = search(s, &jql)?;
    let proj = s.ws.project(key.as_str()).map_err(err)?;
    let columns: Vec<Value> = proj
        .workflow
        .statuses
        .iter()
        .map(|st| {
            let cards: Vec<Value> = keys
                .iter()
                .filter_map(|k| s.ws.issues.get(k))
                .filter(|i| i.status.eq_ignore_ascii_case(&st.name))
                .map(|i| view::issue_brief(&s.ws, i))
                .collect();
            json!({"status": st.name, "statusCategory": st.category.name(), "issues": cards})
        })
        .collect();
    Ok(json!({"project": key.as_str(), "columns": columns}))
}
