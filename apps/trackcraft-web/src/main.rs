//! TrackCraft in the browser.
//!
//! Runs the same [`trackcraft_ui_egui::TrackApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory, or `cargo xtask web` from the repository root (output in `dist/web`). See
//! `docs/web.md`.
//!
//! Differences from the desktop app: no control channel and no file system. File › Open uses the
//! browser's file picker, Save and Save As download the `.track` file, New and the sample work in
//! memory, and the open workspace is kept in the browser's `localStorage` so a reload keeps it.
//! URL flags: `?sample` opens the sample workspace (instead of the stored one), `?webgl` forces
//! WebGL2. Served by `trackcraft-server serve --web DIR`, the page notices the server (same
//! origin, `api/health`) and offers to connect to a server workspace with a token; it is then
//! edited live (M4.6: [`sync`] is the state machine, `sync_web` the fetch/EventSource transport).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(any(target_arch = "wasm32", test))]
mod logic;
#[cfg(any(target_arch = "wasm32", test))]
mod remote;
#[cfg(any(target_arch = "wasm32", test))]
mod sync;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod sync_web;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("trackcraft-web only runs in the browser: build it with `cargo xtask web` (or `trunk build --release` in apps/trackcraft-web)");
}
