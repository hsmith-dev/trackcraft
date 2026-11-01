//! Short-lived, single-use **stream tickets** for browsers (M4.6, `docs/sync.md`).
//!
//! A browser's `EventSource` can't send an `Authorization` header, and the API authenticates by
//! header only. Rather than put the long-lived API token in a URL (where it would end up in
//! proxy logs, browser history and `Referer`s), the web client:
//!
//! 1. `POST {base}/api/sync/ticket` with its usual `Authorization: Bearer` header →
//!    `{"ticket": "tcs_…", "expiresIn": 60}`;
//! 2. opens `GET {base}/api/sync/events?since=…&ticket=tcs_…` at once.
//!
//! A ticket is 128 random bits, lives [`TTL`], is good for **one** request, only on the
//! **event-stream path** of the **workspace base it was minted for**, and acts as the user whose
//! token minted it (so workspace membership is checked as usual when the stream opens). Our
//! request log never prints query strings; a ticket that reaches someone else's log is spent or
//! expired by the time anyone reads it.
//!
//! How it plugs in without touching the API's handlers: [`api_router`] builds
//! `trackcraft_api::router` with the server's identity provider wrapped in [`TicketIdentity`],
//! adds the mint route, and layers [`redeem`] in front. `redeem` turns a valid `?ticket=` on the
//! events path into `Authorization: Bearer tcs_…` and marks the ticket *redeemed*;
//! `TicketIdentity` accepts a redeemed ticket exactly once (fresh ones never), and passes every
//! other credential to the real provider unchanged.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use serde_json::json;
use trackcraft_api::ctx::Ctx;
use trackcraft_api::{ApiError, Credential, Flavor, Identity, IdentityProvider};

use crate::Server;

/// How long a ticket can wait before it's used.
pub const TTL: Duration = Duration::from_secs(60);
/// Prefix of every ticket (never a valid API token).
pub const PREFIX: &str = "tcs_";
/// Most outstanding tickets; minting answers 503 above it (expired ones are pruned first).
pub const MAX_TICKETS: usize = 10_000;

const MINT_PATH: &str = "/api/sync/ticket";
const EVENTS_PATH: &str = "/api/sync/events";

#[derive(Debug)]
struct Ticket {
    user: String,
    /// The workspace base the ticket was minted under (`""` or `/w/lab`, as routed).
    base: String,
    expires: Instant,
    redeemed: bool,
}

/// Outstanding tickets.
#[derive(Debug, Default)]
pub struct Tickets {
    map: Mutex<HashMap<String, Ticket>>,
}

impl Tickets {
    /// Mints a ticket for `user` under `base`; `None` when too many are outstanding or the OS
    /// random source failed.
    pub fn mint(&self, user: &str, base: &str) -> Option<String> {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).ok()?;
        let id = format!("{PREFIX}{}", hex::encode(b));
        let now = Instant::now();
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, t| t.expires > now);
        if map.len() >= MAX_TICKETS {
            return None;
        }
        map.insert(id.clone(), Ticket { user: user.to_string(), base: base.to_string(), expires: now + TTL, redeemed: false });
        Some(id)
    }

    /// Marks a fresh, unexpired ticket minted under `base` as redeemed. A ticket presented on
    /// the wrong base, a second time, or late is removed and refused.
    fn redeem(&self, id: &str, base: &str) -> bool {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        match map.get_mut(id) {
            Some(t) if !t.redeemed && t.base == base && t.expires > Instant::now() => {
                t.redeemed = true;
                true
            }
            Some(_) => {
                map.remove(id);
                false
            }
            None => false,
        }
    }

    /// The user of a redeemed ticket, consuming it.
    fn take_redeemed(&self, id: &str) -> Option<String> {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let ok = map.get(id).is_some_and(|t| t.redeemed && t.expires > Instant::now());
        let t = map.remove(id)?;
        ok.then_some(t.user)
    }
}

