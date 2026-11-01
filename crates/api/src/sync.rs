//! Live sync (M4.2): the workspace snapshot, the journal as Server-Sent Events, and the
//! [`SyncHub`] seam that fans "workspace X is at revision N" out to stream handlers.
//!
//! - `GET /api/sync/snapshot[?client=ID]` → [`proto::Snapshot`] (`workspace` is the `.track`
//!   JSON envelope).
//! - `GET /api/sync/events?since=REV&epoch=E` → `text/event-stream`: `hello`, then one `entry`
//!   per journal entry after `since` (live), or a single `resync` when `since` can't be served
//!   (older than the journal ring, from another epoch, or in the future). Keep-alive comments
//!   every 15 s.
//!
//! Commands still arrive through `POST /api/command` (see `native.rs`); every mutating command
//! from any surface (Jira/Confluence routes too) lands in the journal, because the journal is
//! written by `Session::run` itself.

use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;
use tokio::sync::watch;
use trackcraft_engine::sync::SinceError;
use trackcraft_sync_proto as proto;

use crate::ApiState;
use crate::ctx::{Ctx, Q};
use crate::error::{ApiError, Flavor};
use crate::host::Hosted;

/// Entries read from the journal per lock.
const BATCH: usize = 256;
/// Interval of keep-alive comments on idle streams.
pub const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// Fans revision changes out to event streams. Standalone: [`LocalSyncHub`]. A suite server can
/// pass one shared hub to both apps (or bridge it to its own event bus); TrackCraft only needs
/// "publish rev N of workspace W" and "wake me when W moves".
pub trait SyncHub: Send + Sync + 'static {
    /// Called after a request moved `workspace` to sync revision `rev` (lock released).
    fn publish(&self, workspace: &str, rev: u64);
    /// A receiver that changes after the next [`SyncHub::publish`] for `workspace`.
    fn subscribe(&self, workspace: &str) -> watch::Receiver<u64>;
}

/// One `watch` channel per workspace, in process.
#[derive(Default)]
pub struct LocalSyncHub {
    chans: Mutex<BTreeMap<String, watch::Sender<u64>>>,
}

impl LocalSyncHub {
    pub fn new() -> Self {
        LocalSyncHub::default()
    }

    fn with<T>(&self, workspace: &str, f: impl FnOnce(&watch::Sender<u64>) -> T) -> T {
        let mut map = self.chans.lock().unwrap_or_else(|p| p.into_inner());
        let tx = map.entry(workspace.to_string()).or_insert_with(|| watch::Sender::new(0));
        f(tx)
    }
}

impl SyncHub for LocalSyncHub {
    fn publish(&self, workspace: &str, rev: u64) {
        self.with(workspace, |tx| {
            tx.send_replace(rev);
        });
    }

    fn subscribe(&self, workspace: &str) -> watch::Receiver<u64> {
        self.with(workspace, watch::Sender::subscribe)
    }
}

pub fn routes() -> Router<ApiState> {
    Router::new().route("/api/sync/snapshot", get(snapshot)).route("/api/sync/events", get(events))
}

fn client_param(q: &Q) -> Result<Option<String>, ApiError> {
    match q.get("client") {
        None | Some("") => Ok(None),
        Some(c) if proto::Origin::valid_client(c) => Ok(Some(c.to_string())),
        Some(_) => Err(ApiError::bad(Flavor::Native, "'client' must be 1-64 of A-Z a-z 0-9 - _")),
    }
}

async fn snapshot(cx: Ctx, q: Q) -> Result<Response, ApiError> {
    let client = client_param(&q)?;
    let epoch = cx.hosted.epoch().to_string();
    let user = cx.user.clone();
    cx.run(move |tx| {
        let track = trackcraft_store::to_string(tx.ws()).map_err(|e| ApiError::internal(Flavor::Native, e.to_string()))?;
        let applied = client.and_then(|c| tx.s.journal().and_then(|j| j.applied_seq(&c)));
        let head = json!({"protocol": proto::PROTOCOL, "epoch": epoch, "rev": tx.s.sync_rev(), "user": user, "appliedSeq": applied});
        // Splice the `.track` JSON in as it is (no second parse of a large workspace).
        let mut body = head.to_string();
        body.pop();
        body.push_str(",\"workspace\":");
        body.push_str(&track);
        body.push('}');
        Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
    })
    .await
}

