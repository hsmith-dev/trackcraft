//! Page templates: the built-in ones plus templates saved for the workspace or a space.

use serde_json::{Value, json};
use trackcraft_core::Id;
use trackcraft_model::{BUILTIN_TEMPLATES, NewTemplate};

use super::err;
use crate::{CmdResult, CommandSpec, Session, params};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("template.list", "Page Templates", "Space › Templates", template_list)
            .params(r#"{"space": "ENG" (optional; built-in, workspace and that space's templates — all saved ones when omitted)}"#)
            .query(),
        CommandSpec::new("template.create", "Save Template", "Space › Templates", template_create).params(
            r#"{"space": "ENG" (optional; default every space), "name": "Incident review", "title": "Incident {date}" (optional), "body": "Markdown or ADF", "description": "…" (optional)} ({date} becomes the creation day)"#,
        ),
        CommandSpec::new("template.delete", "Delete Template", "Space › Templates", template_delete).params(r#"{"id": 31} (saved templates only)"#),
    ]
}

fn template_list(s: &mut Session, p: &Value) -> CmdResult {
    let space = match params::opt_str(p, "space")? {
        Some(k) => Some(s.ws.space(k).map_err(err)?.key.clone()),
        None => None,
    };
    let mut out: Vec<Value> = BUILTIN_TEMPLATES
        .iter()
        .map(|t| json!({"id": t.key, "name": t.name, "description": t.description, "title": t.title, "body": t.body, "builtin": true, "space": null}))
        .collect();
    out.extend(s.ws.templates.values().filter(|t| space.is_none() || t.space.is_none() || t.space == space).map(|t| {
        json!({
            "id": t.id.0, "name": t.name, "description": t.description, "title": t.title, "body": t.body.to_markdown(),
            "builtin": false, "space": t.space.as_ref().map(|k| k.as_str()), "author": t.author, "created": t.created.to_rfc3339(),
        })
    }));
    Ok(Value::Array(out))
}

fn template_create(s: &mut Session, p: &Value) -> CmdResult {
    let new = NewTemplate {
        space: params::opt_str(p, "space")?.map(str::to_string),
        name: params::str(p, "name")?.to_string(),
        description: params::opt_str(p, "description")?.unwrap_or_default().to_string(),
        title: params::opt_str(p, "title")?.unwrap_or_default().to_string(),
        body: params::opt_doc(p, "body")?.ok_or("missing \"body\"")?,
    };
    let (now, me) = (s.now(), s.user.clone());
    let id = s.ws.create_template(new, &me, now).map_err(err)?;
    Ok(json!({"id": id.0}))
}

fn template_delete(s: &mut Session, p: &Value) -> CmdResult {
    if let Some(Value::String(k)) = p.get("id")
        && trackcraft_model::builtin_template(k).is_some()
    {
        return Err("built-in templates can't be deleted".into());
    }
    let id: Id = params::id(p, "id")?;
    s.ws.delete_template(id).map_err(err)?;
    Ok(json!({"deleted": id.0}))
}
