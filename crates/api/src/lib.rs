//! TrackCraft REST API (L5): HTTP routes that map requests onto engine commands.
//!
//! Three surfaces, each answering in its public shape (clean room: shapes from Atlassian's public
//! REST references, reimplemented for interoperability):
//!
//! - Jira Cloud REST v3 (`/rest/api/3/…`) and Jira Software Agile 1.0 (`/rest/agile/1.0/…`);
//! - Confluence Cloud REST v2 (`/wiki/api/v2/…`);
//! - TrackCraft native (`/api/command`, `/api/commands`): any engine command by id.
//!
//! Every route is served at the root (the default workspace) and under `/w/{workspace}/…`.
//! Reads build responses from the workspace; every change runs through `Session::run`, as the
//! authenticated user, on a session locked for the request (see [`ctx::Ctx::run`]).
//!
//! This crate creates no runtime and does no file I/O: `trackcraft-server` (crates/server)
//! supplies identity, persistence, limits and the listener.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod agile;
pub mod ctx;
pub mod error;
pub mod host;
pub mod ids;
pub mod jira;
pub mod native;
pub mod wiki;

use std::sync::Arc;

use axum::Router;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};

pub use error::{ApiError, Flavor};
pub use host::{CommandEvent, Credential, EventSink, Hosted, Hub, Identity, IdentityProvider, MemorySink, NullSink};

/// Commands the server API refuses: they act on the server's own files or on the shared
/// session rather than on the workspace (`file.open` would read any path on the host; undo would
/// undo another user's change).
pub const BLOCKED: &[&str] = &["file.open", "file.save", "workspace.new", "edit.undo", "edit.redo", "user.switch"];

/// What every handler can reach.
#[derive(Clone)]
pub struct ApiState {
    pub hub: Arc<Hub>,
    pub identity: Arc<dyn IdentityProvider>,
    pub events: Arc<dyn EventSink>,
    /// The public origin (`https://track.example.com`) used in `self` links; `None` = from the
    /// request's `Host` header.
    pub base_url: Option<String>,
}

fn routes() -> Router<ApiState> {
    Router::new().merge(jira::routes()).merge(agile::routes()).merge(wiki::routes()).merge(native::routes())
}

async fn not_found(uri: Uri) -> Response {
    let flavor = Flavor::of(uri.path());
    ApiError::not_found(flavor, format!("no such endpoint: {}", uri.path().chars().take(200).collect::<String>())).into_response()
}

async fn not_allowed(uri: Uri) -> Response {
    ApiError::new(Flavor::of(uri.path()), StatusCode::METHOD_NOT_ALLOWED, "method not allowed for this endpoint").into_response()
}

/// Every route, at the root and under `/w/{ws}`, with JSON 404/405 fallbacks. Limits (body size,
/// timeout) and logging are layered on by the server.
pub fn router(state: ApiState) -> Router {
    Router::new().merge(routes()).nest("/w/{ws}", routes()).fallback(not_found).method_not_allowed_fallback(not_allowed).with_state(state)
}
