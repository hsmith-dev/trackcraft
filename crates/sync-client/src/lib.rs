//! TrackCraft live-sync client (L5, blocking, std only): keeps a desktop [`Session`] in step with
//! a `trackcraft-server` workspace (`docs/sync.md`).
//!
//! - [`Remote::connect`] fetches the snapshot and returns the session the UI edits (in remote
//!   mode: no undo, no host-file commands) plus the [`Remote`] that syncs it.
//! - The UI keeps calling `Session::run`. The session's after-command observer queues every
//!   local change; [`Remote::pump`] (call it every frame) sends them in order on a background
//!   thread and applies what the server streams back.
//! - **Optimistic with server order.** The client keeps a second, *confirmed* session that only
//!   ever applies server journal entries, in revision order. The visible session is the
//!   confirmed one plus the local commands not yet confirmed, re-run on top whenever someone
//!   else's change arrives first (a rebase), so every client ends on exactly the server's
//!   workspace once its queue drains: the server's order wins conflicts.
//! - **Offline.** Commands queue while the server is unreachable and are resent in order on
//!   reconnect; the server drops a resend it already applied (origin `client` + `seq`). A
//!   command the server refuses is dropped and reported ([`Remote::pump`]'s notices).
//! - **Resync.** A revision gap, a replay that fails, a server restart (new epoch) or a journal
//!   that no longer reaches back far enough reloads the confirmed copy from a snapshot; the
//!   visible session keeps the user's view state (only its workspace is replaced).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod http;

use std::collections::{BTreeMap, VecDeque};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use trackcraft_engine::{Executed, Session};
use trackcraft_sync_proto::{self as proto, CommandRequest, CommandResponse, Entry, Origin, Snapshot, SseParser};

use http::Endpoint;

/// Largest snapshot accepted (the store's own file cap applies too).
pub const MAX_SNAPSHOT: usize = 256 * 1024 * 1024;
/// Largest command response.
const MAX_RESPONSE: usize = 32 * 1024 * 1024;
/// Out-of-order entries held while waiting for the one before them.
const MAX_BUFFERED: usize = 4096;
/// Commands queued (not yet confirmed) before new local changes are refused by a resync.
pub const MAX_PENDING: usize = 10_000;

/// Where and as whom to connect.
#[derive(Debug, Clone)]
pub struct Config {
    /// `https://host[:port][/prefix]`, or `http://…` (meant for a server on this computer).
    pub server: String,
    /// The hosted workspace (`/w/{name}`); `None` = the server's default.
    pub workspace: Option<String>,
    pub token: String,
    /// Connect / request timeout.
    pub timeout: Duration,
    /// Longest silence on the event stream before reconnecting (the server pings every 15 s).
    pub idle: Duration,
    /// Longest wait between reconnect attempts.
    pub max_backoff: Duration,
    /// M4.7: for `https://`, trust only these root certificates (DER) instead of the system's
    /// trust store (a private CA; tests). Empty = the system store.
    pub ca_certs: Vec<Vec<u8>>,
}

impl Config {
    pub fn new(server: &str, workspace: Option<&str>, token: &str) -> Self {
        Config {
            server: server.to_string(),
            workspace: workspace.map(str::to_string),
            token: token.trim().to_string(),
            timeout: Duration::from_secs(20),
            idle: Duration::from_secs(45),
            max_backoff: Duration::from_secs(10),
            ca_certs: Vec::new(),
        }
    }
}

/// Connection state, for a status indicator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    /// Event stream up and the last command (if any) delivered.
    Online,
    /// The server can't be reached; changes are queued.
    Offline { reason: String },
    /// Reloading the workspace from a snapshot.
    Resyncing,
    /// The server rejected the token; changes are queued.
    Unauthorized { reason: String },
}

/// Something worth telling the user about (a toast).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Connection state changed.
    State(ConnState),
    /// The server refused queued commands; they were dropped. One line per command.
    Dropped(Vec<String>),
    /// The local copy diverged and is being reloaded.
    Resync(String),
}

