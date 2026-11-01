//! Workspace, file, user and undo commands.

use serde_json::{Value, json};

use crate::{CmdResult, CommandSpec, Session, params};
use trackcraft_model::Workspace;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("workspace.new", "New Workspace", "File", ws_new).params(r#"{"name": "Acme"}"#).query(),
        CommandSpec::new("workspace.info", "Workspace Info", "File", ws_info).query(),
        CommandSpec::new("file.open", "Open…", "File", file_open).key("Mod+O").params(r#"{"path": "work.track"}"#).query(),
        CommandSpec::new("file.save", "Save", "File", file_save)
            .key("Mod+S")
            .params(r#"{"path": "work.track" (optional after the first save)}"#)
            .query(),
        CommandSpec::new("edit.undo", "Undo", "Edit", undo).key("Mod+Z").when(|s| (!s.can_undo()).then_some("nothing to undo")).query(),
        CommandSpec::new("edit.redo", "Redo", "Edit", redo).key("Mod+Shift+Z").when(|s| (!s.can_redo()).then_some("nothing to redo")).query(),
        CommandSpec::new("user.add", "Add User", "Admin › Users", user_add).params(r#"{"id": "ann", "name": "Ann Lee"}"#),
        CommandSpec::new("user.list", "Users", "Admin › Users", user_list).query(),
        CommandSpec::new("user.switch", "Act As User", "Admin › Users", user_switch).params(r#"{"id": "ann"}"#).query(),
        CommandSpec::new("app.commands", "Command List", "Help", app_commands).key("Mod+Shift+P").query(),
    ]
}

fn ws_new(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::opt_str(p, "name")?.unwrap_or("Untitled workspace");
    let name = trackcraft_model::title(name, "the workspace name").map_err(|e| e.to_string())?;
    let mut ws = Workspace::new(&name);
    // Keep the acting user so the new workspace is usable right away.
    if let Ok(u) = s.ws.user(&s.user) {
        let (id, display) = (u.id.clone(), u.display_name.clone());
        ws.add_user(&id, &display).map_err(|e| e.to_string())?;
    }
    s.ws = ws;
    s.path = None;
    s.reset_history();
    Ok(json!({"name": name}))
}

fn ws_info(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({
        "name": s.ws.name,
        "path": s.path.as_ref().map(|p| p.display().to_string()),
        "dirty": s.dirty,
        "user": s.user,
        "projects": s.ws.projects.len(),
        "issues": s.ws.issues.len(),
        "spaces": s.ws.spaces.len(),
        "pages": s.ws.pages.len(),
        "users": s.ws.users.len(),
    }))
}

fn file_open(s: &mut Session, p: &Value) -> CmdResult {
    let path = std::path::PathBuf::from(params::str(p, "path")?);
    let ws = trackcraft_store::load(&path).map_err(|e| e.to_string())?;
    if ws.user(&s.user).is_err() {
        s.user = ws.users.keys().next().cloned().unwrap_or_default();
    }
    s.ws = ws;
    s.path = Some(path);
    s.reset_history();
    ws_info(s, &Value::Null)
}

fn file_save(s: &mut Session, p: &Value) -> CmdResult {
    let path = match params::opt_str(p, "path")? {
        Some(path) => std::path::PathBuf::from(path),
        None => s.path.clone().ok_or("no path yet: pass {\"path\": …}")?,
    };
    trackcraft_store::save(&s.ws, &path).map_err(|e| e.to_string())?;
    s.path = Some(path.clone());
    s.dirty = false;
    Ok(json!({"path": path.display().to_string()}))
}

fn undo(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({"undone": s.undo()}))
}

fn redo(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!({"redone": s.redo()}))
}

fn user_add(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::str(p, "id")?;
    let name = params::opt_str(p, "name")?.unwrap_or(id);
    s.ws.add_user(id, name).map_err(|e| e.to_string())?;
    Ok(json!({"id": id.trim()}))
}

fn user_list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(Value::Array(s.ws.users.values().map(|u| json!({"id": u.id, "name": u.display_name})).collect()))
}

fn user_switch(s: &mut Session, p: &Value) -> CmdResult {
    let id = s.ws.user(params::str(p, "id")?).map_err(|e| e.to_string())?.id.clone();
    s.user = id.clone();
    Ok(json!({"user": id}))
}

fn app_commands(s: &mut Session, _: &Value) -> CmdResult {
    Ok(Value::Array(
        crate::commands()
            .iter()
            .map(|c| {
                json!({
                    "id": c.id, "label": c.label, "location": c.location, "shortcut": c.shortcut,
                    "params": c.params, "mutates": c.mutates, "disabled": (c.enabled)(s),
                })
            })
            .collect(),
    ))
}
