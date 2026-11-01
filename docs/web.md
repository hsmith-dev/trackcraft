# TrackCraft in the browser

> **Status:** 2026-10-31 · M4.4 — first web client (local workspaces; server workspaces wait for sync, M4.2).

`apps/trackcraft-web` runs the same `TrackApp` (`crates/ui-egui`) as the desktop app, compiled to
WebAssembly and hosted by eframe's `WebRunner` on a `<canvas>` (wgpu: WebGPU where the browser
has it, WebGL2 otherwise). The setup mirrors Wordcraft's `apps/wordcraft-web`: the same
`Trunk.toml` (`dist = "../../dist/web"`, `public_url = "./"`, hashed file names), the same
`index.html` shape (one canvas, a loading note, a `data-trunk rel="rust"` link with
`data-wasm-opt="z"`), the same `?webgl` flag, and `packaging/web/` with `_headers`, `.htaccess`
and hosting notes. All JS is the glue wasm-bindgen generates; browser APIs (downloads, the file
picker, `localStorage`, `fetch`) are called from Rust through `web-sys`.

## Building

Needs the `wasm32-unknown-unknown` target and [trunk](https://trunkrs.dev) (trunk fetches a
matching `wasm-bindgen` CLI and `wasm-opt` itself):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked          # or: brew install trunk
cargo xtask web                       # = (cd apps/trackcraft-web && trunk build --release) + header files
```

The site lands in `dist/web` (`index.html`, `trackcraft-web-<hash>.js`,
`trackcraft-web-<hash>_bg.wasm`, `_headers`, `.htaccess`). `cargo xtask web` prints these steps
and succeeds without building when trunk isn't installed. For development,
`trunk serve` in `apps/trackcraft-web` rebuilds on change at http://127.0.0.1:8772/.

`cargo xtask wasm` (part of `cargo xtask ci`) checks the crate with
`cargo check --target wasm32-unknown-unknown`. Natively the crate builds a stub binary that only
says to use the browser, so `cargo test --workspace` runs its unit tests (`src/logic.rs`,
`src/remote.rs`).

## Serving

- **With the team server, same origin:**
  `trackcraft-server serve --data ./data --web dist/web`. Paths no API route matches are served
  from the directory (`crates/server/src/static_files.rs`): `/` and directories give `index.html`,
  `.wasm` is `application/wasm`, `.js` `text/javascript`, hashed files are cached for a year and
  `index.html` is `no-cache`. API routes always win, and API paths (`/api`, `/rest`, `/wiki`,
  `/w/…`) keep their JSON 404s. Paths are percent-decoded and refused if they contain `.`/`..`
  or dot-file segments, backslashes or NULs; the resolved file is canonicalized and must lie inside
  the canonical root (symlinks can't lead out); files over 64 MiB aren't served. Unknown paths
  without an extension get `index.html`; a missing `x.wasm` is a 404. `--web` must name an
  existing directory.
- **Any static host:** upload `dist/web`; see `packaging/web/README.md` (MIME types,
  compression, caching, HTTPS, nginx/Apache/Netlify rules). The site works under any URL prefix.

## What works in the browser

| Desktop | Browser |
|---|---|
| File › New | Same: an empty workspace in memory |
| File › Open… | The browser's file picker (`<input type=file>` made from Rust); a `.track` up to 64 MB is read and opened |
| File › Save, Save As… | Download `<workspace name>.track` (Blob + `<a download>`); clears the unsaved mark |
| Autosave to the file | The open workspace is kept in `localStorage` (`trackcraft.workspace`), at most once a second after a change, and reopened on the next visit |
| Sample on first start | Same; `?sample` opens it even when a workspace is kept |
| Preferences in egui memory | Kept in `localStorage` by eframe's `persistence` |

If the page was served by a TrackCraft server (a same-origin `GET api/health` answers
`{"status":"ok"}`), a note says so: *Server workspaces: open with a token* — opening them needs
sync.

## Limits

- **Server workspaces:** not yet. Live editing of a workspace on the server needs the sync
  protocol (M4.2). The hook is `remote::open_server_workspace` in `apps/trackcraft-web`, which
  returns an error until then; the browser edits a local copy only.
- **Storage:** browsers allow about 5 MB of `localStorage` per origin. A workspace over 4 MB (or
  when storage is blocked, e.g. some private windows) stays in memory only; leaving the page then
  asks first if there are changes that weren't downloaded. Each origin has its own copy; a change
  made after `?sample` replaces the kept workspace. Download to keep a file.
- **No control channel, no MCP, no screenshots** from the page (they are desktop/CLI features).
- **Keyboard:** some browser shortcuts (for example Mod+O in some browsers) may reach the browser
  before the app.
- **Fonts:** the bundled egui fonts only, as on the desktop (no emoji/CJK glyphs yet).
- **Not verified in a browser by the agent that built it:** the build is checked for wasm32
  (`cargo check`/`clippy`), and serving was checked with `curl`; run it in a browser before a
  release.