impl Notice {
    /// A one-line message for a toast.
    pub fn message(&self, server: &str) -> String {
        match self {
            Notice::State(ConnState::Online) => format!("Connected to {server}"),
            Notice::State(ConnState::Offline { reason }) => format!("Offline ({reason}): changes are queued and sent on reconnect"),
            Notice::State(ConnState::Resyncing) => "Reloading the workspace from the server".into(),
            Notice::State(ConnState::Unauthorized { reason }) => format!("The server refused the token ({reason}); changes are queued"),
            Notice::Dropped(lines) => format!("The server refused {} change(s), which were dropped: {}", lines.len(), lines.join("; ")),
            Notice::Resync(why) => format!("Out of sync with the server ({why}); reloading. Your view is kept."),
        }
    }
}

/// A local command sent but not yet confirmed.
#[derive(Debug, Clone)]
struct Pending {
    seq: u64,
    command: String,
    params: Value,
}

impl Pending {
    fn label(&self) -> String {
        let target = ["key", "id", "project", "space"].iter().find_map(|k| match self.params.get(k) {
            Some(Value::String(s)) => Some(s.chars().take(40).collect::<String>()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        });
        match target {
            Some(t) => format!("{} {t}", self.command),
            None => self.command.clone(),
        }
    }
}

struct Outgoing {
    seq: u64,
    command: String,
    params: Value,
}

enum Incoming {
    Entry(Entry),
    Ack(u64, CommandResponse),
    Rejected(u64, String),
    Snapshot(Box<Snapshot>),
    /// The stream asked for a resync; the event thread is fetching a snapshot.
    ResyncStarted,
    Events(Result<(), String>),
    Commands(Result<(), String>),
    Unauthorized(String),
}

/// Shared between the UI side and the network threads.
struct Shared {
    stop: AtomicBool,
    resync: AtomicBool,
    /// Bumped by [`Remote::reconnect_now`]: cuts retry waits short (M4.7).
    kick: AtomicU64,
    stream: Mutex<Option<TcpStream>>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Shared {
    fn wake(&self) {
        if let Some(w) = &self.wake {
            w();
        }
    }

    fn interrupt_stream(&self) {
        if let Some(s) = self.stream.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            http::close(s);
        }
    }

