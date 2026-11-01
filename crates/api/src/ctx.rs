//! Request extractors: the authenticated caller and workspace ([`Ctx`]), query parameters
//! ([`Q`]), JSON bodies ([`Body`]) — every rejection in the surface's error shape — and [`Tx`],
//! the locked session a handler runs commands on.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, Query, RawPathParams, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use serde_json::Value;
use trackcraft_engine::{Session, model::Workspace};

use crate::error::{ApiError, Flavor};
use trackcraft_engine::sync::proto::Origin;

use crate::host::{CommandEvent, Credential, Hosted};
use crate::{ApiState, BLOCKED};

/// The caller and the workspace a request acts on.
pub struct Ctx {
    pub state: ApiState,
    pub hosted: Arc<Hosted>,
    pub user: String,
    pub flavor: Flavor,
    /// Absolute base URL of this workspace (`http://host:port` or `…/w/acme`), for `self` links.
    pub base: String,
}

fn path_param(params: &RawPathParams, name: &str) -> Option<String> {
    params.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
}

impl FromRequestParts<ApiState> for Ctx {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &ApiState) -> Result<Self, ApiError> {
        let flavor = Flavor::of(parts.uri.path());
        let auth = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let identity = credential(&auth, |c| state.identity.authenticate(&c)).ok_or_else(|| ApiError::unauthorized(flavor))?;
        let ws = match RawPathParams::from_request_parts(parts, state).await {
            Ok(p) => path_param(&p, "ws"),
            Err(_) => return Err(ApiError::bad(flavor, "the request path is not valid UTF-8")),
        };
        let hosted = state.hub.resolve(ws.as_deref()).map_err(|e| ApiError::not_found(flavor, e))?;
        let origin = match &state.base_url {
            Some(b) => b.trim_end_matches('/').to_string(),
            None => format!("http://{}", host(&parts.headers)),
        };
        let base = match &ws {
            Some(w) => format!("{origin}/w/{}", pct(w)),
            None => origin,
        };
        Ok(Ctx { state: state.clone(), hosted, user: identity.user, flavor, base })
    }
}

/// The `Host` header, limited to characters that are safe to echo into URLs.
fn host(h: &HeaderMap) -> String {
    let raw = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    let ok = raw.len() <= 255 && raw.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if ok { raw.to_string() } else { "localhost".into() }
}

/// Parses an `Authorization` header value and hands the credential to `check`.
pub fn credential<T>(header: &str, check: impl FnOnce(Credential<'_>) -> Option<T>) -> Option<T> {
    let (scheme, rest) = header.trim().split_once(' ')?;
    let rest = rest.trim();
    if scheme.eq_ignore_ascii_case("bearer") {
        return check(Credential::Bearer(rest));
    }
    if scheme.eq_ignore_ascii_case("basic") {
        let decoded = base64_decode(rest)?;
        let text = String::from_utf8(decoded).ok()?;
        let (user, secret) = text.split_once(':')?;
        return check(Credential::Basic { user, secret });
    }
    None
}

/// Standard base64 (with or without padding). `None` on anything else.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() > 8192 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    Some(out)
}

/// Percent-encodes a URL component (unreserved characters pass through).
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Ctx {
    /// Runs `f` on the workspace's session as the caller, off the async threads. The session's
    /// acting user is set for the call and restored afterwards; the caller must be a member of
    /// the workspace (403 otherwise). Changes raise the hub's saver signal and go to the event
    /// sink after the lock is released.
    pub async fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut Tx<'_>) -> Result<T, ApiError> + Send + 'static) -> Result<T, ApiError> {
        let hosted = Arc::clone(&self.hosted);
        let (user, flavor) = (self.user.clone(), self.flavor);
        let joined = tokio::task::spawn_blocking(move || {
            let mut s = hosted.lock();
            if s.ws.user(&user).is_err() {
                return (
                    Err(ApiError::new(flavor, StatusCode::FORBIDDEN, format!("user {user} is not a member of workspace {}", hosted.name()))),
                    vec![],
                    false,
                    None,
                );
            }
            let (prev_user, rev, sync_rev) = (std::mem::replace(&mut s.user, user.clone()), s.revision(), s.sync_rev());
            let mut tx = Tx { s: &mut s, flavor, events: Vec::new(), before: None };
            let out = f(&mut tx);
            let (events, before) = (std::mem::take(&mut tx.events), tx.before.take());
            let events = match (&out, before) {
                (Err(_), Some(before)) => {
                    s.rollback_to(before, sync_rev);
                    Vec::new()
                }
                _ => events,
            };
            s.user = prev_user;
            s.set_origin(None);
            let changed = s.revision() != rev;
            let synced = (s.sync_rev() != sync_rev).then(|| s.sync_rev());
            (out, events, changed, synced)
        })
        .await;
        let (out, events, changed, synced) =
            joined.map_err(|_| ApiError::internal(self.flavor, "internal error while handling the request (please report this)"))?;
        if changed {
            self.state.hub.notify_changed();
        }
        if let Some(rev) = synced {
            self.state.sync.publish(self.hosted.name(), rev);
        }
        for (command, params, result) in events {
            self.state.events.emit(&CommandEvent { workspace: self.hosted.name().to_string(), user: self.user.clone(), command, params, result });
        }
        out
    }
}

