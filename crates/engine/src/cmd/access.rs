//! Permission commands (M4.3): read roles and restrictions, change them, and ask whether a
//! command would be allowed. Rules: `docs/permissions.md`, `crate::authz`.

use serde_json::{Map, Value, json};
use trackcraft_model::{AccessLevel, Grants, ProjectRole, SpaceRole, WorkspaceRole};

use crate::{CmdResult, CommandSpec, Session, params};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("access.get", "Permissions", "Admin › Permissions", access_get)
            .params(r#"{} → your role, every workspace role, project and space access levels and roles, page restrictions (what you can see)"#)
            .query(),
        CommandSpec::new("access.check", "Check Permission", "Admin › Permissions", access_check)
            .params(r#"{"command": "issue.edit", "params": {"key": "LUME-1"}, "user": "sam" (optional; default you)} → {"allowed": true|false, "reason"}"#)
            .query(),
        CommandSpec::new("access.setWorkspaceRole", "Set Workspace Role", "Admin › Permissions", set_workspace_role)
            .params(r#"{"user": "sam", "role": "admin"|"member"|"viewer"} (workspace admins; the last admin can't be demoted)"#),
        CommandSpec::new("access.setProjectRole", "Set Project Role", "Project › Settings › Access", set_project_role)
            .params(r#"{"project": "LUME", "user": "sam", "role": "administrator"|"member"|"viewer"|null (null: back to the project's access level)}"#),
        CommandSpec::new("access.setProjectAccess", "Project Access", "Project › Settings › Access", set_project_access)
            .params(r#"{"project": "LUME", "access": "open"|"limited"|"private"} (what members without a project role get: member, viewer, nothing)"#),
        CommandSpec::new("access.setSpaceRole", "Set Space Role", "Space › Settings › Permissions", set_space_role)
            .params(r#"{"space": "ENG", "user": "sam", "role": "admin"|"editor"|"viewer"|null}"#),
        CommandSpec::new("access.setSpaceAccess", "Space Access", "Space › Settings › Permissions", set_space_access)
            .params(r#"{"space": "ENG", "access": "open"|"limited"|"private"} (what members without a space role get: editor, viewer, nothing)"#),
        CommandSpec::new("page.restrict", "Restrictions", "Page › More", page_restrict).params(
            r#"{"id": 12, "view": ["ann", "sam"] | null, "edit": ["ann"] | null (each optional; null removes it; view restrictions apply to child pages too; keep yourself on the lists unless you administer the space)}"#,
        ),
    ]
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn grants<R: Copy>(g: &Grants<R>, name: fn(R) -> &'static str) -> Value {
    let roles: Map<String, Value> = g.roles.iter().map(|(u, r)| (u.clone(), json!(name(*r)))).collect();
    json!({"access": g.access.name(), "roles": roles})
}

fn access_get(s: &mut Session, _: &Value) -> CmdResult {
    let ws = &s.ws;
    let me = s.user.clone();
    let users: Vec<Value> = ws.users.keys().map(|u| json!({"user": u, "role": ws.workspace_role(u).map(WorkspaceRole::name)})).collect();
    let projects: Vec<Value> = ws
        .projects
        .keys()
        .map(|k| {
            let mut v = ws
                .access
                .projects
                .get(k)
                .map(|g| grants(g, ProjectRole::name))
                .unwrap_or_else(|| grants(&Grants::<ProjectRole>::default(), ProjectRole::name));
            if let Some(o) = v.as_object_mut() {
                o.insert("key".into(), json!(k.as_str()));
                o.insert("myRole".into(), json!(ws.project_role(&me, k).map(ProjectRole::name)));
            }
            v
        })
        .collect();
    let spaces: Vec<Value> = ws
        .spaces
        .keys()
        .map(|k| {
            let mut v = ws
                .access
                .spaces
                .get(k)
                .map(|g| grants(g, SpaceRole::name))
                .unwrap_or_else(|| grants(&Grants::<SpaceRole>::default(), SpaceRole::name));
            if let Some(o) = v.as_object_mut() {
                o.insert("key".into(), json!(k.as_str()));
                o.insert("myRole".into(), json!(ws.space_role(&me, k).map(SpaceRole::name)));
            }
            v
        })
        .collect();
    let pages: Vec<Value> = ws
        .access
        .pages
        .iter()
        .filter(|(id, _)| ws.pages.contains_key(id))
        .map(|(id, r)| json!({"id": id.0, "view": r.view, "edit": r.edit}))
        .collect();
    Ok(json!({
        "user": me,
        "role": ws.workspace_role(&me).map(WorkspaceRole::name),
        "users": users,
        "projects": projects,
        "spaces": spaces,
        "pages": pages,
    }))
}

fn access_check(s: &mut Session, p: &Value) -> CmdResult {
    let command = params::str(p, "command")?;
    crate::command(command).ok_or_else(|| format!("unknown command '{}'", command.chars().take(80).collect::<String>()))?;
    let user = params::opt_str(p, "user")?.unwrap_or(&s.user).to_string();
    let args = p.get("params").cloned().unwrap_or_else(|| json!({}));
    Ok(match s.authorize(&user, command, &args) {
        Ok(()) => json!({"allowed": true, "reason": null}),
        Err(d) => json!({"allowed": false, "reason": d.to_string()}),
    })
}

fn role_param<R>(p: &Value, parse: fn(&str) -> Option<R>, allowed: &str) -> Result<Option<R>, String> {
    match p.get("role") {
        None => Err(format!("missing \"role\" ({allowed}, or null)")),
        Some(Value::Null) => Ok(None),
        Some(Value::String(r)) => parse(r).map(Some).ok_or_else(|| format!("\"role\" is one of {allowed}, or null")),
        Some(_) => Err(format!("\"role\" is one of {allowed}, or null")),
    }
}

fn level(p: &Value) -> Result<AccessLevel, String> {
    AccessLevel::parse(params::str(p, "access")?).ok_or_else(|| "\"access\" is open, limited or private".to_string())
}

fn set_workspace_role(s: &mut Session, p: &Value) -> CmdResult {
    let user = params::str(p, "user")?;
    let role = WorkspaceRole::parse(params::str(p, "role")?).ok_or("\"role\" is admin, member or viewer")?;
    s.ws.set_workspace_role(user, role).map_err(err)?;
    Ok(json!({"user": user, "role": role.name()}))
}

fn set_project_role(s: &mut Session, p: &Value) -> CmdResult {
    let (project, user) = (params::str(p, "project")?, params::str(p, "user")?);
    let role = role_param(p, ProjectRole::parse, "administrator, member, viewer")?;
    s.ws.set_project_role(project, user, role).map_err(err)?;
    Ok(json!({"project": project.trim().to_ascii_uppercase(), "user": user, "role": role.map(ProjectRole::name)}))
}

fn set_project_access(s: &mut Session, p: &Value) -> CmdResult {
    let project = params::str(p, "project")?;
    let l = level(p)?;
    s.ws.set_project_access(project, l).map_err(err)?;
    Ok(json!({"project": project.trim().to_ascii_uppercase(), "access": l.name()}))
}

fn set_space_role(s: &mut Session, p: &Value) -> CmdResult {
    let (space, user) = (params::str(p, "space")?, params::str(p, "user")?);
    let role = role_param(p, SpaceRole::parse, "admin, editor, viewer")?;
    s.ws.set_space_role(space, user, role).map_err(err)?;
    Ok(json!({"space": space.trim().to_ascii_uppercase(), "user": user, "role": role.map(SpaceRole::name)}))
}

fn set_space_access(s: &mut Session, p: &Value) -> CmdResult {
    let space = params::str(p, "space")?;
    let l = level(p)?;
    s.ws.set_space_access(space, l).map_err(err)?;
    Ok(json!({"space": space.trim().to_ascii_uppercase(), "access": l.name()}))
}

/// `None` = key absent (unchanged), `Some(None)` = null (remove), `Some(Some(list))`.
fn list_param(p: &Value, k: &str) -> Result<Option<Option<Vec<String>>>, String> {
    match p.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::Array(a)) if a.len() > trackcraft_model::MAX_RESTRICTION_USERS => {
            Err(format!("\"{k}\" lists at most {} users", trackcraft_model::MAX_RESTRICTION_USERS))
        }
        Some(Value::Array(_)) => Ok(Some(params::strings(p, k)?)),
        Some(_) => Err(format!("\"{k}\" must be a list of user ids or null")),
    }
}

fn page_restrict(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let (view, edit) = (list_param(p, "view")?, list_param(p, "edit")?);
    if view.is_none() && edit.is_none() {
        return Err("nothing to change: pass \"view\" and/or \"edit\" (a list of users, or null)".into());
    }
    s.ws.restrict_page(id, view, edit).map_err(err)?;
    let r = s.ws.access.pages.get(&id).cloned().unwrap_or_default();
    Ok(json!({"id": id.0, "view": r.view, "edit": r.edit}))
}