/// The server's identity provider, plus redeemed tickets.
pub struct TicketIdentity {
    pub inner: Arc<dyn IdentityProvider>,
    pub tickets: Arc<Tickets>,
}

impl IdentityProvider for TicketIdentity {
    fn authenticate(&self, credential: &Credential<'_>) -> Option<Identity> {
        match credential {
            Credential::Bearer(t) if t.starts_with(PREFIX) => self.tickets.take_redeemed(t).map(|user| Identity { user }),
            c => self.inner.authenticate(c),
        }
    }
}

/// `trackcraft_api::router` with stream tickets: the wrapped identity, the mint route (root and
/// `/w/{ws}`) and the redeem layer.
pub fn api_router(server: &Server) -> Router {
    let tickets = Arc::new(Tickets::default());
    let mut state = server.api_state();
    state.identity = Arc::new(TicketIdentity { inner: Arc::clone(&state.identity), tickets: Arc::clone(&tickets) });
    let mint_routes = Router::new()
        .route(MINT_PATH, post(mint))
        .route(&format!("/w/{{ws}}{MINT_PATH}"), post(mint))
        .layer(Extension(Arc::clone(&tickets)))
        .with_state(state.clone());
    trackcraft_api::router(state).merge(mint_routes).layer(from_fn_with_state(tickets, redeem))
}

/// `POST …/api/sync/ticket` (authenticated as usual) → `{"ticket", "expiresIn"}`.
async fn mint(Extension(tickets): Extension<Arc<Tickets>>, uri: Uri, cx: Ctx) -> Result<Response, ApiError> {
    // A ticket can't mint another (it is only ever accepted on the events path).
    let base = uri.path().strip_suffix(MINT_PATH).unwrap_or_default().to_string();
    // Members only, as for every other route (checked under the workspace lock).
    cx.run(|_| Ok(())).await?;
    let ticket = tickets
        .mint(&cx.user, &base)
        .ok_or_else(|| ApiError::new(Flavor::Native, StatusCode::SERVICE_UNAVAILABLE, "too many outstanding stream tickets; try again shortly"))?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(json!({"ticket": ticket, "expiresIn": TTL.as_secs()}))).into_response())
}

