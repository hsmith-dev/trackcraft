//! TrackCraft-native routes: run any engine command (the MCP/CLI equivalent), list commands,
//! list workspaces, and an unauthenticated health check.

use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get, routing::post};
use serde_json::{Value, json};

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

/// `{"command": "issue.create", "params": {…}}` → `{"result": …}`.
async fn command(cx: Ctx, Body(b): Body) -> Result<Response, ApiError> {
    let id = b.get("command").and_then(Value::as_str).map(str::to_string).ok_or_else(|| ApiError::bad(Flavor::Native, "missing \"command\""))?;
    let params = b.get("params").cloned().unwrap_or_else(|| json!({}));
    cx.run(move |tx| {
        let result = tx.run(&id, params)?;
        Ok(Json(json!({"command": id, "result": result})).into_response())
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
