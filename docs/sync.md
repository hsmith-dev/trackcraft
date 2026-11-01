# Live sync

> **Status:** 2026-10-31 · M4.7 — HTTPS in the desktop client, status-bar connection indicator (M4.2: desktop clients edit a server workspace live).

Several desktop clients (and every REST/MCP caller) edit one server-hosted workspace and see each
other's changes as they happen. The server is authoritative; clients apply their own commands
optimistically and converge on the server's order.

```sh
trackcraft-server token create --user riley --data ./data      # prints tct_…
trackcraft --server https://track.example.org --workspace lab --token tct_…
trackcraft --server http://127.0.0.1:8080 --workspace lab --token tct_…   # same computer
# or TRACKCRAFT_TOKEN=tct_… trackcraft --server …, or put the token in
# <config dir>/trackcraft/server-token (macOS: ~/Library/Application Support/trackcraft/)
```

## Model

- **Everything is a command, so sync is a command journal.** Each hosted workspace's `Session`
  keeps a journal (`Session::enable_journal`, switched on by `Hosted::new`). Every local
  `Session::run` that changes the workspace — from `POST /api/command`, the Jira/Confluence
  routes, anything — appends an entry `{rev, command, params, actor, at, idsBefore, origin?}`.
  Recording happens inside `Session::run`, so no surface can forget it.
- **Deterministic replay.** `Session::run` pins the clock for the whole command (`at`: one
  command, one timestamp), and ids come from the workspace's own counter. So the same workspace
  plus the same entry gives the same workspace: `Session::apply_remote(entry)` runs the command
  as `actor` with `now()` pinned to `at`, and checks that `rev` is the next revision and that
  the id counter equals `idsBefore`. A mismatch (`ReplayError`) means the copy diverged.
  `crates/engine/src/tests_sync.rs` replays random command sequences (seeded, 12 × 120
  commands, junk included) and requires equal workspaces.
- **Commands that don't replay are refused remotely.** `trackcraft_engine::LOCAL_ONLY`
  (`file.open`, `file.save`, `workspace.new`, `workspace.import`, `edit.undo`, `edit.redo`,
  `user.switch`) act on the host or on per-session history; the server API answers 403, a remote
  session refuses them, `apply_remote` refuses them. `workspace.import` joined the list in M4.2:
  it reads a host path (on the server that read any file the process could) and would not
  replay elsewhere. Importing into a server workspace needs an upload endpoint (later work).
- **Undo through the server is refused for now.** A remote-mode session (`Session::set_remote`)
  records no undo steps, so `edit.undo` is disabled in the UI. Per-user undo on the server
  (inverse operations in the journal) is later work; it is also what replaces snapshot undo
  (`architecture.md` §3).

## Protocol

Wire types are in `crates/sync-proto` (`trackcraft-sync-proto`, L1, serde only). Every path is
relative to a workspace base (`/w/{name}` or the root for the default workspace) and needs the
usual `Authorization` header (`server.md`).

| Call | Answer |
| --- | --- |
| `GET /api/sync/snapshot[?client=ID]` | `{protocol, epoch, rev, user, appliedSeq?, workspace}` — `workspace` is the `.track` JSON; `appliedSeq` the highest command seq of `client` it contains. |
| `GET /api/sync/events?since=REV&epoch=E[&client=ID]` | `text/event-stream`: `event: hello` `{protocol, epoch, rev, oldest}`, then `event: entry` (`id:` = rev, data = the entry) for each entry after `since`, live; keep-alive comments every 15 s. If `since` can't be served — older than the ring (`oldest`), ahead of the server, or `epoch` differs — a single `event: resync` `{epoch, rev, reason}` and the stream ends. Without `since`: from now. |
| `POST /api/command` `{command, params, origin?: {client, seq}}` | `{command, result, rev, entry, duplicate}` — `entry` is `null` for queries and commands that changed nothing. A `seq` at or below the client's last applied one answers `duplicate: true` and runs nothing (a resend after a lost response). |

- **Revisions** start at 1 for the first change after the server process starts. The **epoch**
  (per hosted workspace per process) says which run they belong to; a restart means a new epoch,
  and clients resync.
- **The ring** holds the last 10 000 entries and at most ~64 MiB of params
  (`JOURNAL_MAX_ENTRIES`, `JOURNAL_MAX_BYTES`); older clients resync from a snapshot. A failed
  multi-command REST request rolls its entries back with the workspace (`Session::rollback_to`).
- The server remembers the last applied `seq` for up to 10 000 client ids.

## Client (`crates/sync-client`, desktop remote mode)

`Remote::connect` fetches the snapshot (the app exits with an error if the server can't be
reached at start) and returns the `Session` the UI edits plus the `Remote` that syncs it.
`Remote::pump(&mut session)` runs every frame (the network threads wake the UI with
`request_repaint`).

