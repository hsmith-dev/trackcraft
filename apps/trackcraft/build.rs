//! Windows only: embed the app icon and version info (VERSIONINFO) into `trackcraft.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab (as Wordcraft's build.rs does).
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `TRACKCRAFT_REQUIRE_WINRES=1` turns it
//! into an error (packaging/windows/package.ps1 sets it for release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/trackcraft.ico");
    println!("cargo:rerun-if-env-changed=TRACKCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/trackcraft.ico")
        .set("ProductName", "TrackCraft")
        .set("FileDescription", "TrackCraft issue tracker and wiki")
        .set("CompanyName", "TrackCraft contributors")
        .set("LegalCopyright", "Copyright (c) the TrackCraft contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "trackcraft.exe")
        .set("InternalName", "trackcraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("TRACKCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=trackcraft.exe built without icon/version resources: {e}");
    }
}
