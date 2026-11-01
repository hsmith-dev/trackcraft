//! TrackCraft MCP server (L5): the Model Context Protocol over JSON-RPC 2.0 (stdio, one message
//! per line). Agents reach every engine command through two tools:
//!
//! - `trackcraft_commands` — the command list (ids, params docs, enabled state);
//! - `trackcraft_run` — `{"command": "issue.create", "params": {…}}`.
//!
//! [`Server::handle`] is transport-free so the same handler serves stdio, the control channel and
//! (later) the HTTP server.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufRead, Write};

use serde_json::{Value, json};
use trackcraft_engine::Session;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Longest request line accepted (bytes).
pub const MAX_LINE: usize = 16 * 1024 * 1024;

pub struct Server {
    pub session: Session,
    /// Called after every successful mutating command (autosave hook).
    pub after_change: Option<Box<dyn FnMut(&Session)>>,
}

impl Server {
    pub fn new(session: Session) -> Self {
        Server { session, after_change: None }
    }

    /// Handles one JSON-RPC message; `None` for notifications.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "trackcraft", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "TrackCraft: issue tracking and wiki. Call trackcraft_commands for the command list, then trackcraft_run. Search issues with command search.jql (JQL).",
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => Ok(self.call(&params)),
            m if m.starts_with("notifications/") => return None,
            _ => Err(json!({"code": -32601, "message": format!("method not found: {}", method.chars().take(80).collect::<String>())})),
        };
        let id = id?;
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        })
    }

    fn call(&mut self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);
        let out = match name {
            "trackcraft_commands" => self.session.run("app.commands", &Value::Null),
            "trackcraft_run" => match args.get("command").and_then(Value::as_str) {
                Some(cmd) => {
                    let p = args.get("params").cloned().unwrap_or_else(|| json!({}));
                    let r = self.session.run(cmd, &p);
                    if r.is_ok() && trackcraft_engine::command(cmd).is_some_and(|c| c.mutates) {
                        let s = &self.session;
                        if let Some(f) = self.after_change.as_mut() {
                            f(s);
                        }
                    }
                    r
                }
                None => Err("missing \"command\"".into()),
            },
            other => Err(format!("unknown tool {}", other.chars().take(80).collect::<String>())),
        };
        match out {
            Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}], "structuredContent": {"result": v}}),
            Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
        }
    }

    /// Serves stdio until EOF.
    pub fn serve_stdio(&mut self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let mut out = std::io::stdout().lock();
        let mut line = String::new();
        let mut reader = stdin.lock();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            if line.trim().is_empty() {
                continue;
            }
            let reply = if line.len() > MAX_LINE {
                Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "request too large"}}))
            } else {
                match serde_json::from_str::<Value>(&line) {
                    Ok(msg) => self.handle(&msg),
                    Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
                }
            };
            if let Some(r) = reply {
                writeln!(out, "{r}")?;
                out.flush()?;
            }
        }
    }
}

fn tools() -> Value {
    json!([
        {
            "name": "trackcraft_commands",
            "description": "List every TrackCraft command: id, label, params description, whether it changes data, and why it is disabled (if it is).",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "trackcraft_run",
            "description": "Run an TrackCraft command by id, e.g. {\"command\":\"issue.create\",\"params\":{\"project\":\"LUME\",\"summary\":\"…\"}} or {\"command\":\"search.jql\",\"params\":{\"jql\":\"assignee = currentUser() AND statusCategory != Done\"}}. Changes can be undone with edit.undo.",
            "inputSchema": {
                "type": "object",
                "properties": {"command": {"type": "string"}, "params": {"type": "object"}},
                "required": ["command"],
            },
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_list_and_call() {
        let mut s = Server::new(Session::sample());
        let init = s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "trackcraft");
        assert!(s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let list = s.handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        assert_eq!(list["result"]["tools"].as_array().map(Vec::len), Some(2));
        let r = s
            .handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "trackcraft_run", "arguments": {"command": "issue.get", "params": {"key": "LUME-1"}}}}))
            .unwrap();
        assert_eq!(r["result"]["structuredContent"]["result"]["summary"], "Offline mode");
        let bad =
            s.handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "trackcraft_run", "arguments": 5}})).unwrap();
        assert_eq!(bad["result"]["isError"], true);
        let unknown = s.handle(&json!({"id": 5, "method": "nope"})).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
