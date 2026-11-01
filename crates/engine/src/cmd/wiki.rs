//! Wiki commands: spaces, pages, the page tree and version history.

use serde_json::{Value, json};
use trackcraft_model::{NewPage, PageEdit};

use crate::{CmdResult, CommandSpec, Session, json as view, params};

/// Most pages one search returns.
const MAX_PAGE_RESULTS: usize = 1000;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("space.create", "Create Space", "Spaces", space_create).params(r#"{"key": "ENG", "name": "Engineering"}"#),
        CommandSpec::new("space.list", "All Spaces", "Spaces", space_list).query(),
        CommandSpec::new("space.tree", "Page Tree", "Space › Sidebar", space_tree).params(r#"{"key": "ENG"}"#).query(),
        CommandSpec::new("page.create", "Create Page", "Page", page_create)
            .key("C")
            .params(r#"{"space": "ENG", "title": "…", "parent": 12 (optional; default the space home), "body": "text or ADF"}"#),
        CommandSpec::new("page.get", "View Page", "Page", page_get).params(r#"{"id": 12}"#).query(),
        CommandSpec::new("page.edit", "Edit Page", "Page", page_edit)
            .key("E")
            .params(r#"{"id": 12, "title": "…", "body": "…", "baseVersion": 3, "message": "what changed" (each optional but one of title/body)}"#),
        CommandSpec::new("page.move", "Move Page", "Page › More", page_move).params(r#"{"id": 12, "parent": 4}"#),
        CommandSpec::new("page.delete", "Delete Page", "Page › More", page_delete).params(r#"{"id": 12}"#),
        CommandSpec::new("page.labels", "Edit Labels", "Page", page_labels).key("L").params(r#"{"id": 12, "labels": ["howto"]}"#),
        CommandSpec::new("page.history", "Page History", "Page › More", page_history).params(r#"{"id": 12}"#).query(),
        CommandSpec::new("page.restore", "Restore Version", "Page › History", page_restore).params(r#"{"id": 12, "version": 2}"#),
        CommandSpec::new("page.search", "Search Pages", "Search", page_search).params(r#"{"text": "storage", "space": "ENG" (optional)}"#).query(),
    ]
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn space_create(s: &mut Session, p: &Value) -> CmdResult {
    let (now, me) = (s.now(), s.user.clone());
    let key = s.ws.create_space(params::str(p, "key")?, params::str(p, "name")?, &me, now).map_err(err)?;
    let sp = s.ws.space(key.as_str()).map_err(err)?;
    Ok(view::space(sp))
}

fn space_list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(Value::Array(s.ws.spaces.values().map(view::space).collect()))
}

fn space_tree(s: &mut Session, p: &Value) -> CmdResult {
    let sp = s.ws.space(params::str(p, "key")?).map_err(err)?;
    Ok(view::tree(&s.ws, sp.homepage, 0))
}

fn page_create(s: &mut Session, p: &Value) -> CmdResult {
    let new = NewPage {
        space: params::str(p, "space")?.to_string(),
        parent: params::opt_id(p, "parent")?,
        title: params::str(p, "title")?.to_string(),
        body: params::opt_doc(p, "body")?.unwrap_or_default(),
    };
    let (now, me) = (s.now(), s.user.clone());
    let id = s.ws.create_page(new, &me, now).map_err(err)?;
    Ok(json!({"id": id.0}))
}

fn page_get(s: &mut Session, p: &Value) -> CmdResult {
    let page = s.ws.page(params::id(p, "id")?).map_err(err)?;
    let mut v = view::page(page);
    if let Some(o) = v.as_object_mut() {
        o.insert("ancestors".into(), json!(s.ws.ancestors(page.id).iter().rev().map(|a| a.0).collect::<Vec<_>>()));
    }
    Ok(v)
}

fn page_edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let edit = PageEdit {
        title: params::opt_str(p, "title")?.map(str::to_string),
        body: params::opt_doc(p, "body")?,
        base_version: params::opt_u32(p, "baseVersion")?,
        message: params::opt_str(p, "message")?.unwrap_or_default().to_string(),
    };
    if edit.title.is_none() && edit.body.is_none() {
        return Err("nothing to change: pass title and/or body".into());
    }
    let (now, me) = (s.now(), s.user.clone());
    let version = s.ws.edit_page(id, edit, &me, now).map_err(err)?;
    Ok(json!({"id": id.0, "version": version}))
}

fn page_move(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    s.ws.move_page(id, params::id(p, "parent")?).map_err(err)?;
    Ok(json!({"moved": id.0}))
}

fn page_delete(s: &mut Session, p: &Value) -> CmdResult {
    let gone = s.ws.delete_page(params::id(p, "id")?).map_err(err)?;
    Ok(json!({"deleted": gone.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

fn page_labels(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let labels = params::strings(p, "labels")?.ok_or("missing \"labels\"")?;
    s.ws.set_page_labels(id, &labels).map_err(err)?;
    Ok(json!({"id": id.0}))
}

fn page_history(s: &mut Session, p: &Value) -> CmdResult {
    let page = s.ws.page(params::id(p, "id")?).map_err(err)?;
    let mut out: Vec<Value> = page
        .history
        .iter()
        .map(|v| json!({"number": v.version, "title": v.title, "by": v.author, "when": v.at.to_rfc3339(), "message": v.message}))
        .collect();
    out.push(json!({"number": page.version, "title": page.title, "by": page.author, "when": page.updated.to_rfc3339(), "message": page.message, "current": true}));
    out.reverse();
    Ok(Value::Array(out))
}

fn page_restore(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let version = params::opt_u32(p, "version")?.ok_or("missing \"version\"")?;
    let (now, me) = (s.now(), s.user.clone());
    let v = s.ws.restore_page(id, version, &me, now).map_err(err)?;
    Ok(json!({"id": id.0, "version": v}))
}

fn page_search(s: &mut Session, p: &Value) -> CmdResult {
    let text = params::str(p, "text")?.to_lowercase();
    let space = params::opt_str(p, "space")?.map(str::to_ascii_uppercase);
    let words: Vec<&str> = text.split_whitespace().collect();
    let hits: Vec<Value> =
        s.ws.pages
            .values()
            .filter(|pg| space.as_deref().is_none_or(|k| pg.space.as_str() == k))
            .filter(|pg| {
                let hay = format!("{}\n{}", pg.title, pg.body.plain_text()).to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .take(MAX_PAGE_RESULTS)
            .map(|pg| json!({"id": pg.id.0, "space": pg.space.as_str(), "title": pg.title}))
            .collect();
    Ok(Value::Array(hits))
}
