# Architecture

> **Status:** 2026-10-31 · M0. Revision history at the end.

## 1. Shape of the product

TrackCraft is one application with two halves that share a workspace:

- **Tracker** (Jira's domain): projects → issues; issue types; workflows (statuses with a
  category of To Do / In Progress / Done, and transitions); priorities, labels, estimates,
  assignees, comments, links, sub-tasks; boards; JQL.
- **Wiki** (Confluence's domain): spaces → a tree of pages; every save is a new version; titles
  unique per space; labels; search.

Unlike the single-document Craft apps, Jira and Confluence are multi-user services. TrackCraft
gets there in two steps that share all the code below the UI:

1. **Local-first (M0–M3):** a workspace is one `.track` file, edited by one person in the desktop
   app, the CLI or an agent over MCP. This is where the model, commands and UI mature.
2. **Team server (M4):** `trackcraft-server` hosts many workspaces for many users. Clients (desktop,
   web/WASM, agents, REST callers) send the same **commands** the local app runs; the server
   applies them to its copy, persists them, and broadcasts the results. Offline clients queue
   commands and replay them on reconnect. Conflicts are resolved per field (issues) and by
   version check plus three-way merge (pages, `StaleVersion`).

**Shared with Chatcraft:** Chatcraft (Slack/Teams) needs the same server pieces: accounts, auth
(local + OIDC), permissions, a command/op log, real-time fan-out over WebSocket, presence,
notifications. These should be one shared library (proposed: `craft-collab` in
`storytold/craft-libs`, or a repo of its own) rather than two implementations. That decision
belongs to the owners of both projects (`docs/gaps.md` #2).

## 2. Crates and layers

| Layer | Crate | Job | Depends on |
|---|---|---|---|
| L0 | `trackcraft-core` | `ProjectKey`, `IssueKey`, `Id`/`IdGen`, `Timestamp` | serde, chrono |
| L1 | `trackcraft-markup` | rich text: ADF JSON in/out (unknown nodes preserved), Markdown and plain text out | serde_json |
| L1 | `trackcraft-jql` | JQL lexer → parser → AST → evaluator over a `Record` trait; `ORDER BY` sort | chrono |
| L2 | `trackcraft-model` | `Workspace` and validated edit primitives for tracker and wiki; `IssueRecord` (issues as JQL records) | core, markup, jql |
| L3 | `trackcraft-store` | `.track` format: versioned JSON envelope, schema migration hook, size cap, atomic save | model |
| L4 | `trackcraft-engine` | `Session` (workspace, acting user, undo/redo, clock), `CommandSpec` registry, catalog, sample | all of the above |
| L5 | `trackcraft-mcp` | MCP over JSON-RPC (stdio today), two tools: `trackcraft_commands`, `trackcraft_run` | engine |
| L5 | `trackcraft-api` (M5) | Jira/Confluence-compatible REST routes mapped onto commands | engine |
| L6 | `trackcraft-ui-egui` (M1) | the front end, the same egui fork and conventions as the siblings | engine |
| L7 | apps | `trackcraft-cli`; later `trackcraft` (desktop), `trackcraft-server`, `trackcraft-web` | |

`cargo xtask layers` enforces that every internal dependency points to a lower layer, that
nothing below L6 uses UI crates, and that nothing below L5 uses the async server stack. The
crates below L5 build for `wasm32-unknown-unknown` (`cargo xtask wasm`).

## 3. Commands

Every user-visible action is a `CommandSpec { id, label, location, shortcut, params, mutates,
enabled, run }` registered in `crates/engine/src/cmd/*.rs`. `Session::run(id, params)`:

1. looks the command up and checks `enabled`;
2. snapshots the workspace if the command mutates;
3. runs it inside `catch_unwind`;
4. on success with a change: pushes the snapshot on the undo stack (capped at `UNDO_MAX`), clears
   redo, marks the session dirty; on error or panic: restores the snapshot.

Snapshot undo is simple and correct but O(workspace) per change. The server milestone replaces
it with an operation log (each command records its inverse), which is also what sync needs.

## 4. Data model notes

- Issue numbers come from `Project::next_number` and are never reused, even after deletes.
- Status names are free text matched case-insensitively against the project's workflow; the
  status *category* is what boards, JQL `statusCategory` and resolution logic use.
- `resolved` is set on entering a Done-category status and cleared on leaving one.
- Pages keep their current version inline and earlier versions in `history`; restore creates a
  new version (history is never rewritten). `edit_page` takes the editor's `base_version` and
  refuses stale saves.
- Page trees are bounded (`MAX_PAGE_DEPTH`), and tree walks stay bounded even if a hand-edited
  file contains a parent cycle.

## 5. Formats and compatibility

- **Rich text:** the JSON shape of the public Atlassian Document Format, so content round-trips
  through the Jira/Confluence REST APIs. Unknown nodes are kept verbatim.
- **`.track`:** `{"format":"trackcraft","schema":1,"workspace":{…}}`. Newer schemas are refused
  with a clear error; older ones migrate in `store::from_str`.
- **APIs and imports:** see [`api-compatibility.md`](api-compatibility.md).

## Revision history

- 2026-10-31: first version (M0).
