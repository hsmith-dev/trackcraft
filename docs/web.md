# TrackCraft in the browser

> **Status:** 2026-11-01 · M6.1 — release zip (`packaging/web/package.sh`), checked rendering in headless Chrome; M4.6 — the web client edits server workspaces live (and local ones as before).

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
`packaging/web/package.sh` builds the same site and zips it as `trackcraft-web-<version>.zip`, the
release asset ([`releasing.md`](releasing.md)). The release WASM is about 13 MB (about 4.7 MB
zipped); a server or CDN that compresses `.wasm` cuts the download accordingly.

`cargo xtask wasm` (part of `cargo xtask ci`) checks the crate with
`cargo check --target wasm32-unknown-unknown`. Natively the crate builds a stub binary that only
says to use the browser, so `cargo test --workspace` runs its unit tests (`src/logic.rs`,
`src/remote.rs`, `src/sync.rs` with its fake-transport tests in `src/sync/tests.rs`).

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

## Server workspaces (M4.6)

If the page was served by a TrackCraft server (a same-origin `GET api/health` answers
`{"status":"ok"}`), a **Connect to a server workspace** dialog opens:

- **Workspace** — the name on the server (`NAME` of `DATA/NAME.track`); empty = the server's
  default workspace. (The server has no route that lists workspaces, so it's a text field.)
- **Token** — from `trackcraft-server token create --user NAME` (a password field).
- **Remember on this device** — only if ticked, the token (and workspace name) are kept in
  `localStorage` (`trackcraft.server.token`, `trackcraft.server.workspace`) and the page connects
  by itself on the next visit. The dialog says the caveat: anyone using that browser profile (and
  any script on the origin) can read it, so leave it unticked on shared computers. Unticked, the
  token lives in memory only and a reload asks again.
- **Work locally** closes the dialog; a small panel at the bottom right reopens it.

Once connected the same `TrackApp` edits the server's workspace live: other people's changes
appear as they happen, yours are applied at once and sent in order. The document title shows
the workspace and the state (`Lab — TrackCraft · lab · Offline · 2 unsent`); the bottom-right
panel shows the same with **Disconnect** and **Log out** (disconnect and forget the remembered
token). Changes of state, refused changes and resyncs are toasts (`TrackApp::notify`).

- **Offline:** when `navigator.onLine` goes false or requests fail, edits stay visible and are
  queued; the page retries (backoff up to 10 s, at once when the browser says it's back online)
  and replays the queue in order. Leaving the page while changes are unsent asks first (they
  live only in that tab).
- **Local mode is unchanged:** while connected, nothing is written over the workspace kept in
  `localStorage`; Disconnect / Log out goes back to it. File › Open of a local file disconnects
  first; File › Save downloads a copy of the server workspace; File › New is refused (as in the
  desktop's remote mode: `workspace.new` is local-only).
- How it works (`src/sync.rs`, `src/sync_web.rs`) and the stream-ticket auth: `docs/sync.md`,
  *Browser transport*.

## Limits

- **Server workspaces:** the page must be served by the server (same origin: no CORS). Undo is
  disabled in server mode (as on the desktop). No idle timeout on the event stream: `EventSource`
  doesn't surface the server's keep-alive comments, so a silently dead connection is noticed only
  when the browser reports an error. Unsent changes are lost if the tab is closed anyway.
- **Storage:** browsers allow about 5 MB of `localStorage` per origin. A workspace over 4 MB (or
  when storage is blocked, e.g. some private windows) stays in memory only; leaving the page then
  asks first if there are changes that weren't downloaded. Each origin has its own copy; a change
  made after `?sample` replaces the kept workspace. Download to keep a file.
- **No control channel, no MCP, no screenshots** from the page (they are desktop/CLI features).
- **Keyboard:** some browser shortcuts (for example Mod+O in some browsers) may reach the browser
  before the app.
- **Fonts:** the bundled egui fonts only, as on the desktop (no emoji/CJK glyphs yet).
- **Browser check (M6.1):** the release build (`cargo xtask web`) served by
  `trackcraft-server serve --web dist/web` was rendered in headless Chrome
  (`--headless=new --screenshot --virtual-time-budget=15000`, both the default renderer and
  `?webgl` with `--use-gl=angle --use-angle=swiftshader`): the WASM app starts, shows the sample
  workspace's Home and opens the **Connect to a server workspace** dialog. That check found and
  fixed an empty bottom-right panel drawn behind the dialog (it now appears only once the dialog
  is closed or a workspace is connected). Not yet exercised in a browser: typing, clicking, a
  real connect with a token, the `fetch`/`EventSource` sync driver, downloads and the file
  picker (the sync state machine is tested natively against a fake transport, the ticket route
  against the real router).
