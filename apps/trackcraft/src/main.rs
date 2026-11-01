//! TrackCraft desktop app.
//!
//! Usage: `trackcraft [--sample] [file.track]`. With no file, opens the sample workspace on first
//! run so there is something to look at; File › New starts an empty one.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;

use serde_json::json;
use trackcraft_engine::Session;
use trackcraft_ui_egui::{FileRequest, TrackApp};

const USAGE: &str = "usage: trackcraft [--sample] [file.track]\n";

struct App {
    app: TrackApp,
    title: String,
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
        }
    }

    fn save(&mut self, path: Option<PathBuf>) {
        let p = match path {
            Some(p) => json!({"path": p.display().to_string()}),
            None => json!({}),
        };
        match self.app.session.run("file.save", &p) {
            Ok(v) => self.app.notify(format!("Saved {}", v["path"].as_str().unwrap_or_default())),
            Err(e) => self.app.notify(e),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        if let Some(req) = self.app.take_file_request() {
            self.file_request(req);
        }
        let title = self.app.title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return std::process::ExitCode::SUCCESS;
    }
    let file = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from);
    let session = match &file {
        Some(p) => match trackcraft_store::load(p) {
            Ok(ws) => {
                let user = ws.users.keys().next().cloned().unwrap_or_default();
                let mut s = Session::new(ws, &user);
                s.path = Some(p.clone());
                s
            }
            Err(e) => {
                eprintln!("trackcraft: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        None => Session::sample(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("TrackCraft").with_inner_size([1360.0, 860.0]).with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    let app = App { app: TrackApp::new(session), title: String::new() };
    match eframe::run_native("TrackCraft", options, Box::new(|_cc| Ok(Box::new(app)))) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("trackcraft: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
