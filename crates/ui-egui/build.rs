//! fonts: the optional craft-fonts build input (<https://github.com/storytold/craft-fonts>).
//!
//! With `CRAFT_FONTS_DIR=<checkout>` the faces of its `fonts/manifest.txt` that TrackCraft uses
//! are embedded as `crate::fonts::CRAFT_FONTS`; unset, the table is empty and the UI uses egui's
//! faces and the installed system fonts. Font files never live in this repository.
//!
//! Selected: every non-CJK face (Arabic, Devanagari, Hebrew, Thai, Georgian, symbols; about
//! 3 MB at the suite's pin). CJK faces are 2–16 MB each and every desktop OS ships one, so CJK
//! comes from the system fonts. Web (wasm32) builds have no system fonts and a size budget: they
//! embed Arabic, Devanagari and Hebrew only (about 1.2 MB). With `CRAFT_FONTS_REQUIRED=1` (CI
//! with a craft-fonts ref) an unreadable checkout fails the build. Reads the local checkout only.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let wasm = craft_fonts_build::target_is_wasm();
    let written = craft_fonts_build::Embed::new()
        .filter(move |face| !face.is_cjk() && (!wasm || ["Arab", "Deva", "Hebr"].iter().any(|s| face.has_script(s))))
        .write("craft_fonts.rs");
    if let Err(e) = written {
        println!("cargo::error={e}");
    }
}
