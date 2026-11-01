//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `git`) run through
//! `std::process::Command`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

mod icons;
mod version;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        fmt --check, clippy -D warnings, test, assets, layers, wasm, parity --check
  test      workspace tests
  assets    check that every icon/image/font/asset is attributed in ATTRIBUTION.md
  layers    enforce the crate dependency layering (docs/architecture.md)
  wasm      cargo check --target wasm32-unknown-unknown for the wasm-safe crates
  web       build the web client into dist/web with trunk (skipped when trunk isn't installed)
  parity    regenerate docs/parity-checklist.md from crates/engine/src/catalog.rs
            (--check: fail if the committed file is stale)
  version   print the workspace version (`version set X.Y.Z[-pre]` changes it; packaging reads it)
  icons     render assets/app-icon/trackcraft.svg into the PNG/ICO icons (packaging/icons.sh)
";

/// Layer of each internal crate. A crate may depend only on strictly lower layers.
const LAYERS: &[(&str, u8)] = &[
    ("trackcraft-core", 0),
    ("trackcraft-markup", 1),
    ("trackcraft-jql", 1),
    // Live-sync wire types: serde only, app-agnostic (extractable to craft-collab, docs/sync.md).
    ("trackcraft-sync-proto", 1),
    ("trackcraft-model", 2),
    ("trackcraft-store", 3),
    ("trackcraft-import", 3),
    ("trackcraft-engine", 4),
    ("trackcraft-mcp", 5),
    ("trackcraft-api", 5),
    // The blocking sync client (std TcpStream; no async runtime): desktop remote mode.
    ("trackcraft-sync-client", 5),
    // The mountable server library sits above the API routes it serves; no UI crates (checked
    // below by name, since L6 alone would allow them).
    ("trackcraft-server-lib", 6),
    ("trackcraft-ui-egui", 6),
    ("trackcraft", 7),
    ("trackcraft-cli", 7),
    ("trackcraft-server", 7),
    ("trackcraft-web", 7),
    ("xtask", 7),
];

/// UI and server-runtime crates that nothing below L6 (UI) / L5 (HTTP) may depend on.
const UI_ONLY: &[&str] = &["egui", "eframe", "winit", "rfd", "wgpu"];
const SERVER_ONLY: &[&str] = &["tokio", "axum", "hyper", "tower"];

/// Crates checked for wasm32 (everything below the native apps).
const WASM_CRATES: &[&str] = &[
    "trackcraft-core",
    "trackcraft-markup",
    "trackcraft-jql",
    "trackcraft-model",
    "trackcraft-store",
    "trackcraft-import",
    "trackcraft-engine",
    "trackcraft-sync-proto",
    "trackcraft-ui-egui",
    "trackcraft-web",
];

const ASSET_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "icns", "svg", "pdf", "ttf", "otf", "woff", "woff2", "mp4", "wav", "mp3",
    "docx", "xlsx",
];
const ASSET_DIRS: &[&str] = &["assets/", "docs/images/", "examples/"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("test") => cargo(&["test", "--workspace", "--quiet"]),
        Some("assets") => assets(),
        Some("layers") => layers(),
        Some("wasm") => wasm(),
        Some("web") => web(),
        Some("parity") => parity(rest.contains(&"--check")),
        Some("version") => version::run(&root(), &rest),
        Some("icons") => icons::run(&root(), &rest),
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root())
        .args(args)
        .status()
        .map_err(|e| format!("cargo {}: {e}", args.join(" ")))?;
    if status.success() { Ok(()) } else { Err(format!("cargo {} failed", args.join(" "))) }
}

fn ci() -> Result<(), String> {
    let steps: [(&str, fn() -> Result<(), String>); 7] = [
        ("fmt", || cargo(&["fmt", "--all", "--check"])),
        ("clippy", || cargo(&["clippy", "--workspace", "--all-targets", "--quiet", "--", "-D", "warnings"])),
        ("test", || cargo(&["test", "--workspace", "--quiet"])),
        ("assets", assets),
        ("layers", layers),
        ("wasm", wasm),
        ("parity", || parity(true)),
    ];
    for (name, step) in steps {
        println!("── xtask ci: {name}");
        step().map_err(|e| format!("{name}: {e}"))?;
    }
    println!("── xtask ci: all gates passed");
    Ok(())
}

fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

fn layers() -> Result<(), String> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let mut errors = Vec::new();
    for pkg in meta["packages"].as_array().into_iter().flatten() {
        let name = pkg["name"].as_str().unwrap_or_default();
        let Some(l) = layer(name) else {
            errors.push(format!("{name} has no layer: add it to LAYERS in xtask/src/main.rs and docs/architecture.md"));
            continue;
        };
        for dep in pkg["dependencies"].as_array().into_iter().flatten() {
            if dep["kind"].as_str() == Some("dev") {
                continue;
            }
            let d = dep["name"].as_str().unwrap_or_default();
            if let Some(dl) = layer(d)
                && dl >= l
            {
                errors.push(format!("{name} (L{l}) depends on {d} (L{dl}): dependencies must point to lower layers"));
            }
            if (l < 6 || name == "trackcraft-server-lib") && UI_ONLY.contains(&d) {
                errors.push(format!("{name} (L{l}) depends on UI crate {d}: only L6+ may"));
            }
            if l < 5 && SERVER_ONLY.contains(&d) {
                errors.push(format!("{name} (L{l}) depends on server runtime {d}: only L5+ may"));
            }
        }
    }
    if errors.is_empty() {
        println!("layers: ok");
        Ok(())
    } else {
        Err(errors.join("\n  "))
    }
}

