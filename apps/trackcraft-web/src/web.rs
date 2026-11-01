//! The browser shell: the eframe web runner, File requests through the browser, the workspace kept
//! in `localStorage`, and — when served by a TrackCraft server — the connect dialog and live
//! editing of a server workspace (M4.6, [`crate::sync_web::Connection`]).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::json;
use trackcraft_engine::Session;
use trackcraft_ui_egui::{FileRequest, TrackApp, View};
use wasm_bindgen::JsCast as _;

use crate::browser::{self, Picked};
use crate::logic::{self, MAX_OPEN_BYTES, MAX_STORED_BYTES, STORAGE_KEY};
use crate::remote::{self, ServerWorkspace, TOKEN_KEY, WORKSPACE_KEY};
use crate::sync_web::Connection;

const CANVAS_ID: &str = "trackcraft_canvas";
const LOADING_ID: &str = "trackcraft_loading";
/// Shortest wait between two writes of the workspace to `localStorage` (typing a page fires many
/// changes; each write serializes the whole workspace).
const STORE_EVERY_SECS: f64 = 1.0;

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let unsafe_to_leave = Rc::new(Cell::new(false));
        if let Err(e) = browser::guard_unload(unsafe_to_leave.clone()) {
            log::error!("no unsaved-changes guard: {e}");
        }
        let mut options = eframe::WebOptions::default();
        if browser::has_flag("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("trackcraft-web: wgpu backend {:?}", rs.adapter.get_info().backend);
                    }
                    let (session, note) = first_session();
                    // Nothing is written to storage until something changes: `?sample` must not
                    // replace the workspace kept from an earlier visit.
                    let stored = Some((0, session.revision()));
                    let mut app = TrackApp::new(session);
                    if let Some(note) = note {
                        app.notify(note);
                    }
                    let server = Rc::new(RefCell::new(None));
                    browser::detect_server(server.clone(), cc.egui_ctx.clone());
                    Ok(Box::new(WebShell {
                        app,
                        picked: Picked::default(),
                        server,
                        server_noted: false,
                        conn: None,
                        dialog: ConnectDialog::default(),
                        connecting: Rc::default(),
                        title: String::new(),
                        generation: 0,
                        stored,
                        last_store: 0.0,
                        unsafe_to_leave,
                    }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>TrackCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

/// The workspace to start on: `?sample` asks for the sample; otherwise the copy kept in this
/// browser, else the sample. A stored copy that no longer reads is left in place (a later save
/// replaces it) and reported.
fn first_session() -> (Session, Option<String>) {
    if browser::has_flag("sample") {
        return (Session::sample(), None);
    }
    match browser::stored(STORAGE_KEY) {
        Some(text) => match logic::session_from_text(&text) {
            Ok(s) => (s, None),
            Err(e) => (Session::sample(), Some(format!("The workspace kept in this browser couldn't be read ({e}); opened the sample"))),
        },
        None => (Session::sample(), None),
    }
}

struct WebShell {
    app: TrackApp,
    /// A file from File › Open, once the browser has read it.
    picked: Picked,
    /// The TrackCraft server's version, when the page was served by one.
    server: Rc<RefCell<Option<String>>>,
    server_noted: bool,
    /// The server workspace being edited (M4.6); `None` = a local workspace.
    conn: Option<Connection>,
    dialog: ConnectDialog,
    /// Where a connection attempt delivers.
    connecting: Rc<RefCell<Option<Result<(Connection, Session), String>>>>,
    /// The document title last set.
    title: String,
    /// Bumped when the session is replaced (New, Open), so revision numbers of different
    /// sessions never compare equal.
    generation: u64,
    /// The (generation, revision) last written to `localStorage`.
    stored: Option<(u64, u64)>,
    /// When it was written (seconds, egui time).
    last_store: f64,
    /// Read by the `beforeunload` guard: changes exist that neither storage nor a download kept.
    unsafe_to_leave: Rc<Cell<bool>>,
}

/// "Connect to a server workspace".
#[derive(Default)]
struct ConnectDialog {
    open: bool,
    target: ServerWorkspace,
    remember: bool,
    busy: bool,
    error: Option<String>,
}

impl WebShell {
    /// Starts connecting to the dialog's workspace (the answer arrives in `connecting`).
    fn connect(&mut self, ctx: &egui::Context) {
        if let Err(e) = crate::sync::clean_token(&self.dialog.target.token) {
            self.dialog.error = Some(e);
            self.dialog.open = true;
            return;
        }
        self.dialog.busy = true;
        self.dialog.error = None;
        let (target, into, ctx) = (self.dialog.target.clone(), self.connecting.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let r = Connection::open(target, ctx.clone()).await;
            *into.borrow_mut() = Some(r);
            ctx.request_repaint();
        });
    }

    /// A finished connection attempt: switch the app to the server workspace, or show why not.
    fn take_connected(&mut self, ctx: &egui::Context) {
        let Some(result) = self.connecting.borrow_mut().take() else { return };
        self.dialog.busy = false;
        match result {
            Ok((conn, session)) => {
                // Keep the local workspace first: it comes back on disconnect.
                self.keep(ctx, true);
                if self.dialog.remember {
                    let kept = browser::store(TOKEN_KEY, &conn.target.token).and_then(|()| browser::store(WORKSPACE_KEY, &conn.target.workspace));
                    if let Err(e) = kept {
                        self.app.notify_error(format!("Couldn't remember the token on this device: {e}"));
                    }
                } else {
                    forget_token();
                }
                let user = session.user.clone();
                self.app = TrackApp::new(session);
                self.generation = self.generation.wrapping_add(1);
                self.app.notify(format!("Connected to {} as {user}: changes are shared live", conn.target.label()));
                self.conn = Some(conn);
                self.dialog.open = false;
                self.dialog.error = None;
            }
            Err(e) => {
                self.dialog.error = Some(e);
                self.dialog.open = true;
            }
        }
    }

    /// Back to the workspace kept in this browser. `logout` also forgets the token.
    fn disconnect(&mut self, logout: bool) {
        let Some(conn) = self.conn.take() else { return };
        let unsent = conn.pending();
        drop(conn);
        self.unsafe_to_leave.set(false);
        if logout {
            forget_token();
            self.dialog.target.token.clear();
            self.dialog.remember = false;
        }
        let (session, note) = first_session();
        self.app = TrackApp::new(session);
        self.generation = self.generation.wrapping_add(1);
        // The workspace just came from storage: nothing to write back.
        self.stored = Some((self.generation, self.app.session.revision()));
        let what = if logout { "Logged out" } else { "Disconnected" };
        let lost = if unsent > 0 { format!(" ({unsent} unsent change(s) were discarded)") } else { String::new() };
        self.app.notify(format!("{what}{lost}; editing the workspace kept in this browser"));
        if let Some(note) = note {
            self.app.notify(note);
        }
    }

    /// The connect dialog and the server panel (bottom right).
    fn server_ui(&mut self, ctx: &egui::Context) {
        if self.dialog.open {
            let mut submit = false;
            let mut local = false;
            egui::Window::new("Connect to a server workspace")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_max_width(380.0);
                    ui.label("This page was served by a TrackCraft server. Connect to edit one of its workspaces live with your team, or keep working on a local workspace.");
                    ui.add_space(6.0);
                    let l = ui.label("Workspace");
                    ui.add(egui::TextEdit::singleline(&mut self.dialog.target.workspace).hint_text("the server's default").desired_width(f32::INFINITY))
                        .labelled_by(l.id);
                    let l = ui.label("Token");
                    let token = ui
                        .add(egui::TextEdit::singleline(&mut self.dialog.target.token).password(true).hint_text("tct_...").desired_width(f32::INFINITY))
                        .labelled_by(l.id);
                    if token.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                    ui.small("Create one on the server: trackcraft-server token create --user NAME");
                    ui.checkbox(&mut self.dialog.remember, "Remember on this device");
                    ui.small(
                        "Keeps the token in this browser's local storage, where anyone using this browser profile can read it. \
                         Leave it unticked on shared computers. Log out forgets it.",
                    );
                    if let Some(e) = &self.dialog.error {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if self.dialog.busy {
                            ui.add_enabled(false, egui::Button::new("Connecting..."));
                        } else if ui.button("Connect").clicked() {
                            submit = true;
                        }
                        if ui.button("Work locally").clicked() {
                            local = true;
                        }
                    });
                });
            if submit && !self.dialog.busy {
                self.connect(ctx);
            }
            if local {
                self.dialog.open = false;
            }
        }
        let server_found = self.server.borrow().is_some();
        if !server_found && self.conn.is_none() {
            return;
        }
        let mut action = None;
        egui::Area::new(egui::Id::unique("trackcraft_server_panel")).anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -36.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| match &self.conn {
                    Some(c) => {
                        ui.label(format!("{} · {}", c.target.label(), c.status_text()));
                        if ui.button("Disconnect").clicked() {
                            action = Some(false);
                        }
                        if ui.button("Log out").on_hover_text("Disconnect and forget the token on this device").clicked() {
                            action = Some(true);
                        }
                    }
                    None => {
                        if !self.dialog.open && ui.button("Connect to a server workspace...").clicked() {
                            self.dialog.open = true;
                        }
                    }
                });
            });
        });
        if let Some(logout) = action {
            self.disconnect(logout);
        }
    }

    /// Server mode: one sync step per frame, its notices as toasts.
    fn pump(&mut self, ctx: &egui::Context) {
        let Some(conn) = self.conn.as_mut() else { return };
        let now = ctx.input(|i| i.time);
        for n in conn.pump(&mut self.app.session, now) {
            let msg = n.message(&conn.target.label());
            if n.is_error() {
                self.app.notify_error(msg);
            } else {
                self.app.notify(msg);
            }
        }
        // Unsent changes live only in this tab: ask before it closes.
        self.unsafe_to_leave.set(conn.pending() > 0);
    }

    /// `document.title`: the workspace, and in server mode the connection state.
    fn set_title(&mut self) {
        let status = self.conn.as_ref().map(|c| (c.target.label(), c.status_text()));
        let title = remote::title(&self.app.title(), status.as_ref().map(|(l, s)| (l.as_str(), s.as_str())));
        if title != self.title {
            if let Some(d) = web_sys::window().and_then(|w| w.document()) {
                d.set_title(&title);
            }
            self.title = title;
        }
    }

    fn file_request(&mut self, req: FileRequest, ctx: &egui::Context) {
        match req {
            FileRequest::New => {
                if let Err(e) = self.app.session.run("workspace.new", &json!({"name": "New workspace"})) {
                    self.app.notify(e);
                }
                self.generation = self.generation.wrapping_add(1);
                self.app.navigate(View::Home);
            }
            FileRequest::Open => {
                if let Err(e) = browser::pick_text_file(".track,application/json", MAX_OPEN_BYTES, self.picked.clone(), ctx.clone()) {
                    self.app.notify(format!("Can't open the file picker: {e}"));
                }
            }
            // The browser has no paths: Save and Save As both download `<name>.track`.
            FileRequest::Save | FileRequest::SaveAs => {
                let name = logic::file_name(&self.app.session.ws.name);
                match logic::to_text(&self.app.session).and_then(|t| browser::download(&name, t.as_bytes(), "application/json")) {
                    Ok(()) => {
                        self.app.session.dirty = false;
                        self.app.notify(format!("Downloaded {name}"));
                    }
                    Err(e) => self.app.notify(format!("Couldn't save: {e}")),
                }
            }
            // `workspace.import` reads a file path, which the browser doesn't have. Importing in
            // the browser needs an import-from-bytes engine entry point (docs/gaps.md).
            FileRequest::Import(_) => self.app.notify("Import works in the desktop app for now; open the imported .track file here."),
        }
    }

    /// Takes a file the picker delivered and opens it in place of the current workspace.
    fn take_picked(&mut self) {
        let Some(picked) = self.picked.borrow_mut().take() else { return };
        match picked.and_then(|(name, text)| logic::session_from_text(&text).map(|s| (name, s)).map_err(|e| format!("Couldn't open the file: {e}"))) {
            Ok((name, session)) => {
                if self.conn.is_some() {
                    self.disconnect(false);
                }
                self.app = TrackApp::new(session);
                self.generation = self.generation.wrapping_add(1);
                self.app.notify(format!("Opened {name}"));
            }
            Err(e) => self.app.notify(e),
        }
    }

    /// Keeps the workspace in `localStorage` (at most every [`STORE_EVERY_SECS`]), so a reload or
    /// a closed tab doesn't lose it. A workspace too large for storage stays in memory only.
    fn keep(&mut self, ctx: &egui::Context, force: bool) {
        // A server workspace is kept by the server, never in place of the local one.
        if self.conn.is_some() {
            return;
        }
        let key = (self.generation, self.app.session.revision());
        if self.stored == Some(key) {
            self.unsafe_to_leave.set(false);
            return;
        }
        let now = ctx.input(|i| i.time);
        let wait = STORE_EVERY_SECS - (now - self.last_store);
        if !force && self.stored.is_some() && wait > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            return;
        }
        self.last_store = now;
        self.stored = Some(key);
        let kept = logic::to_text(&self.app.session).and_then(|text| {
            if text.len() > MAX_STORED_BYTES {
                return Err(format!("the workspace is over {} MB", MAX_STORED_BYTES / 1_000_000));
            }
            browser::store(STORAGE_KEY, &text)
        });
        match kept {
            Ok(()) => self.unsafe_to_leave.set(false),
            Err(e) => {
                // Asking before leaving protects changes that weren't downloaded either.
                self.unsafe_to_leave.set(self.app.session.dirty);
                log::warn!("trackcraft-web: workspace not kept in this browser: {e}");
            }
        }
    }
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        if let Some(req) = self.app.take_file_request() {
            self.file_request(req, ctx);
        }
        self.take_picked();
        let found = self.server.borrow().clone();
        if !self.server_noted
            && let Some(version) = found
        {
            self.server_noted = true;
            log::info!("trackcraft-web: served by TrackCraft server {version}");
            // A remembered token connects at once; otherwise ask.
            match browser::stored(TOKEN_KEY) {
                Some(token) if !token.is_empty() && self.conn.is_none() => {
                    self.dialog.target = ServerWorkspace { workspace: browser::stored(WORKSPACE_KEY).unwrap_or_default(), token };
                    self.dialog.remember = true;
                    self.connect(ctx);
                }
                _ if self.conn.is_none() => self.dialog.open = true,
                _ => {}
            }
        }
        self.take_connected(ctx);
        self.pump(ctx);
        self.keep(ctx, false);
        self.set_title();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        let ctx = ui.ctx().clone();
        self.server_ui(&ctx);
    }
}

/// Removes the remembered token (and workspace name) from this browser.
fn forget_token() {
    for key in [TOKEN_KEY, WORKSPACE_KEY] {
        if let Err(e) = browser::remove(key) {
            log::warn!("trackcraft-web: couldn't remove {key}: {e}");
        }
    }
}
