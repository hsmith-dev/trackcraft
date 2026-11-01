# The TrackCraft server

> **Status:** 2026-10-31 · M4.2 — live sync (journal, SSE events, snapshot; [`sync.md`](sync.md)) on the M4.1 server and REST API subset.

`trackcraft-server` hosts TrackCraft workspaces for a team and answers a subset of the Jira Cloud
REST v3, Jira Software Agile 1.0 and Confluence Cloud REST v2 APIs (see
[`api-compatibility.md`](api-compatibility.md)), plus TrackCraft's own command endpoint. Scripts,
CI integrations and agents written against those public APIs work against it for the covered
calls.

## Crates

| Crate | Layer | Job |
| --- | --- | --- |
| `crates/api` (`trackcraft-api`) | L5 | Route handlers: maps each request onto engine commands and answers in the public shapes. Defines the `IdentityProvider` and `EventSink` traits and the workspace `Hub`. No runtime, no file I/O. |
| `crates/server` (`trackcraft-server-lib`, lib `trackcraft_server`) | L6 | The mountable server: `Server::open(data_dir)`, `router(&server) -> axum::Router`, `mount`, token auth, body limit, timeout, request log, debounced atomic saves. Spawns tasks on the caller's runtime; never creates one. No UI crates (`cargo xtask layers`). |
| `apps/trackcraft-server` | L7 | The binary: argument parsing, the Tokio runtime, Ctrl-C shutdown. |
| `crates/sync-proto` (`trackcraft-sync-proto`) | L1 | Live-sync wire types and the SSE line parser (serde only; [`sync.md`](sync.md)). |
| `crates/sync-client` (`trackcraft-sync-client`) | L5 | The blocking desktop sync client (std `TcpStream`, no runtime). |

The library sits at L6 only because it depends on `trackcraft-api` (L5) and layers point strictly
downwards; it is not a UI crate.

## Running it

```sh
cargo build --release -p trackcraft-server
trackcraft-server workspace create lab --sample --data ./data       # or: workspace create acme --user ann
trackcraft-server token create --user riley --email riley@example.com --data ./data   # prints the token once
trackcraft-server serve --data ./data --listen 127.0.0.1:8080
curl -u riley@example.com:$TOKEN http://127.0.0.1:8080/rest/api/3/myself
```

| Command | What it does |
| --- | --- |
| `serve [--data DIR] [--listen ADDR] [--base-url URL] [--timeout SECS] [--max-body BYTES] [--quiet]` | Serves every `DIR/NAME.track`. Defaults: `./data`, `127.0.0.1:8080` (loopback only — use `--listen 0.0.0.0:8080` behind a TLS proxy to serve a network), 30 s, 4 MiB. Logs one line per request to stderr (method, path without query, status, time) unless `--quiet`. Ctrl-C saves and exits. |
| `token create --user ID [--email E] [--label L]` | Mints a token for a workspace user id. |
| `token list`, `token revoke TOKEN_ID` | Lists token ids/users (never the tokens), revokes one. |
| `workspace create NAME [--sample] [--user ID]`, `workspace list` | Creates `NAME.track` (never overwrites); names are `a-z 0-9 - _`, at most 64. |

Workspaces are loaded at start-up; restart to pick up a workspace file added while running (token
changes are picked up live).

## Data directory and config

```
data/
  server.json        # tokens (hashes), optional defaultWorkspace and baseUrl; mode 0600
  lab.track          # one file per workspace; the file stem is its name
  acme.track
```

```json
{
  "defaultWorkspace": "lab",
  "baseUrl": "https://track.example.com",
  "tokens": [{"id": "1a1f5715", "user": "riley", "email": "riley@example.com", "label": "ci", "sha256": "…", "created": "…"}]
}
```

- **Workspaces in URLs.** Every route is served under `/w/{name}/…` and, for the default workspace,
  at the root (`/rest/api/3/…`). Without `defaultWorkspace` the root serves the only workspace when
  there is exactly one; with several it answers 404 naming the `/w/{name}` form. A Jira client
  pointed at `https://host/w/acme` therefore talks to `acme`.
- **`baseUrl`** is used in `self` links; without it they are built from the `Host` header.
- **Persistence.** Each workspace is one `Session` behind a mutex. After a request that changed it,
  the saver waits 500 ms (changes in that window are written together), copies the workspace under
  the lock and writes it with `trackcraft_store::save` (temp file + rename — never in place). The
  server also saves on shutdown. A failed save is logged and retried after the next change.

## Auth

- `Authorization: Bearer <token>`, or `Authorization: Basic base64(<user id or email>:<token>)` —
  the form Jira/Confluence clients send with an email and API token. With Basic, the user name
  must be the token's user id or its `--email`.
