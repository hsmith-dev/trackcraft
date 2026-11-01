//! Headless UI screenshots: renders the whole TrackCraft window offscreen (wgpu), so it works
//! with a locked screen and never steals focus.
//!
//! `cargo run -p trackcraft-ui-egui --example ui_shot -- script.jsonl`
//!
//! The script is JSON lines: control requests (`{"method": …, "params": …}`, see
//! `TrackApp::control`), `{"shot": "/abs/out.png"}` to save the window as PNG, or
//! `{"steps": n}` to run extra frames. The window is 1440×900 pt at 2× and opens the sample
//! workspace (fixed clock, so shots are reproducible).
//!
//! A first line `{"fixture": "stress" | "empty" | "stress-intl"}` opens a hostile-data fixture
//! instead (see `trackcraft_ui_egui::fixtures`). `stress-intl` adds emoji, CJK, Arabic and
//! Devanagari text, which the bundled font lacks: it draws tofu, as the app does.

use trackcraft_engine::Session;
use trackcraft_ui_egui::TrackApp;

fn fixed_now() -> trackcraft_engine::model::Timestamp {
    trackcraft_engine::model::Timestamp::from_timestamp(1_793_448_000, 0).unwrap_or_default()
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: ui_shot script.jsonl");
    let script = std::fs::read_to_string(&path).expect("read script");
    let lines: Vec<serde_json::Value> =
        script.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).expect("json line")).collect();
    // UI-6
    let fixture = lines.first().and_then(|l| l.get("fixture")).and_then(|v| v.as_str()).map(str::to_string);
    let ws = match fixture.as_deref() {
        Some("stress") => trackcraft_ui_egui::fixtures::stress(fixed_now),
        Some("stress-intl") => trackcraft_ui_egui::fixtures::stress_with(
            fixed_now,
            trackcraft_ui_egui::fixtures::StressSpec { uncovered_scripts: true, ..Default::default() },
        ),
        Some("empty") => trackcraft_ui_egui::fixtures::empty(),
        Some(other) => panic!("unknown fixture {other}"),
        None => trackcraft_engine::sample::workspace(fixed_now()),
    };
    let app = TrackApp::new(Session::new(ws, "riley").with_clock(fixed_now));
    let mut builder = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(2.0);
    if fixture.as_deref() == Some("stress-intl") {
        builder = builder.allow_missing_glyphs();
    }
    let mut harness = builder.wgpu().build_ui_state(
        |ui, app: &mut TrackApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);
    for l in lines {
        if l.get("fixture").is_some() {
            continue;
        } else if let Some(p) = l.get("shot").and_then(|v| v.as_str()) {
            harness.run_steps(4);
            match harness.render() {
                Ok(img) => {
                    img.save(p).expect("save png");
                    println!("{p}");
                }
                Err(e) => eprintln!("render failed: {e}"),
            }
        } else if let Some(n) = l.get("steps").and_then(|v| v.as_u64()) {
            harness.run_steps(n as usize);
        } else {
            match harness.state_mut().control(&l) {
                Ok(_) => {}
                Err(e) => eprintln!("{l}: {e}"),
            }
            harness.run_steps(2);
        }
    }
}
