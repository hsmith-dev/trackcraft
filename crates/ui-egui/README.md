# trackcraft-ui-egui

TrackCraft's egui front end (layer L6). Views read `Session` state and change it only through
`Session::run`; each frame they push `Action`s that `TrackApp::ui` applies after drawing.

## Module map

| Path | What |
|---|---|
| `src/lib.rs` | `TrackApp`: state, `logic` (global shortcuts), `ui` (frame), `Action` / `Then`, `apply`, `run`, `control` (the `ui.*` / `engine.execute` channel) |
| `src/route.rs` | `View` (re-exported from the crate root) and `parse`, the `ui.navigate` request → `View` |
| `src/guard.rs` | Unsaved-edit guard: dirty checks, held actions, the "Discard your changes?" dialog |
| `src/shell.rs` | Top bar, sidebar (filters, projects, spaces + page tree), home |
| `src/issues/` | `list.rs` (JQL list), `board.rs` (board + `board_columns`), `detail.rs` (issue view), `create.rs` (create dialog) |
| `src/wiki/` | `mod.rs` (page view, editor, history, new-page dialog), `comments.rs` (footer + block comments, UI-27), `diff.rs` (version diff, UI-28), `templates.rs` (UI-29), `tree.rs` (tree drag, Copy / Move to…, UI-30) |
| `src/palette.rs` | Command palette: recent destinations first, `ui_route` opens a command's form instead of running it without params |
| `src/directory.rs` | Projects / Spaces directories, sidebar state (filter, stars kept in egui persisted memory until `user.favourite` exists), Create project / space dialog (UI-8) |
| `src/keys.rs` | Shortcut table, per-view single-key handlers, `?` overlay |
| `src/widgets/` | Shared pieces: `pill`, `who` / `name_of` (avatar by user id), `card`, `field_row`, `empty_state` |
| `src/fonts.rs` | UI font stack (craft-ui `UiFonts`): egui faces, embedded craft-fonts faces, system fonts; `install` at startup (desktop and web) |
| `src/fixtures.rs` | Hostile-data fixtures (`stress`, `empty`) for tests and `ui_shot`'s `{"fixture": …}` first line (doc-hidden; not app code) |
| `src/icons.rs`, `src/theme.rs`, `src/markup_view.rs`, `src/order.rs` | Code-drawn icons, colour tokens, rich-text rendering, ORDER BY rewriting |
| `src/tests/` | kittest UI tests: `mod.rs` (helpers) + one file per area (`nav`, `list`, `board`, `issue`, `wiki`, `keys`, `guard`, `hostile`) |
| `shots/` | One `ui_shot` script per area, writing PNGs under `/tmp/tc-shots/` |
| `examples/ui_shot.rs` | Headless screenshot runner |

## Rules for parallel work

- The shared hot spots are `lib.rs` (`Action`, `Then`, their `apply`/`run` arms) and `route.rs`
  (`View` variants, `parse` arms). **Append-only:** add one variant and one arm per work item at
  the end; don't reorder, rename or reformat neighbours.
- **Tests go in `src/tests/<area>.rs`** (add a new area file and its `mod` line if none fits);
  shared helpers in `src/tests/mod.rs`.
- New shared UI pieces go in `src/widgets/`, one file each, re-exported from `widgets/mod.rs`.
- Every control has an accessible name (`labelled_by`), shortcuts are written as text
  (`crate::key("Mod+K")`), and combo boxes are `craft_ui::ComboBox`: kittest and clippy enforce these.
- Leaving typed text goes through the guard: push `Action::Nav` / `Action::Back`, never call
  `TrackApp::navigate` from a view. Control requests fail with "unsaved changes" unless `"force": true`.

## Screenshots

```sh
mkdir -p /tmp/tc-shots
cargo run -p trackcraft-ui-egui --example ui_shot -- crates/ui-egui/shots/board.jsonl
```

Each line of a script is a `TrackApp::control` request, `{"shot": "/abs/out.png"}` or
`{"steps": n}`. A first line `{"fixture": "stress"|"empty"|"stress-intl"}` swaps the sample
workspace for a hostile-data fixture (`shots/hostile.jsonl`, `shots/empty.jsonl`,
`shots/intl.jsonl`). egui's bundled faces have no emoji, CJK, Arabic or Devanagari glyphs;
`src/fonts.rs` adds them (embedded craft-fonts faces when built with `CRAFT_FONTS_DIR`, then
system fonts). `ui_shot` installs that stack; the plain test `harness()` doesn't, so kittest
panics on those scripts unless a test installs `fonts::install_waiting` and checks
`fonts::missing` first (see `tests/hostile.rs`). Add or extend the script for your area and look
at the PNGs.