    /// Sleeps up to `d`, returning early on stop, on [`Remote::reconnect_now`] (then `true`)
    /// or, with `on_resync`, on a resync request.
    fn sleep(&self, d: Duration, on_resync: bool) -> bool {
        let end = Instant::now() + d;
        let kick = self.kick.load(Ordering::SeqCst);
        while Instant::now() < end && !self.stop.load(Ordering::SeqCst) && !(on_resync && self.resync.load(Ordering::SeqCst)) {
            if self.kick.load(Ordering::SeqCst) != kick {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// Waits before the next retry: `wait`, doubled up to `max` afterwards; a reconnect request
    /// cuts the wait short and starts the backoff over.
    fn retry_wait(&self, wait: &mut Duration, max: Duration, on_resync: bool) {
        if self.sleep(*wait, on_resync) {
            *wait = Duration::from_millis(250);
        } else {
            *wait = backoff(*wait, max);
        }
    }
}

struct Net {
    ep: Endpoint,
    token: String,
    client: String,
    cfg: Config,
    shared: Arc<Shared>,
    tx: Sender<Incoming>,
}

impl Net {
    fn send(&self, m: Incoming) {
        let _ = self.tx.send(m);
        self.shared.wake();
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }
}

fn fetch_snapshot(ep: &Endpoint, token: &str, client: &str, timeout: Duration) -> Result<Snapshot, (u16, String)> {
    let r = http::request(ep, "GET", &format!("/api/sync/snapshot?client={client}"), token, None, timeout, MAX_SNAPSHOT).map_err(|e| (0, e))?;
    if r.status != 200 {
        return Err((r.status, r.error_message()));
    }
    let snap: Snapshot = serde_json::from_slice(&r.body).map_err(|e| (0, format!("bad snapshot: {e}")))?;
    if snap.protocol != proto::PROTOCOL {
        return Err((0, format!("the server speaks sync protocol {}, this client {}", snap.protocol, proto::PROTOCOL)));
    }
    Ok(snap)
}

fn workspace_of(snap: &Snapshot) -> Result<trackcraft_engine::model::Workspace, String> {
    trackcraft_store::from_str(&snap.workspace.to_string()).map_err(|e| format!("the server's workspace did not load: {e}"))
}

fn backoff(d: Duration, max: Duration) -> Duration {
    d.saturating_mul(2).min(max)
}

/// The command thread: sends queued commands in order, retrying until delivered or refused.
fn command_loop(net: Net, rx: Receiver<Outgoing>) {
    while let Ok(out) = rx.recv() {
        let body = CommandRequest { command: out.command, params: out.params, origin: Some(Origin { client: net.client.clone(), seq: out.seq }) };
        let body = match serde_json::to_vec(&body) {
            Ok(b) => b,
            Err(e) => {
                net.send(Incoming::Rejected(out.seq, e.to_string()));
                continue;
            }
        };
        let mut wait = Duration::from_millis(250);
        loop {
            if net.stopped() {
                return;
            }
            match http::request(&net.ep, "POST", "/api/command", &net.token, Some(&body), net.cfg.timeout, MAX_RESPONSE) {
                Ok(r) if r.status == 200 => match serde_json::from_slice::<CommandResponse>(&r.body) {
                    Ok(resp) => {
                        net.send(Incoming::Commands(Ok(())));
                        net.send(Incoming::Ack(out.seq, resp));
                        break;
                    }
                    Err(e) => {
                        net.send(Incoming::Rejected(out.seq, format!("bad response: {e}")));
                        break;
                    }
                },
                Ok(r) if r.status == 401 => net.send(Incoming::Unauthorized(r.error_message())),
                Ok(r) if r.status == 408 || r.status == 429 || r.status >= 500 => net.send(Incoming::Commands(Err(r.error_message()))),
                Ok(r) => {
                    net.send(Incoming::Commands(Ok(())));
                    net.send(Incoming::Rejected(out.seq, r.error_message()));
                    break;
                }
                Err(e) => net.send(Incoming::Commands(Err(e))),
            }
            net.shared.retry_wait(&mut wait, net.cfg.max_backoff, false);
        }
    }
}

/// The event thread: follows the journal over SSE, reconnecting and resyncing as needed.
fn event_loop(net: Net, mut since: u64, mut epoch: String) {
    let mut need_snapshot = false;
    let mut wait = Duration::from_millis(250);
    while !net.stopped() {
        if net.shared.resync.swap(false, Ordering::SeqCst) {
            need_snapshot = true;
        }
        if need_snapshot {
            match fetch_snapshot(&net.ep, &net.token, &net.client, net.cfg.timeout) {
                Ok(snap) => {
                    since = snap.rev;
                    epoch = snap.epoch.clone();
                    need_snapshot = false;
                    net.send(Incoming::Snapshot(Box::new(snap)));
                }
                Err((401, m)) => net.send(Incoming::Unauthorized(m)),
                Err((_, m)) => net.send(Incoming::Events(Err(m))),
            }
            if need_snapshot {
                net.shared.retry_wait(&mut wait, net.cfg.max_backoff, false);
                continue;
            }
        }
        let path = format!("/api/sync/events?since={since}&epoch={epoch}&client={}", net.client);
        let mut stream = match http::open_stream(&net.ep, &path, &net.token, net.cfg.timeout, net.cfg.idle) {
            Ok(s) if s.status == 200 => s,
            Ok(s) if s.status == 401 => {
                net.send(Incoming::Unauthorized(format!("HTTP {}", s.status)));
                net.shared.retry_wait(&mut wait, net.cfg.max_backoff, true);
                continue;
            }
            Ok(s) => {
                net.send(Incoming::Events(Err(format!("HTTP {}", s.status))));
                net.shared.retry_wait(&mut wait, net.cfg.max_backoff, true);
                continue;
            }
            Err(e) => {
                net.send(Incoming::Events(Err(e)));
                net.shared.retry_wait(&mut wait, net.cfg.max_backoff, true);
                continue;
            }
        };
        if let Ok(h) = stream.handle.try_clone() {
            *net.shared.stream.lock().unwrap_or_else(|p| p.into_inner()) = Some(h);
        }
        // A resync requested while connecting: drop this stream at once.
        if net.shared.resync.load(Ordering::SeqCst) || net.stopped() {
            http::close(&stream.handle);
        }
        let mut parser = SseParser::new();
        let why = loop {
            let line = match stream.line(proto::MAX_EVENT_BYTES + 64) {
                Ok(Some(l)) => l,
                Ok(None) => break "the server closed the event stream".to_string(),
                Err(e) => break e,
            };
            let Some(ev) = parser.feed_line(&line) else { continue };
            if ev.truncated {
                need_snapshot = true;
                break "an event was too large".into();
            }
            match ev.event.as_str() {
                proto::EVENT_HELLO => {
                    wait = Duration::from_millis(250);
                    net.send(Incoming::Events(Ok(())));
                }
                proto::EVENT_ENTRY => match serde_json::from_str::<Entry>(&ev.data) {
                    Ok(e) => {
                        since = e.rev;
                        net.send(Incoming::Entry(e));
                    }
                    Err(_) => {
                        need_snapshot = true;
                        break "the server sent an unreadable entry".into();
                    }
                },
                proto::EVENT_RESYNC => {
                    need_snapshot = true;
                    net.send(Incoming::ResyncStarted);
                    break String::new();
                }
                _ => {}
            }
        };
        *net.shared.stream.lock().unwrap_or_else(|p| p.into_inner()) = None;
        if net.stopped() {
            return;
        }
        if !need_snapshot && !net.shared.resync.load(Ordering::SeqCst) {
            net.send(Incoming::Events(Err(why)));
            net.shared.retry_wait(&mut wait, net.cfg.max_backoff, true);
        }
    }
}

/// A random client id (per process): 16 hex characters from the OS source.
fn client_id() -> String {
    let mut b = [0u8; 8];
    if getrandom::fill(&mut b).is_err() {
        // Fall back to the clock; uniqueness per process is all that matters.
        let t = trackcraft_engine::system_now();
        b = t.timestamp_nanos_opt().unwrap_or_default().to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The sync side of a remote session (owned by the UI thread).
pub struct Remote {
    server: String,
    /// `https://` (M4.7).
    secure: bool,
    /// The server is on this computer.
    loopback: bool,
    client: String,
    confirmed: Session,
    pending: VecDeque<Pending>,
    next_seq: u64,
    buffered: BTreeMap<u64, Entry>,
    awaiting_snapshot: bool,
    outbox: Arc<Mutex<Vec<Executed>>>,
    to_net: Option<Sender<Outgoing>>,
    from_net: Receiver<Incoming>,
    shared: Arc<Shared>,
    events: Result<(), String>,
    commands: Result<(), String>,
    unauthorized: Option<String>,
    state: ConnState,
}

impl Remote {
    /// Fetches the workspace and starts syncing. Returns the session for the UI (remote mode,
    /// acting as the token's user) and the sync handle. `wake` is called from the network
    /// threads whenever [`Remote::pump`] has work (e.g. `egui::Context::request_repaint`).
    pub fn connect(cfg: Config, wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Result<(Remote, Session), String> {
        let mut ep = Endpoint::parse(&cfg.server, cfg.workspace.as_deref())?;
        if !cfg.ca_certs.is_empty() {
            ep = ep.with_ca_certs(&cfg.ca_certs)?; // M4.7
        }
        if !http::valid_token(&cfg.token) {
            return Err("the token is empty or contains spaces or control characters".into());
        }
        let client = client_id();
        let snap = fetch_snapshot(&ep, &cfg.token, &client, cfg.timeout).map_err(|(code, m)| match code {
            401 => format!("{}: the server refused the token ({m})", ep.display()),
            // The transport's message already names the host (certificate errors included).
            0 => m,
            c => format!("{}: HTTP {c}: {m}", ep.display()),
        })?;
        let ws = workspace_of(&snap)?;
        let mut confirmed = Session::new(ws.clone(), &snap.user);
        confirmed.load_synced(ws.clone(), snap.rev);
        confirmed.set_remote(true);
        let mut view = Session::new(ws.clone(), &snap.user);
        view.load_synced(ws, snap.rev);
        view.set_remote(true);
        let outbox: Arc<Mutex<Vec<Executed>>> = Arc::default();
        let sink = Arc::clone(&outbox);
        view.set_after_command(Some(Box::new(move |e| sink.lock().unwrap_or_else(|p| p.into_inner()).push(e.clone()))));

        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            resync: AtomicBool::new(false),
            kick: AtomicU64::new(0),
            stream: Mutex::new(None),
            wake,
        });
        let (in_tx, in_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let net = |tx: Sender<Incoming>| Net {
            ep: ep.clone(),
            token: cfg.token.clone(),
            client: client.clone(),
            cfg: cfg.clone(),
            shared: Arc::clone(&shared),
            tx,
        };
        let (ev_net, cmd_net) = (net(in_tx.clone()), net(in_tx));
        let (since, epoch) = (snap.rev, snap.epoch.clone());
        std::thread::Builder::new()
            .name("trackcraft-sync-events".into())
            .spawn(move || event_loop(ev_net, since, epoch))
            .map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("trackcraft-sync-commands".into())
            .spawn(move || command_loop(cmd_net, out_rx))
            .map_err(|e| e.to_string())?;
        let remote = Remote {
            server: ep.display(),
            secure: ep.tls.is_some(),
            loopback: ep.is_loopback(),
            client,
            confirmed,
            pending: VecDeque::new(),
            next_seq: 1,
            buffered: BTreeMap::new(),
            awaiting_snapshot: false,
            outbox,
            to_net: Some(out_tx),
            from_net: in_rx,
            shared,
            events: Err("connecting".into()),
            commands: Ok(()),
            unauthorized: None,
            state: ConnState::Offline { reason: "connecting".into() },
        };
        Ok((remote, view))
    }

    /// `https://host:port/w/name` (or `http://…`).
    pub fn server(&self) -> &str {
        &self.server
    }

    /// The connection is encrypted (`https://`) (M4.7).
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Plain `http://` to another computer: the token and workspace cross the network in the
    /// clear (worth a warning in the UI) (M4.7).
    pub fn unencrypted_remote(&self) -> bool {
        !self.secure && !self.loopback
    }

    /// The last connection error, if not online (for a status tooltip) (M4.7).
    pub fn last_error(&self) -> Option<&str> {
        match &self.state {
            ConnState::Offline { reason } | ConnState::Unauthorized { reason } => Some(reason),
            ConnState::Online | ConnState::Resyncing => None,
        }
    }

    /// Retries now instead of waiting out the backoff ("Reconnect now") (M4.7).
    pub fn reconnect_now(&self) {
        self.shared.kick.fetch_add(1, Ordering::SeqCst);
    }

    pub fn state(&self) -> &ConnState {
        &self.state
    }

    /// Local changes not yet confirmed by the server.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// The last server revision applied.
    pub fn rev(&self) -> u64 {
        self.confirmed.sync_rev()
    }

    /// The server's workspace as last confirmed (what `view` converges to once nothing is pending).
    pub fn confirmed(&self) -> &Session {
        &self.confirmed
    }

    /// A short status for a status bar or window title: `Online`, `Offline · 3 queued`, …
    pub fn status_text(&self) -> String {
        let base = match &self.state {
            ConnState::Online => "Online".to_string(),
            ConnState::Offline { .. } => "Offline".to_string(),
            ConnState::Resyncing => "Resyncing".to_string(),
            ConnState::Unauthorized { .. } => "Token refused".to_string(),
        };
        if self.pending.is_empty() { base } else { format!("{base} · {} unsent", self.pending.len()) }
    }

    /// Reloads the confirmed copy from a snapshot (then rebuilds `view`).
    pub fn resync(&mut self) {
        self.awaiting_snapshot = true;
        self.shared.resync.store(true, Ordering::SeqCst);
        self.shared.interrupt_stream();
    }

    /// Sends local changes and applies the server's. Call every frame (and when woken) with the
    /// session [`Remote::connect`] returned. Returns what to tell the user.
    pub fn pump(&mut self, view: &mut Session) -> Vec<Notice> {
        let mut notices = Vec::new();
        let mut rebuild = false;
        let mut dropped = Vec::new();
        // 1. Local changes, in the order they ran.
        let local: Vec<Executed> = std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|p| p.into_inner()));
        for e in local {
            if self.pending.len() >= MAX_PENDING {
                dropped.push(format!("{} (too many unsent changes)", e.command));
                rebuild = true;
                continue;
            }
            let p = Pending { seq: self.next_seq, command: e.command, params: e.params };
            self.next_seq = self.next_seq.saturating_add(1);
            if let Some(tx) = &self.to_net {
                let _ = tx.send(Outgoing { seq: p.seq, command: p.command.clone(), params: p.params.clone() });
            }
            self.pending.push_back(p);
        }
        // 2. Whatever the network threads delivered.
        let incoming: Vec<Incoming> = self.from_net.try_iter().take(10_000).collect();
        for m in incoming {
            match m {
                Incoming::Entry(e) => {
                    if self.awaiting_snapshot {
                        continue;
                    }
                    let rev = e.rev;
                    self.buffer(e);
                    rebuild |= self.advance(view, &mut notices);
                    // The stream is in order: if this entry still can't be applied, revisions are missing.
                    if !self.awaiting_snapshot && rev > self.confirmed.sync_rev() {
                        self.request_resync(&mut notices, format!("missing revisions before {rev}"));
                    }
                }
                Incoming::Ack(seq, resp) => {
                    if resp.duplicate {
                        continue;
                    }
                    match resp.entry {
                        Some(e) => {
                            self.buffer(e);
                            if !self.awaiting_snapshot {
                                rebuild |= self.advance(view, &mut notices);
                            }
                        }
                        // Ran on the server but changed nothing there.
                        None => rebuild |= self.forget(seq).is_some(),
                    }
                }
                Incoming::Rejected(seq, why) => {
                    if let Some(p) = self.forget(seq) {
                        dropped.push(format!("{}: {why}", p.label()));
                        rebuild = true;
                    }
                }
                Incoming::Snapshot(snap) => match workspace_of(&snap) {
                    Ok(ws) => {
                        self.confirmed.load_synced(ws, snap.rev);
                        self.awaiting_snapshot = false;
                        if let Some(applied) = snap.applied_seq {
                            self.pending.retain(|p| p.seq > applied);
                        }
                        let rev = snap.rev;
                        self.buffered.retain(|r, _| *r > rev);
                        rebuild = true;
                        rebuild |= self.advance(view, &mut notices);
                    }
                    Err(e) => self.request_resync(&mut notices, e),
                },
                Incoming::ResyncStarted => self.awaiting_snapshot = true,
                Incoming::Events(r) => {
                    if r.is_ok() {
                        self.unauthorized = None;
                    }
                    self.events = r;
                }
                Incoming::Commands(r) => {
                    if r.is_ok() {
                        self.unauthorized = None;
                    }
                    self.commands = r;
                }
                Incoming::Unauthorized(m) => self.unauthorized = Some(m),
            }
        }
        if rebuild {
            self.rebuild(view);
        }
        if !dropped.is_empty() {
            notices.push(Notice::Dropped(dropped));
        }
        let state = match (&self.unauthorized, &self.events, &self.commands) {
            (Some(m), _, _) => ConnState::Unauthorized { reason: m.clone() },
            _ if self.awaiting_snapshot => ConnState::Resyncing,
            (None, Err(e), _) | (None, Ok(()), Err(e)) => ConnState::Offline { reason: e.chars().take(120).collect() },
            (None, Ok(()), Ok(())) => ConnState::Online,
        };
        if state != self.state {
            // "connecting" → online is not news; everything else is.
            let first = matches!(&self.state, ConnState::Offline { reason } if reason == "connecting");
            if !(first && state == ConnState::Online)
                && !(matches!(state, ConnState::Offline { .. }) && matches!(self.state, ConnState::Offline { .. }))
            {
                notices.push(Notice::State(state.clone()));
            }
            self.state = state;
        }
        notices
    }

    fn buffer(&mut self, e: Entry) {
        if e.rev > self.confirmed.sync_rev() && self.buffered.len() < MAX_BUFFERED {
            self.buffered.insert(e.rev, e);
        }
    }

    fn forget(&mut self, seq: u64) -> Option<Pending> {
        let i = self.pending.iter().position(|p| p.seq == seq)?;
        self.pending.remove(i)
    }

    fn request_resync(&mut self, notices: &mut Vec<Notice>, why: String) {
        if !self.awaiting_snapshot {
            notices.push(Notice::Resync(why));
        }
        self.buffered.clear();
        self.resync();
    }

    /// Applies buffered entries in revision order. Returns whether `view` must be rebuilt.
    fn advance(&mut self, view: &mut Session, notices: &mut Vec<Notice>) -> bool {
        let mut rebuild = false;
        let next = |s: &Session| s.sync_rev().saturating_add(1);
        while let Some(e) = self.buffered.remove(&next(&self.confirmed)) {
            if let Err(err) = self.confirmed.apply_remote(&e) {
                self.request_resync(notices, err.to_string());
                return false;
            }
            let ours = e.origin.as_ref().filter(|o| o.client == self.client).map(|o| o.seq);
            match ours {
                Some(seq) => {
                    let was_first = self.pending.front().is_some_and(|p| p.seq == seq);
                    self.forget(seq);
                    // Our own echo: `view` already shows it unless the order changed. Once the
                    // queue drains, rebuild so `view` is exactly the server's (its timestamps).
                    if !was_first || self.pending.is_empty() {
                        rebuild = true;
                    }
                }
                None if self.pending.is_empty() && !rebuild => {
                    // Nobody's local change in the way: replay straight onto the view.
                    if !matches!(view.replay(&e.command, &e.params, &e.actor, e.at), Ok((_, true))) {
                        rebuild = true;
                    }
                }
                None => rebuild = true,
            }
        }
        let rev = self.confirmed.sync_rev();
        self.buffered.retain(|r, _| *r > rev);
        rebuild
    }

    /// `view` = confirmed + pending (each pending command re-run on top, as its user).
    fn rebuild(&mut self, view: &mut Session) {
        view.set_workspace(self.confirmed.ws.clone());
        let (user, now) = (view.user.clone(), view.now());
        for p in &self.pending {
            // A pending command that no longer applies stays queued: the server decides.
            let _ = view.replay(&p.command, &p.params, &user, now);
        }
    }

    /// Stops the network threads (also on drop). Unsent changes are lost.
    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.to_net = None;
        self.shared.interrupt_stream();
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
