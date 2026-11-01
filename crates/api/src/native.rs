//! TrackCraft-native routes: run any engine command (the MCP/CLI equivalent), list commands,
//! list workspaces, and an unauthenticated health check.

use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get, routing::post};
use serde_json::{Value, json};
use trackcraft_engine::sync::proto::Origin;

use crate::ctx::{Body, Ctx};
use crate::error::{ApiError, Flavor};
use crate::{ApiState, BLOCKED};

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/commands", get(commands))
        .route("/api/command", post(command))
        .route("/api/workspace", get(workspace))
}

async fn health() -> Response {
    Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")})).into_response()
}

async fn commands(cx: Ctx) -> Result<Response, ApiError> {
    cx.run(|tx| {
        let mut list = tx.run("app.commands", Value::Null)?;
        for c in list.as_array_mut().into_iter().flatten() {
            if c.get("id").and_then(Value::as_str).is_some_and(|id| BLOCKED.contains(&id)) {
                c["disabled"] = json!("not available over the server API");
            }
        }
        Ok(Json(list).into_response())
    })
    .await
}

/// `{"command": "issue.create", "params": {…}, "origin": {"client", "seq"} (sync clients)}` →
/// `{"command", "result", "rev", "entry", "duplicate"}`. `rev` is the workspace's sync revision
/// after the command and `entry` the journal entry it produced (`null` for queries and no-ops).
/// A sync client's `origin.seq` that was already applied answers `duplicate: true` without
/// running anything (a resend after a lost response).
async fn command(cx: Ctx, Body(b): Body) -> Result<Response, ApiError> {
    let id = b.get("command").and_then(Value::as_str).map(str::to_string).ok_or_else(|| ApiError::bad(Flavor::Native, "missing \"command\""))?;
    let params = b.get("params").cloned().unwrap_or_else(|| json!({}));
    let origin = match b.get("origin") {
        None | Some(Value::Null) => None,
        Some(o) => {
            let o: Origin =
                serde_json::from_value(o.clone()).map_err(|_| ApiError::bad(Flavor::Native, "\"origin\" must be {\"client\": id, \"seq\": n}"))?;
            if !Origin::valid_client(&o.client) {
                return Err(ApiError::bad(Flavor::Native, "\"origin.client\" must be 1-64 of A-Z a-z 0-9 - _"));
            }
            Some(o)
        }
    };
    cx.run(move |tx| {
        if let Some(o) = &origin
            && tx.s.journal().and_then(|j| j.applied_seq(&o.client)).is_some_and(|max| o.seq <= max)
        {
            return Ok(Json(json!({"command": id, "result": null, "rev": tx.s.sync_rev(), "entry": null, "duplicate": true})).into_response());
        }
        let before = tx.s.sync_rev();
        let result = tx.run_from(&id, params, origin)?;
        let rev = tx.s.sync_rev();
        let entry = if rev != before { tx.s.journal().and_then(|j| j.last()).cloned() } else { None };
        Ok(Json(json!({"command": id, "result": result, "rev": rev, "entry": entry, "duplicate": false})).into_response())
    })
    .await
}

async fn workspace(cx: Ctx) -> Result<Response, ApiError> {
    let name = cx.hosted.name().to_string();
    let user = cx.user.clone();
    cx.run(move |tx| {
        let ws = tx.ws();
        Ok(Json(json!({
            "name": name,
            "title": ws.name,
            "user": user,
            "projects": ws.projects.len(),
            "issues": ws.issues.len(),
            "spaces": ws.spaces.len(),
            "pages": ws.pages.len(),
        }))
        .into_response())
    })
    .await
}
