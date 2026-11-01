//! M4.5 notifications and watchers: the acting user's inbox (`inbox.*`), watch lists (`issue.watch`,
//! `page.watch`, their `unwatch`, and the `watchers.list` query).
//!
//! Notifications are generated inside the model's edit primitives and stored in the workspace,
//! so they sync and replay like every other change (`docs/notifications.md`).

use serde_json::{Value, json};
use trackcraft_core::{Id, IssueKey};
use trackcraft_model::{MAX_NOTIFICATIONS, Notification, Target};

use crate::{CmdResult, CommandSpec, Session, params};

/// Most ids one `inbox.markRead` takes.
pub const MAX_MARK: usize = MAX_NOTIFICATIONS;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("inbox.list", "Notifications", "Inbox", inbox_list)
            .params(r#"{"unreadOnly": false, "limit": 50} → the acting user's notifications, newest first, and the unread count"#)
            .query(),
        CommandSpec::new("inbox.markRead", "Mark Notifications Read", "Inbox", inbox_mark_read).params(r#"{"ids": [3, 4]} | {"all": true}"#),
        CommandSpec::new("issue.watch", "Watch Issue", "Issue", issue_watch).params(r#"{"key": "LUME-1"}"#),
        CommandSpec::new("issue.unwatch", "Stop Watching Issue", "Issue", issue_unwatch).params(r#"{"key": "LUME-1"}"#),
        CommandSpec::new("page.watch", "Watch Page", "Page", page_watch).params(r#"{"id": 14}"#),
        CommandSpec::new("page.unwatch", "Stop Watching Page", "Page", page_unwatch).params(r#"{"id": 14}"#),
        CommandSpec::new("watchers.list", "Watchers", "Issue", watchers)
            .params(r#"{"key": "LUME-1"} | {"id": 14} → {"watchers": ["riley"], "watching": true}"#)
            .query(),
    ]
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// A notification in the shape `inbox.list` returns.
pub fn view(n: &Notification) -> Value {
    let (kind, target) = match &n.target {
        Target::Issue(k) => ("issue", json!({"type": "issue", "key": k.to_string()})),
        Target::Page(id) => ("page", json!({"type": "page", "id": id.0})),
    };
    json!({
        "id": n.id,
        "kind": n.kind.name(),
        "target": target,
        "targetType": kind,
        "actor": n.actor,
        "at": n.at.to_rfc3339(),
        "title": n.title,
        "excerpt": n.excerpt,
        "read": n.read,
    })
}

fn inbox_list(s: &mut Session, p: &Value) -> CmdResult {
    let unread_only = match p.get("unreadOnly") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("\"unreadOnly\" must be true or false".into()),
    };
    let limit = params::usize_or(p, "limit", 50, MAX_NOTIFICATIONS)?;
    let list = s.ws.inbox.of(&s.user);
    let items: Vec<Value> = list.iter().rev().filter(|n| !unread_only || !n.read).take(limit).map(view).collect();
    Ok(json!({"user": s.user, "unread": s.ws.inbox.unread(&s.user), "total": list.len(), "notifications": items}))
}

fn inbox_mark_read(s: &mut Session, p: &Value) -> CmdResult {
    let all = match p.get("all") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("\"all\" must be true or false".into()),
    };
    let ids: Option<Vec<u64>> = match p.get("ids") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) if a.len() > MAX_MARK => return Err(format!("\"ids\" takes at most {MAX_MARK} ids")),
        Some(Value::Array(a)) => {
            Some(a.iter().map(|v| v.as_u64().ok_or_else(|| "\"ids\" must be a list of notification ids".to_string())).collect::<Result<_, _>>()?)
        }
        Some(_) => return Err("\"ids\" must be a list of notification ids".into()),
    };
    let user = s.user.clone();
    let changed = match (all, ids) {
        (true, None) => s.ws.mark_read(&user, None),
        (false, Some(ids)) => s.ws.mark_read(&user, Some(&ids)),
        (true, Some(_)) => return Err("give \"ids\" or \"all\": true, not both".into()),
        (false, None) => return Err("missing \"ids\" (a list of notification ids) or \"all\": true".into()),
    };
    Ok(json!({"marked": changed, "unread": s.ws.inbox.unread(&user)}))
}

fn issue_target(s: &Session, p: &Value) -> Result<Target, String> {
    let k = params::str(p, "key")?;
    let key = IssueKey::parse(k).map_err(|_| format!("no issue {}", k.chars().take(40).collect::<String>()))?;
    s.ws.issue(&key.to_string()).map_err(err)?;
    Ok(Target::Issue(key))
}

fn page_target(s: &Session, p: &Value) -> Result<Target, String> {
    let id: Id = params::id(p, "id")?;
    s.ws.page(id).map_err(err)?;
    Ok(Target::Page(id))
}

fn watch_result(s: &Session, t: &Target, changed: bool) -> CmdResult {
    let watchers = s.ws.watchers(t).map_err(err)?;
    Ok(json!({"changed": changed, "watching": s.ws.is_watching(&s.user, t), "watchCount": watchers.len()}))
}

fn set_watch(s: &mut Session, t: &Target, on: bool) -> CmdResult {
    let user = s.user.clone();
    let changed = if on { s.ws.watch(&user, t) } else { s.ws.unwatch(&user, t) }.map_err(err)?;
    watch_result(s, t, changed)
}

fn issue_watch(s: &mut Session, p: &Value) -> CmdResult {
    let t = issue_target(s, p)?;
    set_watch(s, &t, true)
}

fn issue_unwatch(s: &mut Session, p: &Value) -> CmdResult {
    let t = issue_target(s, p)?;
    set_watch(s, &t, false)
}

fn page_watch(s: &mut Session, p: &Value) -> CmdResult {
    let t = page_target(s, p)?;
    set_watch(s, &t, true)
}

fn page_unwatch(s: &mut Session, p: &Value) -> CmdResult {
    let t = page_target(s, p)?;
    set_watch(s, &t, false)
}

fn watchers(s: &mut Session, p: &Value) -> CmdResult {
    let t = match (p.get("key"), p.get("id")) {
        (Some(_), None) => issue_target(s, p)?,
        (None, Some(_)) => page_target(s, p)?,
        _ => return Err("give \"key\" (an issue) or \"id\" (a page)".into()),
    };
    let w = s.ws.watchers(&t).map_err(err)?;
    Ok(json!({"watching": s.ws.is_watching(&s.user, &t), "watchCount": w.len(), "watchers": w}))
}
