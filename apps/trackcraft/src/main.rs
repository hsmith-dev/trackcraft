//! TrackCraft desktop app.
//!
//! Usage: `trackcraft [--sample] [--control PORT] [--recent] [file.track]`. With no file and no
//! `--sample`, reopens the most recent workspace (if it still exists), else the sample workspace.
//! File › New starts an empty one.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod control;
mod paths;
mod recent;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use trackcraft_engine::Session;
use trackcraft_ui_egui::{FileRequest, TrackApp};

use control::{ControlRequest, KeyFile, reply_for};

const USAGE: &str = "\
usage: trackcraft [--sample] [--control PORT] [file.track]
       trackcraft --recent

  file.track        open this workspace (default: the most recent one, else the sample)
  --sample          open the sample workspace
  --control PORT    serve the control channel on 127.0.0.1:PORT (docs/control-protocol.md);
                    TRACKCRAFT_CONTROL_PORT does the same
  --recent          list recently opened workspaces and exit
";

/// How long to wait before retrying a failed autosave.
const AUTOSAVE_RETRY: Duration = Duration::from_secs(30);

/// At most this many control requests are handled per frame (the rest wait a frame).
const REQUESTS_PER_FRAME: usize = 64;

#[derive(Debug, Default, PartialEq)]
struct Args {
    help: bool,
    sample: bool,
    recent: bool,
    control: Option<u16>,
    file: Option<PathBuf>,
}

fn parse_port(s: &str) -> Result<u16, String> {
    match s.trim().parse::<u16>() {
        Ok(p) if p > 0 => Ok(p),
        _ => Err(format!("invalid control port '{}': expected 1-65535", s.chars().take(40).collect::<String>())),
    }
}

/// Parses the command line; `env_port` is `TRACKCRAFT_CONTROL_PORT` (used without `--control`).
fn parse_args(args: &[String], env_port: Option<&str>) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--help" | "-h" => a.help = true,
            "--sample" => a.sample = true,
            "--recent" => a.recent = true,
            "--control" => a.control = Some(parse_port(it.next().ok_or("--control needs a port")?)?),
            s if s.starts_with("--control=") => a.control = Some(parse_port(s.trim_start_matches("--control="))?),
            s if s.starts_with('-') => return Err(format!("unknown option '{}'", s.chars().take(40).collect::<String>())),
            s if a.file.is_none() => a.file = Some(PathBuf::from(s)),
            _ => return Err("more than one file given".into()),
        }
    }
    if a.control.is_none()
        && let Some(p) = env_port.filter(|p| !p.trim().is_empty())
    {
        a.control = Some(parse_port(p).map_err(|e| format!("TRACKCRAFT_CONTROL_PORT: {e}"))?);
    }
    Ok(a)
}

fn open_session(p: &Path) -> Result<Session, String> {
    let ws = trackcraft_store::load(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let user = ws.users.keys().next().cloned().unwrap_or_default();
    let mut s = Session::new(ws, &user);
    s.path = Some(p.to_path_buf());
    Ok(s)
}

/// The `path` of a `ui.screenshot` request: an absolute `.png` path.
fn screenshot_path(params: &Value) -> Result<PathBuf, String> {
    let p = params.get("path").and_then(Value::as_str).ok_or("ui.screenshot needs {\"path\": \"/abs/out.png\"}")?;
    if p.is_empty() || p.len() > 4096 {
        return Err("ui.screenshot: bad path".into());
    }
    let path = PathBuf::from(p);
    if !path.is_absolute() {
        return Err("ui.screenshot: the path must be absolute".into());
    }
    if !path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png")) {
        return Err("ui.screenshot: the path must end in .png".into());
    }
    Ok(path)
}

