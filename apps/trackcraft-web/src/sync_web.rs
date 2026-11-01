//! The browser transport for [`crate::sync::Sync`] (M4.6): `fetch` for the snapshot, stream
//! tickets and commands, `EventSource` for the journal stream, and the window's
//! `online`/`offline` events. Every decision is made by `Sync::step`; this file only carries out
//! its [`Action`]s and reports back [`Net`]s.
//!
//! JS callbacks (fetch completions, stream events) only push onto an inbox and ask egui for a
//! repaint; streams are opened, closed and dropped from [`Connection::pump`] (a frame), never
//! inside a callback of their own.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use trackcraft_engine::Session;
use trackcraft_sync_proto::{CommandResponse, Snapshot};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

use crate::remote::{self, ServerWorkspace};
use crate::sync::{self, Action, Failure, MAX_SNAPSHOT, Net, Notice, Sync};

/// How long a snapshot request may take (ms), and any other request.
const SNAPSHOT_TIMEOUT_MS: i32 = 120_000;
const REQUEST_TIMEOUT_MS: i32 = 30_000;
/// Largest answer to a command or ticket request.
const MAX_RESPONSE: usize = 32 * 1024 * 1024;

fn js(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

type Inbox = Rc<RefCell<Vec<Net>>>;

/// A random client id (per connection) from `crypto.getRandomValues`, else `Math.random`.
pub fn new_client_id() -> String {
    let mut b = [0u8; 8];
    let ok = web_sys::window().and_then(|w| w.crypto().ok()).is_some_and(|c| c.get_random_values_with_u8_array(&mut b).is_ok());
    if !ok {
        for x in &mut b {
            *x = (js_sys::Math::random() * 256.0) as u8;
        }
    }
    remote::client_id(b)
}

/// The browser says it has no network.
pub fn browser_offline() -> bool {
    web_sys::window().is_some_and(|w| !w.navigator().on_line())
}

/// One request: `Authorization: Bearer`, same-origin, not cached, aborted after `timeout_ms`.
/// Answers the body of a 2xx, else the status and the server's message (status 0 = no answer).
async fn request(method: &str, url: &str, token: &str, body: Option<&str>, timeout_ms: i32, max: usize) -> Result<String, Failure> {
    let net = |e: JsValue| Failure::new(0, js(e));
    let window = web_sys::window().ok_or_else(|| Failure::new(0, "no window"))?;
    let headers = web_sys::Headers::new().map_err(net)?;
    headers.set("Authorization", &format!("Bearer {token}")).map_err(net)?;
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    init.set_cache(web_sys::RequestCache::NoStore);
    if let Some(b) = body {
        headers.set("Content-Type", "application/json").map_err(net)?;
        init.set_body(&JsValue::from_str(b));
    }
    init.set_headers(&headers);
    // A request that hangs would stall the command queue: abort it after the timeout.
    let abort = web_sys::AbortController::new().ok();
    if let Some(a) = &abort {
        init.set_signal(Some(&a.signal()));
        let a = a.clone();
        let cb = Closure::once_into_js(move || a.abort());
        if let Err(e) = window.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), timeout_ms) {
            log::warn!("trackcraft-web: no request timeout: {}", js(e));
        }
    }
    let req = web_sys::Request::new_with_str_and_init(url, &init).map_err(net)?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| Failure::new(0, format!("the server can't be reached ({})", js(e).chars().take(80).collect::<String>())))?
        .dyn_into()
        .map_err(|_| Failure::new(0, "not a response"))?;
    let status = resp.status();
    let declared = resp.headers().get("content-length").ok().flatten().and_then(|v| v.trim().parse::<usize>().ok());
    if declared.is_some_and(|n| n > max) {
        return Err(Failure::new(0, format!("the answer is larger than {} MB", max >> 20)));
    }
    let text = JsFuture::from(resp.text().map_err(net)?).await.map_err(net)?.as_string().unwrap_or_default();
    if text.len() > max {
        return Err(Failure::new(0, format!("the answer is larger than {} MB", max >> 20)));
    }
    if (200..300).contains(&status) { Ok(text) } else { Err(Failure::new(status, remote::error_message(&text))) }
}

