//! The browser shell: the eframe web runner, File requests through the browser, the workspace kept
//! in `localStorage`, and the served-by-a-server note.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::json;
use trackcraft_engine::Session;
use trackcraft_ui_egui::{FileRequest, TrackApp, View};
use wasm_bindgen::JsCast as _;

use crate::browser::{self, Picked};
use crate::logic::{self, MAX_OPEN_BYTES, MAX_STORED_BYTES, STORAGE_KEY};

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

impl WebShell {
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
        }
    }

    /// Takes a file the picker delivered and opens it in place of the current workspace.
    fn take_picked(&mut self) {
        let Some(picked) = self.picked.borrow_mut().take() else { return };
        match picked.and_then(|(name, text)| logic::session_from_text(&text).map(|s| (name, s)).map_err(|e| format!("Couldn't open the file: {e}"))) {
            Ok((name, session)) => {
                self.app = TrackApp::new(session);
                self.generation = self.generation.wrapping_add(1);
                self.app.notify(format!("Opened {name}"));
            }
            Err(e) => self.app.notify(e),
        }
    }

    /// Keeps the workspace in `localStorage` (at most every [`STORE_EVERY_SECS`]), so a reload or
    /// a closed tab doesn't lose it. A workspace too large for storage stays in memory only.
    fn keep(&mut self, ctx: &egui::Context) {
        let key = (self.generation, self.app.session.revision());
        if self.stored == Some(key) {
            self.unsafe_to_leave.set(false);
            return;
        }
        let now = ctx.input(|i| i.time);
        let wait = STORE_EVERY_SECS - (now - self.last_store);
        if self.stored.is_some() && wait > 0.0 {
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
        if !self.server_noted
            && let Some(version) = self.server.borrow().clone()
        {
            self.server_noted = true;
            self.app.notify(logic::server_note(&version));
        }
        self.keep(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}