/// Encodes the image as PNG and writes it atomically (temp file + rename).
fn write_png(img: &egui::ColorImage, path: &Path) -> Result<Value, String> {
    let [w, h] = img.size;
    let (w32, h32) = (u32::try_from(w).map_err(|e| e.to_string())?, u32::try_from(h).map_err(|e| e.to_string())?);
    let mut data = Vec::with_capacity(img.pixels.len().saturating_mul(4));
    for px in &img.pixels {
        data.extend_from_slice(&px.to_srgba_unmultiplied());
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w32, h32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| format!("png: {e}"))?;
        wr.write_image_data(&data).map_err(|e| format!("png: {e}"))?;
        wr.finish().map_err(|e| format!("png: {e}"))?;
    }
    let tmp = path.with_extension("png.tmp");
    std::fs::write(&tmp, &out).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", path.display())
    })?;
    Ok(json!({"path": path.display().to_string(), "width": w, "height": h}))
}

struct App {
    app: TrackApp,
    title: String,
    control: Option<Receiver<ControlRequest>>,
    key_file: Option<KeyFile>,
    /// The autosave timer: the revision last seen while dirty, and when it was first seen.
    /// Autosave waits the Settings interval after the last change (UI-7).
    dirty_since: Option<(u64, Instant)>,
    /// The workspace path last recorded in the recent list.
    seen_path: Option<PathBuf>,
}

impl App {
    fn file_request(&mut self, req: FileRequest) {
        let filter = |d: rfd::FileDialog| d.add_filter("TrackCraft workspace", &["track"]);
        match req {
            FileRequest::New => {
                if let Err(e) = self.app.session.run("workspace.new", &json!({"name": "New workspace"})) {
                    self.app.notify(e);
                }
                self.app.navigate(trackcraft_ui_egui::View::Home);
            }
            FileRequest::Open => {
                if let Some(path) = filter(rfd::FileDialog::new()).pick_file() {
                    match self.app.session.run("file.open", &json!({"path": path.display().to_string()})) {
                        Ok(_) => {
                            let s = std::mem::replace(&mut self.app.session, Session::new(Default::default(), ""));
                            self.app = TrackApp::new(s);
                        }
                        Err(e) => self.app.notify(e),
                    }
                }
            }
            FileRequest::Save if self.app.session.path.is_some() => self.save(None),
            FileRequest::Save | FileRequest::SaveAs => {
                let name = format!("{}.track", self.app.session.ws.name);
                if let Some(path) = filter(rfd::FileDialog::new()).set_file_name(name).save_file() {
                    self.save(Some(path));
                }
            }
            // UI-fixes: File › Import… — the UI picked the kind; pick the export, run the import.
            FileRequest::Import(kind) => {
                use trackcraft_ui_egui::ImportKind;
                let d = rfd::FileDialog::new().set_title("Import");
                let picked = match kind {
                    ImportKind::JiraCsv => d.add_filter("CSV export", &["csv"]).pick_file(),
                    ImportKind::JiraJson => d.add_filter("JSON export", &["json"]).pick_file(),
                    ImportKind::Confluence { folder: true } => d.pick_folder(),
                    ImportKind::Confluence { folder: false } => d.add_filter("Space export", &["zip", "html", "htm", "xml"]).pick_file(),
                };
                if let Some(path) = picked {
                    self.app.import_file(kind, &path);
                }
            }
        }
    }

    fn save(&mut self, path: Option<PathBuf>) {
        let p = match path {
            Some(p) => json!({"path": p.display().to_string()}),
            None => json!({}),
        };
        match self.app.session.run("file.save", &p) {
            Ok(v) => {
                let saved = v["path"].as_str().unwrap_or_default().to_string();
                remember(Path::new(&saved));
                self.app.notify(format!("Saved {saved}"));
            }
            Err(e) => self.app.notify(e),
        }
    }