- Tokens are `tct_` + 64 hex characters from the OS random source. Only their SHA-256 is stored.
- A token names a user id; the user must be a member of the workspace (`user.add`), or requests
  get 403. Every command runs **as that user** (reporter, comment author, history author,
  `currentUser()` in JQL): the session's acting user is set for the locked call and restored
  afterwards, so concurrent requests from different users never see each other's identity.
- There are no per-project permissions yet: any member may do anything a command allows (the
  engine's own rules still apply, e.g. only a comment's author may edit it).
- Over the native endpoint the server refuses commands that act on the server rather than the
  workspace: `file.open`, `file.save`, `workspace.new`, `edit.undo`, `edit.redo`, `user.switch`
  (403), and `workspace.import` (it reads a path on the host) — the engine's `LOCAL_ONLY` list.
  Undo/redo is per-session in the engine and would undo another user's change.

## Limits and errors

- Request bodies above `--max-body` → 413. Requests longer than `--timeout` → 503 (the command
  itself still completes or fails atomically under the lock). Page sizes are capped: Jira search
  100, comments/changelog 1000, Agile lists 50, Confluence `limit` 250.
- Errors use each surface's public shape — Jira `{"errorMessages": […], "errors": {}}`,
  Confluence `{"errors": [{"status", "code", "title", "detail"}]}`, native `{"error", "status"}` —
  including 401/404/405/413 from the router itself. Missing things are 404, a stale page version
  409, anything else wrong with the request 400. Messages are capped at 1000 characters.
- Multi-command requests (`PUT /issue` with fields + assignee, a transition with a comment) are
  all-or-nothing: if a later step fails, the workspace is put back.
- Handlers never panic; engine commands run inside `Session::run`'s guard, and a crashed handler
  thread becomes a JSON 500.

## Native API

- `GET /api/health` (no auth) → `{"status": "ok", "version"}`.
- `GET /api/commands` → every command (`app.commands`), server-blocked ones marked `disabled`.
- `POST /api/command {"command": "issue.create", "params": {…}, "origin"?: {"client", "seq"}}` →
  `{"command", "result", "rev", "entry", "duplicate"}` — the same commands and params as the CLI
  and MCP (`trackcraft-cli commands`); `rev`/`entry` are the sync revision and journal entry
  ([`sync.md`](sync.md)).
- `GET /api/sync/snapshot`, `GET /api/sync/events?since=REV&epoch=E` (Server-Sent Events) — live
  sync, see [`sync.md`](sync.md). Every change from any route is journaled.
- `GET /api/workspace` → the workspace's name and counts.

## Suite server

The suite server (TrackCraft + Chatcraft on one host, `docs/architecture.md` §1) is another thin
binary that mounts both libraries:

```rust
let track = trackcraft_server::Server::new(hub, shared_identity.clone(), shared_events.clone(), Options {
    base_url: Some("https://suite.example.com/track".into()),
    ..Options::default()
});
let app = trackcraft_server::mount(chatcraft_server::mount(Router::new(), "/chat", &chat), "/track", &track);
track.spawn_saver();
axum::serve(listener, app).await?;
```

- **Identity.** `trackcraft_api::IdentityProvider` (`authenticate(&Credential) -> Option<Identity>`)
  is the seam: standalone it is `TokenIdentity` over `server.json`; the suite passes one shared
  provider (sessions, SSO, shared tokens — proposed home `craft-collab`) to both apps, so a user
  signs in once. Workspace membership stays TrackCraft's: the shared provider returns a user id,
  and TrackCraft checks that id is a member of the workspace.
- **Events.** `trackcraft_api::EventSink` receives a `CommandEvent { workspace, user, command,
  params, result }` for every successful mutating command, after the workspace lock is released.
  Standalone it is `NullSink` (or `MemorySink`); the suite's sink feeds the integration API
  (issue unfurls, notifications in Chatcraft).
- **Sync fan-out.** `trackcraft_api::SyncHub` (`publish(workspace, rev)`, `subscribe(workspace)`)
  wakes the SSE streams after each change; standalone `LocalSyncHub`, in the suite one shared hub
  via `Server::with_sync_hub` (what moves to `craft-collab`: [`sync.md`](sync.md)).
- **Hosting.** `Hub` is plain data (`Arc<Hosted>` per workspace), so a suite can build it from its
  own storage instead of a directory, and later swap the file store for a database store.
- Mounted under `/track`, request paths seen by the API are prefix-stripped by axum; set
  `base_url` to the prefixed origin so `self` links include `/track`.

## Not yet

TLS (use a reverse proxy), per-project permissions, rate limiting, WebSocket push, per-user
undo through the server, a persisted journal (revisions restart with the process), attachments, OAuth, and the Jira/Confluence endpoints not listed in
[`api-compatibility.md`](api-compatibility.md).
