//! Page comments: footer comments with replies, inline comments anchored to a selection.

use serde_json::{Value, json};
use trackcraft_model::{InlineAnchor, NewPageComment, Page, PageComment, locate_anchor};

use super::err;
use crate::{CmdResult, CommandSpec, Session, params};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("page.comment", "Add Comment", "Page › Comments", page_comment).params(
            r#"{"id": 12, "body": "Markdown or ADF", "parent": 40 (optional; reply to a top comment), "anchor": {"block": 0, "start": 4, "end": 9, "quote": "quick"} (optional; inline comment on characters start..end of top-level block `block`)}"#,
        ),
        CommandSpec::new("page.editComment", "Edit Comment", "Page › Comments", page_edit_comment)
            .params(r#"{"id": 12, "comment": 40, "body": "…"} (your own comments only)"#),
        CommandSpec::new("page.deleteComment", "Delete Comment", "Page › Comments", page_delete_comment)
            .params(r#"{"id": 12, "comment": 40} (your own comments only; replies go with it)"#),
        CommandSpec::new("page.resolveComment", "Resolve Comment", "Page › Comments", page_resolve_comment)
            .params(r#"{"id": 12, "comment": 40, "resolved": true} (inline threads; false reopens)"#),
        CommandSpec::new("page.comments", "Page Comments", "Page › Comments", page_comments)
            .params(r#"{"id": 12, "inline": true (optional; only inline or only footer threads)}"#)
            .query(),
    ]
}

fn opt_bool(p: &Value, k: &str) -> Result<Option<bool>, String> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("\"{k}\" must be true or false")),
    }
}

fn anchor(p: &Value) -> Result<Option<InlineAnchor>, String> {
    let a = match p.get("anchor") {
        None | Some(Value::Null) => return Ok(None),
        Some(a @ Value::Object(_)) => a,
        Some(_) => return Err("\"anchor\" must be {\"block\", \"start\", \"end\", \"quote\"}".into()),
    };
    // `blockPath` ([3]) is accepted for top-level blocks, the only kind anchors address today.
    let block = match (params::opt_u32(a, "block")?, a.get("blockPath")) {
        (Some(b), _) => b,
        (None, Some(Value::Array(path))) => match path.as_slice() {
            [one] => one.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("\"blockPath\" must hold block indexes")?,
            _ => return Err("\"blockPath\" must name one top-level block, such as [3]".into()),
        },
        (None, _) => return Err("missing \"anchor.block\"".into()),
    };
    let start = params::opt_u32(a, "start")?.ok_or("missing \"anchor.start\"")?;
    let end = params::opt_u32(a, "end")?.ok_or("missing \"anchor.end\"")?;
    let quote = params::str(a, "quote").map_err(|e| format!("anchor: {e}"))?.to_string();
    Ok(Some(InlineAnchor { block, start, end, quote, version: 0 }))
}

fn body(p: &Value) -> Result<trackcraft_model::Doc, String> {
    params::opt_doc(p, "body")?.ok_or_else(|| "missing \"body\"".to_string())
}

fn page_comment(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let new = NewPageComment { body: body(p)?, parent: params::opt_id(p, "parent")?, anchor: anchor(p)? };
    let (now, me) = (s.now(), s.user.clone());
    let c = s.ws.add_page_comment(id, new, &me, now).map_err(err)?;
    Ok(json!({"id": id.0, "comment": c.0}))
}

fn page_edit_comment(s: &mut Session, p: &Value) -> CmdResult {
    let (id, c) = (params::id(p, "id")?, params::id(p, "comment")?);
    let b = body(p)?;
    let (now, me) = (s.now(), s.user.clone());
    s.ws.edit_page_comment(id, c, b, &me, now).map_err(err)?;
    Ok(json!({"id": id.0, "comment": c.0}))
}

fn page_delete_comment(s: &mut Session, p: &Value) -> CmdResult {
    let (id, c) = (params::id(p, "id")?, params::id(p, "comment")?);
    let me = s.user.clone();
    let gone = s.ws.delete_page_comment(id, c, &me).map_err(err)?;
    Ok(json!({"id": id.0, "deleted": gone.iter().map(|g| g.0).collect::<Vec<_>>()}))
}

fn page_resolve_comment(s: &mut Session, p: &Value) -> CmdResult {
    let (id, c) = (params::id(p, "id")?, params::id(p, "comment")?);
    let resolved = opt_bool(p, "resolved")?.ok_or("missing \"resolved\" (true or false)")?;
    let me = s.user.clone();
    s.ws.resolve_page_comment(id, c, resolved, &me).map_err(err)?;
    Ok(json!({"id": id.0, "comment": c.0, "resolved": resolved}))
}

fn comment_view(page: &Page, c: &PageComment) -> Value {
    let mut v = json!({
        "id": c.id.0,
        "author": c.author,
        "body": c.body.to_markdown(),
        "created": c.created.to_rfc3339(),
        "edited": c.edited.map(|t| t.to_rfc3339()),
    });
    if let Some(o) = v.as_object_mut()
        && c.parent.is_none()
    {
        o.insert("inline".into(), json!(c.anchor.is_some()));
        o.insert("resolved".into(), json!(c.resolved));
        o.insert("resolvedBy".into(), json!(c.resolved_by));
        if let Some(a) = &c.anchor {
            let at = locate_anchor(&page.body, a);
            o.insert(
                "anchor".into(),
                json!({
                    "block": a.block, "start": a.start, "end": a.end, "quote": a.quote, "version": a.version,
                    "current": at.map(|(b, st, en)| json!({"block": b, "start": st, "end": en})),
                }),
            );
            o.insert("outdated".into(), json!(at.is_none()));
        } else {
            o.insert("outdated".into(), json!(false));
        }
    }
    v
}

fn page_comments(s: &mut Session, p: &Value) -> CmdResult {
    let page = s.ws.page(params::id(p, "id")?).map_err(err)?;
    let only = opt_bool(p, "inline")?;
    let threads: Vec<Value> = page
        .comments
        .iter()
        .filter(|c| c.parent.is_none() && only.is_none_or(|i| i == c.anchor.is_some()))
        .map(|c| {
            let mut v = comment_view(page, c);
            let replies: Vec<Value> = page.comments.iter().filter(|r| r.parent == Some(c.id)).map(|r| comment_view(page, r)).collect();
            if let Some(o) = v.as_object_mut() {
                o.insert("replies".into(), Value::Array(replies));
            }
            v
        })
        .collect();
    Ok(json!({"id": page.id.0, "total": page.comments.len(), "comments": threads}))
}