    /// Answers queued control requests on the UI thread.
    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.control else { return };
        let reqs: Vec<ControlRequest> = std::iter::from_fn(|| rx.try_recv().ok()).take(REQUESTS_PER_FRAME).collect();
        if reqs.len() == REQUESTS_PER_FRAME {
            ctx.request_repaint();
        }
        for r in reqs {
            if r.method == "ui.screenshot" {
                match screenshot_path(&r.params) {
                    Ok(path) => {
                        let reply = r.reply;
                        let cb = egui::ScreenshotCallback::new(move |img: Arc<egui::ColorImage>| {
                            let _ = reply.send(reply_for(write_png(&img, &path)));
                        });
                        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(cb));
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        let _ = r.reply.send(reply_for(Err(e)));
                    }
                }
                continue;
            }
            if r.method == "app.quit" {
                // Closing runs `on_exit`: a dirty workspace with a path is saved, the key removed.
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                let _ = r.reply.send(reply_for(Ok(json!({"ok": true}))));
                continue;
            }
            let res = self.app.control(&json!({"method": r.method, "params": r.params}));
            let _ = r.reply.send(reply_for(res));
        }
    }

    /// Records a newly opened or saved-as path in the recent list.
    fn track_path(&mut self) {
        if self.app.session.path != self.seen_path {
            self.seen_path = self.app.session.path.clone();
            if let Some(p) = &self.seen_path {
                remember(p);
            }
        }
    }

    /// Saves a dirty workspace that has a path once no change has happened for the autosave
    /// interval set in Settings (`TrackApp::autosave_secs`; `None` turns autosave off). A
    /// workspace without a path is never autosaved.
    fn autosave(&mut self, ctx: &egui::Context) {
        let Some(secs) = self.app.autosave_secs() else {
            self.dirty_since = None;
            return;
        };
        if !self.app.session.dirty || self.app.session.path.is_none() {
            self.dirty_since = None;
            return;
        }
        let wait = Duration::from_secs(secs);
        let rev = self.app.session.revision();
        let since = match self.dirty_since {
            Some((seen, at)) if seen == rev => at,
            // A new change restarts the wait.
            _ => {
                let now = Instant::now();
                self.dirty_since = Some((rev, now));
                now
            }
        };
        let elapsed = since.elapsed();
        if elapsed < wait {
            ctx.request_repaint_after(wait.saturating_sub(elapsed));
            return;
        }
        match self.app.session.run("file.save", &json!({})) {
            Ok(_) => self.dirty_since = None,
            Err(e) => {
                self.app.notify(format!("Autosave failed: {e}"));
                // Try again after a while (the wait restarts from now).
                let retry = AUTOSAVE_RETRY.max(wait);
                let now = Instant::now();
                self.dirty_since = Some((rev, now.checked_add(retry.saturating_sub(wait)).unwrap_or(now)));
                ctx.request_repaint_after(retry);
            }
        }
    }
}

/// Adds `p` to the recent list; failures are reported, never fatal.
fn remember(p: &Path) {
    if p.as_os_str().is_empty() {
        return;
    }
    if let Some(f) = paths::recent_path()
        && let Err(e) = recent::remember(&f, p)
    {
        eprintln!("trackcraft: recent files: {e}");
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        if let Some(req) = self.app.take_file_request() {
            self.file_request(req);
        }
        self.drain_control(ctx);
        self.track_path();
        self.autosave(ctx);
        let title = self.app.title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }

    fn on_exit(&mut self) {
        if self.app.session.dirty
            && self.app.session.path.is_some()
            && let Err(e) = self.app.session.run("file.save", &json!({}))
        {
            eprintln!("trackcraft: save on exit failed: {e}");
        }
        if let Some(k) = &self.key_file {
            k.remove();
        }
    }
}