fn json_event(name: &'static str, data: &impl serde::Serialize) -> Event {
    Event::default().event(name).data(serde_json::to_string(data).unwrap_or_else(|_| "{}".into()))
}

fn resync_event(epoch: &str, rev: u64, reason: &str) -> Event {
    json_event(proto::EVENT_RESYNC, &proto::Resync { epoch: epoch.to_string(), rev, reason: reason.to_string() })
}

struct Stream {
    hosted: Arc<Hosted>,
    rx: watch::Receiver<u64>,
    since: u64,
    queue: VecDeque<Event>,
    done: bool,
}

async fn events(cx: Ctx, q: Q) -> Result<Response, ApiError> {
    let since = match q.get("since") {
        None | Some("") => None,
        Some(s) => Some(s.trim().parse::<u64>().map_err(|_| ApiError::bad(Flavor::Native, "'since' must be a revision number"))?),
    };
    let want_epoch = q.get("epoch").filter(|e| !e.is_empty()).map(|e| e.chars().take(64).collect::<String>());
    // Subscribe before reading, so a change between the read and the wait is not missed.
    let rx = cx.state.sync.subscribe(cx.hosted.name());
    let (rev, oldest) = cx.run(|tx| Ok((tx.s.sync_rev(), tx.s.journal().map(|j| j.oldest()).unwrap_or(0)))).await?;
    let epoch = cx.hosted.epoch().to_string();
    let since = since.unwrap_or(rev);
    let mut queue = VecDeque::new();
    queue.push_back(json_event(proto::EVENT_HELLO, &proto::Hello { protocol: proto::PROTOCOL, epoch: epoch.clone(), rev, oldest }));
    let reason = match want_epoch {
        Some(e) if e != epoch => Some("the server's journal started over (restart)"),
        _ if since > rev => Some("that revision is ahead of the server"),
        _ if since < oldest => Some("that revision is older than the server's journal"),
        _ => None,
    };
    let done = if let Some(r) = reason {
        queue.push_back(resync_event(&epoch, rev, r));
        true
    } else {
        false
    };
    let st = Stream { hosted: Arc::clone(&cx.hosted), rx, since, queue, done };
    let stream = futures_util::stream::unfold(st, step);
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE)).into_response())
}

async fn step(mut st: Stream) -> Option<(Result<Event, Infallible>, Stream)> {
    loop {
        if let Some(ev) = st.queue.pop_front() {
            return Some((Ok(ev), st));
        }
        if st.done {
            return None;
        }
        let (h, since) = (Arc::clone(&st.hosted), st.since);
        let read = tokio::task::spawn_blocking(move || {
            let s = h.lock();
            (s.journal_since(since, BATCH), s.sync_rev())
        })
        .await;
        match read {
            Err(_) => return None,
            Ok((Err(e), rev)) => {
                let reason = match e {
                    SinceError::TooOld { .. } => "that revision is older than the server's journal",
                    SinceError::Future { .. } => "that revision is ahead of the server",
                };
                st.queue.push_back(resync_event(st.hosted.epoch(), rev, reason));
                st.done = true;
            }
            Ok((Ok(entries), _)) if entries.is_empty() => {
                if st.rx.changed().await.is_err() {
                    return None;
                }
            }
            Ok((Ok(entries), _)) => {
                for e in entries {
                    st.since = e.rev;
                    st.queue.push_back(json_event(proto::EVENT_ENTRY, &e).id(e.rev.to_string()));
                }
            }
        }
    }
}