- **Forwarding without the UI knowing.** The UI still routes everything through `Session::run`.
  The session's `after_command` observer (`Session::set_after_command`) queues each local change;
  `pump` gives it the next `seq` and hands it to the command thread, which POSTs in order.
- **Confirmed copy + rebase.** The client keeps a hidden *confirmed* session that only applies
  server entries, in revision order (an out-of-order POST answer waits in a buffer for the stream
  to catch up). The visible session is *confirmed + unconfirmed local commands*:
  - our own entry comes back (matched by `origin`) → it is already visible; once nothing is
    pending, the view is reset to the confirmed copy, so it equals the server's byte for byte
    (timestamps included);
  - someone else's entry while nothing is pending → replayed straight onto the view;
  - someone else's entry while we have pending commands → the view is rebuilt: confirmed copy,
    then our pending commands re-run on top. The server will apply ours after theirs too, so
    **server order wins conflicts**: two renames of one issue end with the later one in server
    order, on every client.
- **Offline.** The command thread retries (backoff to 10 s) on network errors, 408, 429 and 5xx,
  keeping order; edits stay visible meanwhile. On reconnect the queue drains; duplicates are
  dropped by the server. The event thread reconnects with `since` = the last revision it saw.
- **Refused commands** (4xx other than 401 — e.g. the issue was deleted meanwhile) are dropped
  from the queue, the view is rebuilt without them, and one error toast lists them
  (`The server refused 1 change(s), which were dropped: issue.comment LUME-3: …`). A 401 keeps
  the queue and shows *Token refused*.
- **Resync.** A gap in the stream, a replay error, an unreadable or oversized event, a `resync`
  event or a restarted server reloads the confirmed copy from a snapshot; pending commands the
  snapshot already contains (`appliedSeq`) are dropped, the rest re-run on top. Only the
  workspace is replaced, so the user's view (selection, open issue, scroll) stays.
- **Status.** `Remote::state()` (Online / Offline / Resyncing / Token refused) and
  `status_text()` (`Offline · 2 unsent`). The desktop app shows it in the status bar's
  connection indicator (M4.7, below), keeps it in the window title, and toasts transitions.
- **Transport.** A small blocking HTTP/1.1 client over `std::net::TcpStream` (`http.rs`:
  Content-Length and chunked bodies, every size capped, header injection refused) — no HTTP
  crates; `https://` goes through rustls (M4.7, below).

## TLS (M4.7)

`--server https://host[:port]` (default port 443) connects with TLS 1.2/1.3 through **rustls**
on the **ring** crypto provider and checks the server's certificate against the **operating
system's trust store** (`rustls-native-certs`: the macOS keychain, the Windows store, the usual
Linux bundle files). The handshake completes before anything is sent, so a refused certificate
never leaks the token.

- **Certificate errors are errors.** There is no `--insecure-skip-verify` and no fallback to
  `http://`. The message says what is wrong and what to do: *not signed by an authority this
  computer trusts (self-signed, or from a private CA): add that CA to the system's trust store*,
  *expired*, *not valid yet (check the clock)*, *issued for a different host name*, *revoked*,
  or *didn't answer with TLS: is it an http:// server?*. A self-hosted server with a private CA
  works once that CA is in the OS trust store. `Config::ca_certs` (DER) replaces the system
  roots for a program that embeds the client (the tests use it); the desktop app has no flag
  for it.
- **`http://` stays** for a server on the same computer (no TLS needed on loopback). Plain
  `http://` to another computer still works (e.g. inside a trusted LAN), but the app warns at
  start and the indicator's tooltip says *Not encrypted*: the token crosses the network in the
  clear.
- **Tests** (`crates/sync-client/src/tests/tls.rs`): an in-process TLS server whose self-signed
  CA (made with `rcgen`) is the only trusted root — requests, the SSE stream, a full `Remote`
  session through a TLS-terminating proxy; untrusted CA, the system store, wrong host name,
  expired certificate, `https://` to a plain server; and that nothing reaches the server when the
  certificate fails.

**Why these crates (licences).** The repo has no `deny.toml`; the policy is AGENTS.md's
(permissive only) and mirrors Pdfcraft's `deny.toml` allow-list (MIT, Apache-2.0, BSD, ISC,
Zlib, Unicode-3.0, BSL-1.0, CC0-1.0). New runtime crates: `rustls` (Apache-2.0 OR ISC OR MIT),
`rustls-webpki` (ISC), `rustls-pki-types` (MIT OR Apache-2.0), `ring` (Apache-2.0 AND ISC — both
on the list), `untrusted` (ISC), `rustls-native-certs` (Apache-2.0 OR ISC OR MIT) with
`security-framework` (macOS) / `schannel` (Windows) / `openssl-probe` (Linux; finds bundle
files, links nothing), all MIT/Apache. `rcgen` (MIT OR Apache-2.0) is a dev-dependency only.
- **ring, not aws-lc-rs:** rustls' default provider, `aws-lc-rs`, needs cmake (and on some
  targets Go/NASM) to build its C library; `ring` needs only a C compiler through `cc`, which
  the suite's CI already has. rustls is built with `default-features = false` and the provider
  passed explicitly (`builder_with_provider`), so no feature unification can switch it.
