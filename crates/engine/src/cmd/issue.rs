//! Tracker commands: projects, issues, workflow, comments, links, search and boards.

use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_jql::{Context, matches, parse};
use trackcraft_model::{IssueRecord, LinkKind, NewIssue, ProjectKind, SprintState};

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
            r#"{"project": "LUME", "summary": "…", "type": "Task", "description": "Markdown or ADF", "priority": "High", "assignee": "ann", "labels": ["ui"], "parent": "LUME-1", "storyPoints": 3, "sprint": 7 (optional: an open sprint of the project)}"#,
        ),
        CommandSpec::new("issue.get", "View Issue", "Issue", issue_get).params(r#"{"key": "LUME-1"}"#).query(),
        CommandSpec::new("issue.history", "Change History", "Issue › Activity", issue_history).params(r#"{"key": "LUME-1"}"#).query(),
        CommandSpec::new("issue.edit", "Edit Issue", "Issue", issue_edit)
            .key("E")
            .params(r#"{"key": "LUME-1", "summary": "…", "description": "…", "priority": "Low", "labels": ["a"], "storyPoints": 5, "type": "Bug", "parent": "LUME-2" | null (each optional; a sub-task needs a parent, an epic has none)}"#),
        CommandSpec::new("issue.assign", "Assign", "Issue", issue_assign).key("A").params(r#"{"key": "LUME-1", "assignee": "ann" | null}"#),
        CommandSpec::new("issue.assignToMe", "Assign to Me", "Issue", issue_assign_me).key("I").params(r#"{"key": "LUME-1"}"#),
        CommandSpec::new("issue.transitions", "Available Transitions", "Issue › Workflow", issue_transitions).params(r#"{"key": "LUME-1"}"#).query(),
        CommandSpec::new("issue.transition", "Transition", "Issue › Workflow", issue_transition).params(r#"{"key": "LUME-1", "to": "In Progress"}"#),
        CommandSpec::new("issue.comment", "Add Comment", "Issue › Activity", issue_comment).key("M").params(r#"{"key": "LUME-1", "body": "Markdown or ADF"}"#),
        CommandSpec::new("issue.editComment", "Edit Comment", "Issue › Activity", issue_edit_comment)
            .params(r#"{"key": "LUME-1", "id": 7, "body": "Markdown or ADF"} (only the comment's author can edit it)"#),
        CommandSpec::new("issue.deleteComment", "Delete Comment", "Issue › Activity", issue_delete_comment).params(r#"{"key": "LUME-1", "id": 7}"#),
        CommandSpec::new("issue.link", "Link Issue", "Issue › Links", issue_link)
            .params(r#"{"key": "LUME-1", "type": "blocks"|"relates to"|"duplicates"|"clones", "target": "LUME-2"}"#),
        CommandSpec::new("issue.unlink", "Remove Link", "Issue › Links", issue_unlink)
            .params(r#"{"key": "LUME-1", "type": "blocks"|"relates to"|"duplicates"|"clones", "target": "LUME-2"} (either side of the link)"#),
        CommandSpec::new("issue.delete", "Delete Issue", "Issue", issue_delete).params(r#"{"key": "LUME-1"}"#),
        CommandSpec::new("search.jql", "Search Issues (JQL)", "Filters", search_jql)
            .key("/")
            .params(r#"{"jql": "project = LUME AND status != Done ORDER BY priority", "startAt": 0, "maxResults": 50}"#)
            .query(),
        CommandSpec::new("board.view", "Board", "Project › Board", board_view)
            .params(r#"{"project": "LUME", "jql": "assignee = currentUser() (optional filter)", "sprint": "active" | 7 (optional: only that sprint's issues)} → columns of cards in rank order"#)
            .query(),
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
    // The creator administers what they made (workspace admins already administer everything).
    let me = s.user.clone();
    if s.ws.workspace_role(&me).is_some_and(|r| r != trackcraft_model::WorkspaceRole::Admin) {
        s.ws.set_project_role(key.as_str(), &me, Some(trackcraft_model::ProjectRole::Administrator)).map_err(err)?;
    }
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
    let sprint = params::opt_id(p, "sprint").map_err(|_| "\"sprint\" must be a sprint id".to_string())?;
    let now = s.now();
    let user = s.user.clone();
    let key = s.ws.create_issue(new, &user, now).map_err(err)?;
    // Inline create in a sprint: one command, so one undo step.
    if let Some(id) = sprint {
        let i = s.ws.issue(&key.to_string()).map_err(err)?;
        if s.ws.is_subtask(i) {
            if i.sprint != Some(id) {
                return Err(format!("{key} is a sub-task: it follows its parent's sprint"));
            }
        } else {
            s.ws.set_sprint(&[key.to_string()], Some(id), &user, now).map_err(err)?;
        }
    }
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
    let ty = params::opt_str(p, "type")?.or(params::opt_str(p, "issuetype")?);
    let parent = match p.get("parent") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(k)) => Some(Some(k.as_str())),
        Some(_) => return Err("\"parent\" must be an issue key or null".into()),
    };
    if ty.is_some() || parent.is_some() {
        s.ws.set_type_and_parent(key, ty, parent, &me, now).map_err(err)?;
        if ty.is_some() {
            changed.push("issuetype");
        }
        if parent.is_some() {
            changed.push("parent");
        }
    }
    if changed.is_empty() {
        return Err("nothing to change: pass summary, description, priority, labels, storyPoints, type or parent".into());
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

fn issue_edit_comment(s: &mut Session, p: &Value) -> CmdResult {
    let key = params::str(p, "key")?;
    let id: Id = params::id(p, "id")?;
    let body = params::opt_doc(p, "body")?.ok_or("missing \"body\"")?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.edit_comment(key, id, body, &me, now).map_err(err)?;
    Ok(json!({"id": id.0}))
}

fn link_kind(p: &Value) -> Result<LinkKind, String> {
    let ty = params::str(p, "type")?;
    LinkKind::parse(ty).ok_or_else(|| "\"type\" is one of: blocks, relates to, duplicates, clones".to_string())
}

fn issue_unlink(s: &mut Session, p: &Value) -> CmdResult {
    let kind = link_kind(p)?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.unlink(params::str(p, "key")?, kind, params::str(p, "target")?, &me, now).map_err(err)?;
    Ok(json!({"unlinked": true}))
}

fn issue_link(s: &mut Session, p: &Value) -> CmdResult {
    let kind = link_kind(p)?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.link(params::str(p, "key")?, kind, params::str(p, "target")?, &me, now).map_err(err)?;
    Ok(json!({"linked": true}))
}

fn issue_delete(s: &mut Session, p: &Value) -> CmdResult {
    let (now, me) = (s.now(), s.user.clone());
    let gone = s.ws.delete_issue(params::str(p, "key")?, &me, now).map_err(err)?;
    Ok(json!({"deleted": gone.iter().map(ToString::to_string).collect::<Vec<_>>()}))
}

/// Runs a JQL query and returns matching keys in result order.
pub(crate) fn search(s: &Session, jql: &str) -> Result<Vec<trackcraft_core::IssueKey>, String> {
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
    let sprint = match p.get("sprint") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) if v.trim().eq_ignore_ascii_case("active") => {
            Some(s.ws.active_sprint(&key).ok_or_else(|| format!("project {key} has no active sprint"))?.id)
        }
        Some(_) => {
            let id = params::id(p, "sprint").map_err(|_| "\"sprint\" is \"active\" or a sprint id".to_string())?;
            let sp = s.ws.sprint(id).map_err(err)?;
            if sp.project != key {
                return Err(format!("{} belongs to project {}", sp.name, sp.project));
            }
            Some(id)
        }
    };
    let jql = match extra {
        Some(q) => format!("project = {key} AND ({q}) ORDER BY rank, key"),
        None => format!("project = {key} ORDER BY rank, key"),
    };
    let keys = search(s, &jql)?;
    // A closed sprint's board shows the issues it held when it completed.
    let in_sprint = |i: &&trackcraft_model::Issue| match sprint {
        None => true,
        Some(id) => i.sprint == Some(id) || (i.past_sprints.contains(&id) && s.ws.sprints.get(&id).is_some_and(|sp| sp.state == SprintState::Closed)),
    };
    let proj = s.ws.project(key.as_str()).map_err(err)?;
    let columns: Vec<Value> = proj
        .workflow
        .statuses
        .iter()
        .map(|st| {
            let cards: Vec<Value> = keys
                .iter()
                .filter_map(|k| s.ws.issues.get(k))
                .filter(in_sprint)
                .filter(|i| i.status.eq_ignore_ascii_case(&st.name))
                .map(|i| view::issue_brief(&s.ws, i))
                .collect();
            json!({"status": st.name, "statusCategory": st.category.name(), "issues": cards})
        })
        .collect();
    let sprint = sprint.and_then(|id| s.ws.sprints.get(&id)).map(|sp| view::sprint(&s.ws, sp));
    Ok(json!({"project": key.as_str(), "sprint": sprint, "columns": columns}))
}