/// The `ticket` query parameter, if it looks like one.
fn ticket_param(query: &str) -> Option<&str> {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("ticket="))
        .filter(|t| t.len() <= 64 && t.starts_with(PREFIX) && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// On the events path, swaps a valid `?ticket=` for a bearer header the wrapped identity accepts
/// once. Anything else passes through untouched (an invalid ticket simply authenticates nobody).
async fn redeem(State(tickets): State<Arc<Tickets>>, mut req: Request, next: Next) -> Response {
    let base = req.uri().path().strip_suffix(EVENTS_PATH).map(str::to_string);
    let ticket = req.uri().query().and_then(ticket_param).map(str::to_string);
    if let (Some(base), Some(ticket)) = (base, ticket) {
        let headers = req.headers_mut();
        headers.remove(header::AUTHORIZATION);
        if tickets.redeem(&ticket, &base)
            && let Ok(v) = HeaderValue::from_str(&format!("Bearer {ticket}"))
        {
            headers.insert(header::AUTHORIZATION, v);
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;
    use trackcraft_api::{Hosted, Hub, NullSink};
    use trackcraft_engine::Session;

    use super::*;
    use crate::{Options, TokenIdentity, TokenRecord, config};

    const TOKEN: &str = "tct_ticket_secret";

    fn router() -> Router {
        let hub = Arc::new(Hub::new());
        hub.add(Hosted::new("lab", Session::sample(), None));
        hub.add(Hosted::new("other", Session::sample(), None));
        hub.set_default(Some("lab"));
        let rec = TokenRecord {
            id: "t1".into(),
            user: "riley".into(),
            email: None,
            label: String::new(),
            sha256: config::hash(TOKEN),
            created: String::new(),
        };
        let server =
            Server::new(hub, Arc::new(TokenIdentity::new(vec![rec])), Arc::new(NullSink), Options { log_requests: false, ..Options::default() });
        server.router()
    }

    async fn send(r: &Router, method: &str, path: &str, auth: Option<&str>) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(path).header("host", "track.test");
        if let Some(a) = auth {
            b = b.header("authorization", format!("Bearer {a}"));
        }
        let resp = r.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        // Event streams never end: only read bodies of non-streams.
        let is_stream = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("text/event-stream"));
        if is_stream {
            return (status, Value::Null);
        }
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn mint(r: &Router, base: &str) -> String {
        let (s, v) = send(r, "POST", &format!("{base}/api/sync/ticket"), Some(TOKEN)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["expiresIn"], 60);
        v["ticket"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_ticket_opens_one_stream_once() {
        let r = router();
        let t = mint(&r, "").await;
        assert!(t.starts_with(PREFIX) && t.len() == PREFIX.len() + 32);
        assert_eq!(send(&r, "GET", &format!("/api/sync/events?since=0&ticket={t}"), None).await.0, StatusCode::OK);
        // Single use.
        assert_eq!(send(&r, "GET", &format!("/api/sync/events?since=0&ticket={t}"), None).await.0, StatusCode::UNAUTHORIZED);
        // Without a ticket or token: refused as before.
        assert_eq!(send(&r, "GET", "/api/sync/events", None).await.0, StatusCode::UNAUTHORIZED);
        // Under /w/{ws} too.
        let t = mint(&r, "/w/other").await;
        assert_eq!(send(&r, "GET", &format!("/w/other/api/sync/events?ticket={t}"), None).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn tickets_work_nowhere_else() {
        let r = router();
        // Minting needs the real token.
        assert_eq!(send(&r, "POST", "/api/sync/ticket", None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&r, "POST", "/api/sync/ticket", Some("tct_wrong")).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&r, "POST", "/w/nope/api/sync/ticket", Some(TOKEN)).await.0, StatusCode::NOT_FOUND);
        // A fresh ticket is not a bearer token, nor a query credential elsewhere.
        let t = mint(&r, "").await;
        assert_eq!(send(&r, "GET", "/api/workspace", Some(&t)).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&r, "GET", &format!("/api/sync/snapshot?ticket={t}"), None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&r, "POST", "/api/sync/ticket", Some(&t)).await.0, StatusCode::UNAUTHORIZED);
        // Bound to the workspace base it was minted for; a misuse spends it.
        let t = mint(&r, "").await;
        assert_eq!(send(&r, "GET", &format!("/w/other/api/sync/events?ticket={t}"), None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&r, "GET", &format!("/api/sync/events?ticket={t}"), None).await.0, StatusCode::UNAUTHORIZED);
        // Junk tickets.
        for junk in ["", "tcs_", "tcs_zz", "x", "tcs_%00", &"a".repeat(500)] {
            assert_eq!(send(&r, "GET", &format!("/api/sync/events?ticket={junk}"), None).await.0, StatusCode::UNAUTHORIZED, "{junk}");
        }
        // Existing routes are unchanged.
        assert_eq!(send(&r, "GET", "/api/sync/snapshot", Some(TOKEN)).await.0, StatusCode::OK);
        assert_eq!(send(&r, "GET", "/api/health", None).await.0, StatusCode::OK);
    }

    #[test]
    fn expired_and_capped() {
        let t = Tickets::default();
        let id = t.mint("riley", "").unwrap();
        t.map.lock().unwrap().get_mut(&id).unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(!t.redeem(&id, ""));
        assert!(t.take_redeemed(&id).is_none());
        for _ in 0..MAX_TICKETS {
            t.mint("riley", "").unwrap();
        }
        assert!(t.mint("riley", "").is_none());
    }
}