- **OS roots, not webpki-roots:** `webpki-roots` bundles Mozilla's CA list, whose data is
  **CDLA-Permissive-2.0** — not on the allow-list (Pdfcraft carries it only as a narrow
  build-script exception). The OS store also follows the user's own trust decisions (a company
  CA) and gets updates with the OS.
- **Native only.** `trackcraft-sync-client` is a desktop crate (std sockets, threads); it is not
  in the wasm build. The web client talks to the server through the browser, whose TLS it uses.

## Connection indicator and remote-mode UI (M4.7)

`trackcraft-ui-egui` exposes host hooks (`crates/ui-egui/src/lib.rs`, the M4.7 block):
`TrackApp::set_connection(Option<ConnectionState>)` with `ConnectionState { label, kind:
Online|Syncing|Offline|Error, detail }`, `TrackApp::take_reconnect_request()`, and
`TrackApp::set_remote(bool)` (a remote-mode `Session` counts too). The status bar's left
section shows a dot (filled green: online; ring: syncing; hollow: offline; red: error) and the
label; the tooltip shows `detail` (server, changes waiting, last error, an unencrypted
warning). Clicking it when offline or in error offers **Reconnect now**, which the desktop app
polls each frame and passes to `Remote::reconnect_now()` (the retry threads skip their backoff
wait). On a server workspace Undo/Redo are disabled with the tooltip *Undo isn't available on
server workspaces yet* (their shortcuts show it as a toast; toasts offer no Undo), and
File › Save shows *Saved on the server* instead of a file dialog. `ui.connection` drives the
indicator from the control channel and `ui_shot` (`crates/ui-egui/shots/connection.jsonl`).

## Guarantees and limits

- Every client converges on the server's workspace once its queue is empty and the stream is
  up; tests (`crates/sync-client/src/tests.rs`, real sockets) cover two-client convergence,
  conflicting renames, the offline queue across an outage, refused commands, a server restart
  and a diverged client.
- Not CRDT/OT: concurrent edits to the same field resolve by server order (last writer wins);
  page bodies keep the engine's `baseVersion` check, so a stale page save is refused (and
  reported) rather than merged. Three-way page merge is later work.
- Snapshots are whole workspaces: fine for team-sized workspaces, not for huge ones (deltas from
  the store are later work). The client holds two copies of the workspace in memory.
- Permissions are workspace membership only (as for the REST API); a user removed while
  streaming keeps the open stream until it reconnects.
- Browsers' `EventSource` can't set `Authorization`: the web client needs a cookie or a
  short-lived stream token (later work, with the web client).

## Sharing with Chatcraft (suite server)

Chatcraft (no code yet) needs the same thing: an authoritative server, an ordered per-room
journal, SSE/WebSocket fan-out, optimistic clients with an offline queue. What is app-agnostic
and would move into the proposed shared `craft-collab` crate:

| Piece | Today | Notes |
| --- | --- | --- |
| Wire types: `Entry`, `Origin`, `Hello`, `Resync`, `Snapshot`, `CommandRequest/Response`, `SseParser` | `trackcraft-sync-proto` | Already app-neutral (`command` + `params`); `idsBefore` is an app-defined check, and `Snapshot.workspace` is the app's own document. |
| The journal ring, duplicate suppression by origin, `since`/epoch rules | `trackcraft_engine::sync::Journal` | Lift as a generic `Journal<E>`; the engine keeps the replay (`apply_remote`), which is app logic. |
| Fan-out seam | `trackcraft_api::SyncHub` (`publish` / `subscribe`), `LocalSyncHub` | The suite server passes one hub to both apps (`Server::with_sync_hub`), or bridges it to a shared bus/WebSocket gateway and presence. |
| SSE stream handler, snapshot splice | `crates/api/src/sync.rs` | Generic over "read entries since", "serialize document". |
| Client: confirmed copy + rebase, offline queue, backoff, resync, the std HTTP/SSE transport | `trackcraft-sync-client` | Generic over a `Replica` trait (apply entry, re-run local op, load snapshot) that `Session` implements. |
| Identity | `IdentityProvider` (`server.md`) | Already shared by design. |

Stays per app: the command set, replay determinism, workspace/room membership, the document
format.