async fn fetch_snapshot(base: &str, token: &str, client: &str) -> Result<Snapshot, Failure> {
    let text = request("GET", &format!("{base}api/sync/snapshot?client={client}"), token, None, SNAPSHOT_TIMEOUT_MS, MAX_SNAPSHOT).await?;
    sync::parse_snapshot(&text).map_err(|e| Failure::new(0, e))
}

/// The open event stream and its handlers (dropped together, after `close`).
struct Stream {
    es: web_sys::EventSource,
    _handlers: Vec<Closure<dyn FnMut(web_sys::MessageEvent)>>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.es.close();
        self.es.set_onerror(None);
    }
}

/// A live connection to a server workspace.
pub struct Connection {
    pub target: ServerWorkspace,
    base: String,
    sync: Sync,
    inbox: Inbox,
    stream: Option<Stream>,
    /// Generation of the current stream: events of older ones are dropped.
    generation: Rc<Cell<u64>>,
    ctx: egui::Context,
    on_line: Vec<(&'static str, Closure<dyn FnMut(web_sys::Event)>)>,
}

impl Connection {
    /// Fetches the snapshot and starts syncing. Returns the connection and the session the UI
    /// edits.
    pub async fn open(target: ServerWorkspace, ctx: egui::Context) -> Result<(Connection, Session), String> {
        let token = sync::clean_token(&target.token)?;
        let base = sync::workspace_base(&target.workspace);
        let client = new_client_id();
        let snap = fetch_snapshot(&base, &token, &client).await.map_err(|f| match f.status {
            401 => "The server refused the token. Create one with: trackcraft-server token create --user NAME".to_string(),
            403 => format!("The token's user is not a member of this workspace ({})", f.message),
            404 => format!("No such workspace on this server ({})", f.message),
            0 => format!("Couldn't load the workspace: {}", f.message),
            s => format!("Couldn't load the workspace: HTTP {s}: {}", f.message),
        })?;
        let (sync, session) = Sync::new(&client, &snap)?;
        let inbox: Inbox = Rc::default();
        let mut on_line = Vec::new();
        if let Some(window) = web_sys::window() {
            for (name, online) in [("online", true), ("offline", false)] {
                let (inbox, ctx) = (inbox.clone(), ctx.clone());
                let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                    inbox.borrow_mut().push(Net::Online(online));
                    ctx.request_repaint();
                });
                match window.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref()) {
                    Ok(()) => on_line.push((name, cb)),
                    Err(e) => log::warn!("trackcraft-web: no {name} events: {}", js(e)),
                }
            }
        }
        if browser_offline() {
            inbox.borrow_mut().push(Net::Online(false));
        }
        let target = ServerWorkspace { workspace: target.workspace.trim().to_string(), token };
        Ok((Connection { target, base, sync, inbox, stream: None, generation: Rc::new(Cell::new(0)), ctx, on_line }, session))
    }

    pub fn status_text(&self) -> String {
        self.sync.status_text()
    }

    pub fn pending(&self) -> usize {
        self.sync.pending()
    }

    /// One frame: steps the state machine and carries out what it asks. Returns the toasts.
    pub fn pump(&mut self, view: &mut Session, now: f64) -> Vec<Notice> {
        let nets = std::mem::take(&mut *self.inbox.borrow_mut());
        let step = self.sync.step(view, now, nets);
        for a in step.actions {
            match a {
                Action::FetchSnapshot => {
                    let (base, token, client) = (self.base.clone(), self.target.token.clone(), self.sync.client().to_string());
                    self.spawn(async move { Net::Snapshot(fetch_snapshot(&base, &token, &client).await.map(Box::new)) });
                }
                Action::FetchTicket => {
                    let (url, token) = (format!("{}api/sync/ticket", self.base), self.target.token.clone());
                    self.spawn(async move {
                        let r = request("POST", &url, &token, Some("{}"), REQUEST_TIMEOUT_MS, MAX_RESPONSE).await;
                        Net::Ticket(r.and_then(|t| {
                            remote::parse_ticket(&t).ok_or_else(|| Failure::new(0, "the server sent no stream ticket (is it older than this page?)"))
                        }))
                    });
                }
                Action::OpenStream { query, ticket } => self.open_stream(&query, &ticket),
                Action::CloseStream => self.close_stream(),
                Action::Post { seq, body } => {
                    let (url, token) = (format!("{}api/command", self.base), self.target.token.clone());
                    self.spawn(async move {
                        let r = request("POST", &url, &token, Some(&body), REQUEST_TIMEOUT_MS, MAX_RESPONSE).await;
                        let result =
                            r.and_then(|t| serde_json::from_str::<CommandResponse>(&t).map_err(|e| Failure::new(0, format!("bad response: {e}"))));
                        Net::Posted { seq, result }
                    });
                }
            }
        }
        if let Some(s) = step.wake_in {
            self.ctx.request_repaint_after(std::time::Duration::from_secs_f64(s.clamp(0.0, 60.0)));
        }
        step.notices
    }

    /// Runs `f` and puts its answer in the inbox.
    fn spawn(&self, f: impl std::future::Future<Output = Net> + 'static) {
        let (inbox, ctx) = (self.inbox.clone(), self.ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let n = f.await;
            inbox.borrow_mut().push(n);
            ctx.request_repaint();
        });
    }

    fn close_stream(&mut self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.stream = None;
    }

    fn open_stream(&mut self, query: &str, ticket: &str) {
        self.close_stream();
        let url = format!("{}api/sync/events?{query}&ticket={}", self.base, sync::pct(ticket));
        let es = match web_sys::EventSource::new(&url) {
            Ok(es) => es,
            Err(e) => {
                log::warn!("trackcraft-web: no event stream: {}", js(e));
                self.inbox.borrow_mut().push(Net::StreamError);
                self.ctx.request_repaint();
                return;
            }
        };
        let mine = self.generation.get();
        let mut handlers = Vec::new();
        for name in [trackcraft_sync_proto::EVENT_HELLO, trackcraft_sync_proto::EVENT_ENTRY, trackcraft_sync_proto::EVENT_RESYNC] {
            let (inbox, ctx, generation) = (self.inbox.clone(), self.ctx.clone(), self.generation.clone());
            let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                if generation.get() != mine {
                    return;
                }
                let data = e.data().as_string().unwrap_or_default();
                inbox.borrow_mut().push(Net::Stream { event: name.to_string(), data });
                ctx.request_repaint();
            });
            if let Err(e) = es.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref()) {
                log::warn!("trackcraft-web: no {name} listener: {}", js(e));
            }
            handlers.push(cb);
        }
        let (inbox, ctx, generation) = (self.inbox.clone(), self.ctx.clone(), self.generation.clone());
        // `EventSource` reconnects by itself with the same URL, whose ticket is spent: report the
        // first error instead and let `Sync` reconnect with a fresh ticket and `since`.
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if generation.get() != mine {
                return;
            }
            // Report once per stream.
            generation.set(generation.get().wrapping_add(1));
            inbox.borrow_mut().push(Net::StreamError);
            ctx.request_repaint();
        });
        es.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        self.stream = Some(Stream { es, _handlers: handlers, _on_error: on_error });
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close_stream();
        if let Some(window) = web_sys::window() {
            for (name, cb) in &self.on_line {
                let _ = window.remove_event_listener_with_callback(name, cb.as_ref().unchecked_ref());
            }
        }
    }
}
