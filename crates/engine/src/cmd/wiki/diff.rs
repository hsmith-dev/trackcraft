//! `page.diff`: what changed between two versions of a page, by block and by word.

use serde_json::{Value, json};
use trackcraft_markup::diff::{BlockDiff, Hunk, diff_blocks, diff_words};
use trackcraft_model::Doc;

use super::err;
use crate::{CmdResult, CommandSpec, Session, params};

/// Pages whose two versions together exceed this much Markdown are compared as one replaced
/// body, without block or word detail.
pub const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("page.diff", "Compare Versions", "Page › History", page_diff)
            .params(r#"{"id": 12, "from": 2 (optional; default the version before "to"), "to": 5 (optional; default the current version)}"#)
            .query(),
    ]
}

fn hunks(h: &[Hunk]) -> Value {
    Value::Array(h.iter().map(|h| json!({"kind": h.kind.name(), "text": h.text})).collect())
}

fn blocks_of(d: &Doc) -> Vec<String> {
    d.blocks.iter().map(|b| b.to_markdown()).collect()
}

fn page_diff(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::id(p, "id")?;
    let current = s.ws.page(id).map_err(err)?.version;
    let to = params::opt_u32(p, "to")?.unwrap_or(current);
    let from = match params::opt_u32(p, "from")? {
        Some(f) => f,
        None => to.checked_sub(1).filter(|v| *v >= 1).ok_or("this is the first version: pass \"from\" and \"to\"")?,
    };
    let (from_title, from_body) = s.ws.page_version(id, from).map_err(err)?;
    let (to_title, to_body) = s.ws.page_version(id, to).map_err(err)?;
    let (a, b) = (blocks_of(&from_body), blocks_of(&to_body));
    let size = a.iter().chain(b.iter()).fold(0usize, |n, x| n.saturating_add(x.len()));
    let coarse = size > MAX_DIFF_BYTES;
    let diff = if coarse {
        let (fa, fb) = (a.join("\n\n"), b.join("\n\n"));
        let mut d = Vec::new();
        if fa == fb {
            d.push(BlockDiff::Equal(fa));
        } else {
            if !fa.is_empty() {
                d.push(BlockDiff::Delete(fa));
            }
            if !fb.is_empty() {
                d.push(BlockDiff::Insert(fb));
            }
        }
        d
    } else {
        diff_blocks(&a, &b)
    };
    let (mut ins, mut del, mut chg) = (0u64, 0u64, 0u64);
    let blocks: Vec<Value> = diff
        .iter()
        .map(|d| match d {
            BlockDiff::Equal(t) => json!({"kind": "equal", "text": t}),
            BlockDiff::Insert(t) => {
                ins += 1;
                json!({"kind": "insert", "text": t})
            }
            BlockDiff::Delete(t) => {
                del += 1;
                json!({"kind": "delete", "text": t})
            }
            BlockDiff::Change { from, to, inline } => {
                chg += 1;
                json!({"kind": "change", "from": from, "to": to, "inline": hunks(inline)})
            }
        })
        .collect();
    Ok(json!({
        "id": id.0,
        "from": {"number": from, "title": from_title},
        "to": {"number": to, "title": to_title},
        "title": {"changed": from_title != to_title, "inline": hunks(&diff_words(&from_title, &to_title))},
        "blocks": blocks,
        "stats": {"inserted": ins, "deleted": del, "changed": chg},
        "coarse": coarse,
    }))
}
