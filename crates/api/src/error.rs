//! Errors in each surface's public shape.
//!
//! - Jira (`/rest/…`): `{"errorMessages": ["…"], "errors": {}}`
//! - Confluence v2 (`/wiki/…`): `{"errors": [{"status": 404, "code": "NOT_FOUND", "title": "…", "detail": null}]}`
//! - TrackCraft native (`/api/…`): `{"error": "…"}`

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

/// Longest error message returned (messages can echo hostile input).
const MAX_MESSAGE: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Jira,
    Confluence,
    Native,
}

impl Flavor {
    /// The surface a request path belongs to (after any `/w/{workspace}` prefix).
    pub fn of(path: &str) -> Flavor {
        let p = strip_workspace(path);
        if p.starts_with("/wiki/") || p == "/wiki" {
            Flavor::Confluence
        } else if p.starts_with("/rest/") {
            Flavor::Jira
        } else {
            Flavor::Native
        }
    }
}

/// `/w/acme/rest/api/3/myself` → `/rest/api/3/myself`.
pub fn strip_workspace(path: &str) -> &str {
    match path.strip_prefix("/w/") {
        Some(rest) => rest.find('/').and_then(|i| rest.get(i..)).unwrap_or("/"),
        None => path,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub flavor: Flavor,
    /// Add `WWW-Authenticate` (401s).
    challenge: bool,
}

impl ApiError {
    pub fn new(flavor: Flavor, status: StatusCode, message: impl Into<String>) -> Self {
        let m: String = message.into();
        ApiError { status, message: m.chars().take(MAX_MESSAGE).collect(), flavor, challenge: false }
    }
    pub fn bad(flavor: Flavor, message: impl Into<String>) -> Self {
        ApiError::new(flavor, StatusCode::BAD_REQUEST, message)
    }
    pub fn not_found(flavor: Flavor, message: impl Into<String>) -> Self {
        ApiError::new(flavor, StatusCode::NOT_FOUND, message)
    }
    pub fn unauthorized(flavor: Flavor) -> Self {
        let mut e = ApiError::new(flavor, StatusCode::UNAUTHORIZED, "authentication required: send an API token (Bearer, or Basic email:token)");
        e.challenge = true;
        e
    }
    pub fn internal(flavor: Flavor, message: impl Into<String>) -> Self {
        ApiError::new(flavor, StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    /// Maps an engine command error to a status: missing things are 404 (and so is anything the
    /// caller may not browse: the engine words it as missing), a permission the caller lacks on
    /// something they can see is 403, a stale page version is 409, everything else the caller
    /// sent is 400.
    pub fn from_command(flavor: Flavor, message: String) -> Self {
        const MISSING: &[&str] = &[
            "no issue ",
            "no project with key",
            "no space with key",
            "no page with id",
            "no user ",
            "no comment with id",
            "no sprint with id",
            "no template",
        ];
        let status = if MISSING.iter().any(|m| message.starts_with(m)) {
            StatusCode::NOT_FOUND
        } else if message.starts_with(trackcraft_engine::authz::DENIED) {
            StatusCode::FORBIDDEN
        } else if message.starts_with("the page changed") {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        };
        ApiError::new(flavor, status, message)
    }

    pub fn body(&self) -> Value {
        match self.flavor {
            Flavor::Jira => json!({"errorMessages": [self.message], "errors": {}}),
            Flavor::Confluence => {
                json!({"errors": [{"status": self.status.as_u16(), "code": code(self.status), "title": self.message, "detail": null}]})
            }
            Flavor::Native => json!({"error": self.message, "status": self.status.as_u16()}),
        }
    }
}

fn code(s: StatusCode) -> String {
    s.canonical_reason().unwrap_or("ERROR").to_ascii_uppercase().replace([' ', '-'], "_")
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut r = (self.status, Json(self.body())).into_response();
        if self.challenge {
            r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"trackcraft\""));
        }
        r
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