fn wasm() -> Result<(), String> {
    let mut args = vec!["check", "--quiet", "--target", "wasm32-unknown-unknown"];
    for c in WASM_CRATES {
        args.push("-p");
        args.push(c);
    }
    cargo(&args)
}

/// Builds the web client (`apps/trackcraft-web`) into `dist/web` with trunk, as Wordcraft does,
/// and copies the sample host configs from `packaging/web`. Without trunk it says how to get it
/// and succeeds, so machines without the web toolchain aren't blocked.
fn web() -> Result<(), String> {
    let has_trunk = Command::new("trunk").arg("--version").output().is_ok_and(|o| o.status.success());
    if !has_trunk {
        println!("web: skipped: trunk is not installed.");
        println!("  install: cargo install trunk --locked   (or: brew install trunk)");
        println!("  and:     rustup target add wasm32-unknown-unknown");
        println!("  then:    cargo xtask web   (output in dist/web; see docs/web.md)");
        return Ok(());
    }
    let app = root().join("apps/trackcraft-web");
    let status = Command::new("trunk").current_dir(&app).args(["build", "--release"]).status().map_err(|e| format!("trunk: {e}"))?;
    if !status.success() {
        return Err("trunk build --release failed".into());
    }
    let dist = root().join("dist/web");
    let index = std::fs::read_to_string(dist.join("index.html")).map_err(|e| format!("dist/web/index.html: {e}"))?;
    // Relative URLs only (`public_url = "./"`), so the site works under any prefix.
    if index.contains("src=\"/") || index.contains("href=\"/") {
        return Err("dist/web/index.html has root-absolute URLs; it would break when served from a sub-path".into());
    }
    for f in ["_headers", ".htaccess"] {
        std::fs::copy(root().join("packaging/web").join(f), dist.join(f)).map_err(|e| format!("packaging/web/{f}: {e}"))?;
    }
    println!("web: built dist/web (serve it: trackcraft-server serve --web dist/web)");
    Ok(())
}

fn is_asset(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase).unwrap_or_default();
    ASSET_DIRS.iter().any(|d| path.starts_with(d)) || ASSET_EXT.contains(&ext.as_str())
}

fn assets() -> Result<(), String> {
    let out = Command::new("git")
        .current_dir(root())
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    // A failed listing is no listing: read as empty, it would pass with nothing checked.
    if !out.status.success() {
        return Err(format!("git ls-files failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let md = std::fs::read_to_string(root().join("ATTRIBUTION.md")).map_err(|e| format!("ATTRIBUTION.md: {e}"))?;
    let files: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .filter(|f| root().join(f).exists())
        .collect();
    let assets: Vec<&String> = files.iter().filter(|f| is_asset(f)).collect();
    let missing: Vec<&&String> = assets.iter().filter(|f| !md.contains(&format!("`{f}`"))).collect();
    if missing.is_empty() {
        println!("assets: all {} asset files attributed in ATTRIBUTION.md", assets.len());
        Ok(())
    } else {
        Err(format!(
            "{} asset file(s) lack an ATTRIBUTION.md row (author, source, licence):\n  {}",
            missing.len(),
            missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  ")
        ))
    }
}

fn parity_markdown() -> String {
    use trackcraft_engine::catalog::{FEATURES, coverage};
    let (done, total) = coverage();
    let pct = if total == 0 { 0.0 } else { done as f64 * 100.0 / total as f64 };
    let mut s = String::from("# Parity checklist\n\n");
    s.push_str("Generated by `cargo xtask parity` from `crates/engine/src/catalog.rs` — do not edit by hand.\n\n");
    s.push_str(&format!("**Catalog coverage: {done} / {total} ({pct:.1}%)**\n\n"));
    let mut product = "";
    let mut area = "";
    for f in FEATURES {
        if f.product != product {
            product = f.product;
            area = "";
            s.push_str(&format!("## {product}\n\n"));
        }
        if f.area != area {
            area = f.area;
            s.push_str(&format!("### {area}\n\n"));
        }
        if f.command.is_empty() {
            s.push_str(&format!("- [ ] {}\n", f.name));
        } else {
            s.push_str(&format!("- [x] {} — `{}`\n", f.name, f.command));
        }
    }
    let mut s = s.replace("\n### ", "\n\n### ").replace("\n## ", "\n\n## ");
    while s.contains("\n\n\n") {
        s = s.replace("\n\n\n", "\n\n");
    }
    format!("{}\n", s.trim_end())
}

fn parity(check: bool) -> Result<(), String> {
    let path = root().join("docs/parity-checklist.md");
    let want = parity_markdown();
    if check {
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if have != want {
            return Err("docs/parity-checklist.md is stale: run `cargo xtask parity`".into());
        }
        println!("parity: docs/parity-checklist.md is current");
        return Ok(());
    }
    std::fs::write(&path, want).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("parity: wrote {}", path.display());
    Ok(())
}
