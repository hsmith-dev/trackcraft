# Asset attribution

Every non-code asset in this repository (fonts, icons, images, example files) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset file is missing from this table.

**Policy (mandatory):** TrackCraft contains **no Atlassian iconography, logos, images, templates, themes, illustrations, fonts or other assets** (nor any other vendor's). Every asset is original work by TrackCraft contributors or third-party material under an open licence (OSI open source, public domain / CC0, or a Creative Commons licence that allows redistribution). Screenshots of other vendors' software are never committed. Font files go to [storytold/craft-fonts](https://github.com/storytold/craft-fonts), not this repo.

Generated-in-code content is original and has no file to list: the sample workspace (`crates/engine/src/sample.rs`) and, from M1, the UI icon set and theme colours.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/app-icon/trackcraft.svg` | TrackCraft contributors | Original work (drawn for this repository) | MIT OR Apache-2.0 | The app icon: the in-app mark (`app_mark` in `crates/ui-egui/src/icons.rs`) as artwork. Source of every file below |
| `assets/app-icon/hicolor/scalable/apps/io.github.hsmith_dev.trackcraft.svg` | TrackCraft contributors | Copy of `trackcraft.svg` (`packaging/icons.sh`) | MIT OR Apache-2.0 | Linux hicolor scalable icon |
| `assets/app-icon/trackcraft-1024.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` (`packaging/icons.sh`) | MIT OR Apache-2.0 | 1024 px master |
| `assets/app-icon/trackcraft-macos-512.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | macOS window/Dock icon (Apple icon grid), embedded by `apps/trackcraft` |
| `assets/app-icon/trackcraft.icns` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` + `iconutil` | MIT OR Apache-2.0 | macOS app bundle icon |
| `assets/app-icon/trackcraft.ico` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Windows .exe and installer icon (16–256 px) |
| `assets/app-icon/hicolor/16x16/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon |
| `assets/app-icon/hicolor/24x24/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon |
| `assets/app-icon/hicolor/32x32/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon |
| `assets/app-icon/hicolor/48x48/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon |
| `assets/app-icon/hicolor/64x64/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon |
| `assets/app-icon/hicolor/128x128/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon |
| `assets/app-icon/hicolor/256x256/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon; window icon on Windows/Linux |
| `assets/app-icon/hicolor/512x512/apps/io.github.hsmith_dev.trackcraft.png` | TrackCraft contributors | Rendered from `trackcraft.svg` by `cargo xtask icons` | MIT OR Apache-2.0 | Linux hicolor icon |

## Fonts embedded at build time (not in this repository)

Builds made with `CRAFT_FONTS_DIR` set (CI pins storytold/craft-fonts `b076417b5880bd4154bd22f00695b1d28c6bea83`) embed these faces from that checkout into the UI (`crates/ui-egui/build.rs`, `crates/ui-egui/src/fonts.rs`); web builds embed only Arabic, Devanagari and Hebrew. The files and their licence texts live in craft-fonts. Builds without it embed no font beyond egui's own (egui's bundled faces come with the `egui` crate).

| Face | Author | Source | Licence | Notes |
|---|---|---|---|---|
| Noto Sans Arabic Regular | Google / Noto project authors | https://github.com/google/fonts/tree/7085eb89a950e85db5b166b7a58d414544b4140c/ofl/notosansarabic | OFL-1.1 | Arabic script fallback |
| Noto Sans Devanagari Regular | Google / Noto project authors | https://github.com/notofonts/devanagari/releases/tag/NotoSansDevanagari-v2.007 | OFL-1.1 | Devanagari script fallback |
| Noto Sans Hebrew Regular | Google / Noto project authors | https://github.com/notofonts/hebrew/releases/tag/NotoSansHebrew-v3.001 | OFL-1.1 | Hebrew script fallback |
| Noto Sans Thai Regular | Google / Noto project authors | https://github.com/google/fonts/tree/8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5/ofl/notosansthai | OFL-1.1 | Thai script fallback (desktop) |
| Noto Sans Georgian Regular | Google / Noto project authors | https://github.com/google/fonts/tree/8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5/ofl/notosansgeorgian | OFL-1.1 | Georgian script fallback (desktop) |
| Noto Sans Symbols Regular | Google / Noto project authors | https://github.com/google/fonts/tree/47d54ca1ebb14fd864695f1d39dc734f39017c33/ofl/notosanssymbols | OFL-1.1 | Symbol fallback (desktop) |
| Noto Sans Symbols 2 Regular | Google / Noto project authors | https://github.com/google/fonts/tree/47d54ca1ebb14fd864695f1d39dc734f39017c33/ofl/notosanssymbols2 | OFL-1.1 | Symbol fallback (desktop) |