/// A locked session, acting as the request's user.
pub struct Tx<'a> {
    pub s: &'a mut Session,
    pub flavor: Flavor,
    events: Vec<(String, Value, Value)>,
    before: Option<Workspace>,
}

impl Tx<'_> {
    pub fn ws(&self) -> &Workspace {
        &self.s.ws
    }

    pub fn user(&self) -> &str {
        &self.s.user
    }

    /// Makes the request all-or-nothing: if the handler fails after some commands succeeded,
    /// the workspace is put back as it was. Call before the first command of a multi-command
    /// request.
    pub fn atomic(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.s.ws.clone());
        }
    }

    /// Runs one engine command for a sync client: its journal entry carries `origin`.
    pub fn run_from(&mut self, id: &str, params: Value, origin: Option<Origin>) -> Result<Value, ApiError> {
        self.s.set_origin(origin);
        let r = self.run(id, params);
        self.s.set_origin(None);
        r
    }

    /// Runs one engine command; its error becomes an API error (404 / 409 / 400).
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, ApiError> {
        if BLOCKED.contains(&id) {
            return Err(ApiError::new(self.flavor, StatusCode::FORBIDDEN, format!("{id} is not available over the server API")));
        }
        let r = self.s.run(id, &params).map_err(|e| ApiError::from_command(self.flavor, e))?;
        if trackcraft_engine::command(id).is_some_and(|c| c.mutates) {
            self.events.push((id.to_string(), params, r.clone()));
        }
        Ok(r)
    }
}

/// Query parameters in order, repeated keys kept (`space-id=1&space-id=2`).
pub struct Q(pub Vec<(String, String)>);

impl<S: Send + Sync> FromRequestParts<S> for Q {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        let flavor = Flavor::of(parts.uri.path());
        Query::<Vec<(String, String)>>::try_from_uri(&parts.uri)
            .map(|Query(v)| Q(v))
            .map_err(|e| ApiError::bad(flavor, format!("invalid query string: {e}")))
    }
}

impl Q {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    /// Every value of `k`, comma-separated values split (`fields=a,b&fields=c`).
    pub fn list(&self, k: &str) -> Vec<String> {
        self.0.iter().filter(|(n, _)| n == k).flat_map(|(_, v)| v.split(',')).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }

    /// A whole number, clamped to `max`; 400 if it isn't one.
    pub fn num(&self, k: &str, default: usize, max: usize, flavor: Flavor) -> Result<usize, ApiError> {
        match self.get(k) {
            None | Some("") => Ok(default),
            Some(v) => v.trim().parse::<usize>().map(|n| n.min(max)).map_err(|_| ApiError::bad(flavor, format!("'{k}' must be a whole number"))),
        }
    }

    pub fn flag(&self, k: &str) -> bool {
        self.get(k).is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
    }
}

/// A JSON request body (empty = `null`). Oversized bodies are 413, invalid JSON 400.
pub struct Body(pub Value);

impl<S: Send + Sync> FromRequest<S> for Body {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let flavor = Flavor::of(req.uri().path());
        let bytes = Bytes::from_request(req, state).await.map_err(|e| {
            let status = e.status();
            let msg = if status == StatusCode::PAYLOAD_TOO_LARGE { "the request body is too large".to_string() } else { e.body_text() };
            ApiError::new(flavor, status, msg)
        })?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Body(Value::Null));
        }
        serde_json::from_slice(&bytes).map(Body).map_err(|e| ApiError::bad(flavor, format!("the request body is not valid JSON: {e}")))
    }
}

/// Path parameters by name (works the same under `/w/{ws}`).
pub struct P(pub Vec<(String, String)>);

impl<S: Send + Sync> FromRequestParts<S> for P {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let flavor = Flavor::of(parts.uri.path());
        let raw = RawPathParams::from_request_parts(parts, state).await.map_err(|_| ApiError::bad(flavor, "the request path is not valid UTF-8"))?;
        Ok(P(raw.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()))
    }
}

impl P {
    pub fn get(&self, k: &str) -> String {
        self.0.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_credentials() {
        assert_eq!(base64_decode("YW5uOnRvaw==").unwrap(), b"ann:tok");
        assert_eq!(base64_decode("YW5uOnRvaw").unwrap(), b"ann:tok");
        assert!(base64_decode("!!").is_none());
        let got = credential("Basic YW5uOnRvaw==", |c| match c {
            Credential::Basic { user, secret } => Some(format!("{user}/{secret}")),
            Credential::Bearer(_) => None,
        });
        assert_eq!(got.as_deref(), Some("ann/tok"));
        assert_eq!(credential("Bearer abc", |c| matches!(c, Credential::Bearer("abc")).then_some(1)), Some(1));
        assert_eq!(credential("Digest x", |_| Some(1)), None);
        assert_eq!(credential("", |_| Some(1)), None);
        assert_eq!(pct("a b/é"), "a%20b%2F%C3%A9");
    }
}