fn main() -> std::process::ExitCode {
    use std::process::ExitCode;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let env_port = std::env::var("TRACKCRAFT_CONTROL_PORT").ok();
    let args = match parse_args(&argv, env_port.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("trackcraft: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let recent_list = paths::recent_path().map(|f| recent::load(&f)).unwrap_or_default();
    if args.recent {
        for p in &recent_list {
            println!("{}", p.display());
        }
        return ExitCode::SUCCESS;
    }
    let session = match &args.file {
        Some(p) => match open_session(p) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("trackcraft: {e}");
                return ExitCode::FAILURE;
            }
        },
        None if args.sample => Session::sample(),
        None => match recent_list.iter().find(|p| p.is_file()).map(|p| open_session(p)) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                eprintln!("trackcraft: could not reopen the last workspace ({e}); opening the sample");
                Session::sample()
            }
            None => Session::sample(),
        },
    };
    let server = match args.control {
        Some(port) => {
            let bound = paths::key_path(port)
                .ok_or_else(|| "no config folder for the control key file (set HOME or TRACKCRAFT_CONFIG_DIR)".to_string())
                .and_then(|kp| control::bind(port, &kp));
            match bound {
                Ok(s) => {
                    eprintln!("trackcraft: control channel on 127.0.0.1:{port}, key in {}", s.key_file.path().display());
                    Some(s)
                }
                Err(e) => {
                    eprintln!("trackcraft: control channel: {e}");
                    return ExitCode::from(2);
                }
            }
        }
        None => None,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("TrackCraft").with_inner_size([1360.0, 860.0]).with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    let mut app = App { app: TrackApp::new(session), title: String::new(), control: None, key_file: None, dirty_since: None, seen_path: None };
    let result = eframe::run_native(
        "TrackCraft",
        options,
        Box::new(move |cc| {
            if let Some(server) = server {
                let ctx = cc.egui_ctx.clone();
                let (rx, key_file) = server.serve(move || ctx.request_repaint());
                app.control = Some(rx);
                app.key_file = Some(key_file);
            }
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("trackcraft: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Result<Args, String> {
        parse_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), None)
    }

    #[test]
    fn parses_arguments() {
        assert_eq!(args(&[]).unwrap(), Args::default());
        let a = args(&["--control", "7990", "w.track"]).unwrap();
        assert_eq!((a.control, a.file), (Some(7990), Some(PathBuf::from("w.track"))));
        assert_eq!(args(&["--control=12"]).unwrap().control, Some(12));
        assert!(args(&["--recent"]).unwrap().recent);
        assert!(args(&["--sample"]).unwrap().sample);
        for bad in [&["--control"][..], &["--control", "0"], &["--control", "70000"], &["--control", "x"], &["--nope"], &["a", "b"]] {
            assert!(args(bad).is_err(), "{bad:?}");
        }
        let env = |p: &str| parse_args(&[], Some(p));
        assert_eq!(env("7981").unwrap().control, Some(7981));
        assert_eq!(env("").unwrap().control, None);
        assert!(env("junk").is_err());
        // The flag wins over the environment.
        assert_eq!(parse_args(&["--control".into(), "5".into()], Some("6")).unwrap().control, Some(5));
    }

    #[test]
    fn screenshot_paths_are_checked() {
        let ok = if cfg!(windows) { "C:\\shots\\a.png" } else { "/tmp/a.PNG" };
        assert!(screenshot_path(&json!({"path": ok})).is_ok());
        for bad in [json!({}), json!({"path": 3}), json!({"path": ""}), json!({"path": "rel.png"}), json!({"path": "/tmp/a.jpg"}), json!(null)] {
            assert!(screenshot_path(&bad).is_err(), "{bad}");
        }
        assert!(screenshot_path(&json!({"path": format!("/{}.png", "a".repeat(5000))})).is_err());
    }

    #[test]
    fn writes_png() {
        let d = std::env::temp_dir().join(format!("trackcraft-png-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("shot.png");
        let img = egui::ColorImage::new([3, 2], vec![egui::Color32::RED; 6]);
        let v = write_png(&img, &p).unwrap();
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(3), Some(2)));
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        let _ = std::fs::remove_dir_all(&d);
    }
}
