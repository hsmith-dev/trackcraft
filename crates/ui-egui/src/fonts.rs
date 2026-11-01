//! The UI font stack: egui's bundled faces first, then non-Latin scripts and emoji, so issue
//! titles and pages in any script draw real glyphs instead of tofu.
//!
//! Built with the suite's [`craft_ui::fonts::UiFonts`], which every Craft app shares:
//!
//! 1. egui's bundled faces (Ubuntu Light, Hack): Latin, Greek, Cyrillic — the look is unchanged;
//! 2. the craft-fonts faces this build embedded ([`CRAFT_FONTS`], only when `CRAFT_FONTS_DIR` was
//!    set at build time; see `build.rs`): Arabic, Devanagari, Hebrew, Thai, Georgian, symbols;
//! 3. installed system fonts (desktop): CJK, colour emoji and any other script. They are found
//!    per character on demand: an index of the installed fonts is built once on a background
//!    thread and a face is loaded only when text first needs it, so startup reads no CJK file;
//! 4. on the web (no system fonts): egui's monochrome Noto Emoji for emoji. CJK there needs the
//!    web client to embed or serve a craft-fonts CJK face; until then it draws tofu.
//!
//! Shaping: the suite's egui fork shapes runs with HarfRust and orders them with unicode-bidi, so
//! Arabic letters join and read right to left, and Devanagari conjuncts and reordered vowel signs
//! (त्रुटि) draw correctly (checked in `shots/intl.jsonl` on macOS). Limits: a right-to-left
//! paragraph is still laid out left-aligned in a left-to-right UI (no paragraph direction), an
//! elided mixed-direction label puts its `…` at the logical end, and CJK from a system face
//! (PingFang, Noto Sans CJK) looks heavier than egui's Ubuntu Light.
//!
//! No font file lives in this repository (fonts go to storytold/craft-fonts).

use craft_fonts_build::EmbeddedFace;
use craft_ui::fonts::{CraftFace, I18nScripts, Sample, Stack, UiFonts};

/// The craft-fonts faces this build embedded; empty without `CRAFT_FONTS_DIR`.
pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

fn ui_fonts() -> UiFonts<'static> {
    let faces: Vec<CraftFace> = CRAFT_FONTS.iter().map(|f| CraftFace::from(*f)).collect();
    UiFonts::new(&I18nScripts).ui_language("en").craft_faces(&faces)
}

/// Install the UI fonts in `ctx`: call once at startup, before the first frame (desktop and web).
/// Never blocks: the system font index builds in the background, and text that needs it redraws
/// when it is ready (a frame or two of tofu at most, only for scripts egui and craft-fonts lack).
pub fn install(ctx: &egui::Context) -> Stack {
    ui_fonts().install(ctx)
}

/// [`install`] with a system font lookup that waits for the index, so the first frame already
/// draws every script the machine has: for tests and headless screenshots.
pub fn install_waiting(ctx: &egui::Context) -> Stack {
    let mut fonts = ui_fonts();
    if let Some(system) = craft_ui::fonts::system_font_provider("en") {
        fonts = fonts.system_provider(system);
    }
    fonts.install(ctx)
}

/// The characters of `text` that `stack` (with this machine's system fonts) can't draw, e.g. for a
/// test to skip on a machine without CJK or emoji fonts.
pub fn missing(stack: &Stack, text: &str) -> Vec<char> {
    let sample = Sample { language: "en", key: "text", text };
    craft_ui::fonts::missing_glyphs(stack, &egui::FontFamily::Proportional, [sample], true).into_iter().map(|m| m.character).collect()
}
