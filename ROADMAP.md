# TrackCraft roadmap

**Stage: pre-alpha** · desktop app, backlog/sprints, wiki depth, first server and importers landed; next: M4 multi-user (sync, permissions, web client)

> **Last updated:** 2026-10-31 · **Change:** M1 UI, control channel, keyboard navigation, issue history.

TrackCraft aims for parity with Jira (Software and Work Management) and Confluence, then goes
further on speed, offline use, openness and agent control. This page is the summary; the
assessment is [`docs/target-app-parity.md`](docs/target-app-parity.md), the work list is
[`docs/gaps.md`](docs/gaps.md), and milestones and the current focus are in
[`docs/roadmap.md`](docs/roadmap.md).

## Headline numbers (2026-10-31)

| Number | Value | Kind |
| --- | --- | --- |
| Feature catalog coverage | **68 / 116 (58.6%)** of the rows listed so far | measured (`cargo xtask parity` → [`docs/parity-checklist.md`](docs/parity-checklist.md)) |
| Ready for real work | **~42%**: teams can import, plan sprints and keep a wiki, edit the same workspace live from several desktop apps through `trackcraft-server`, and script it over Jira-compatible REST; no permissions, browser editing of server workspaces, HTTPS in the client or notifications yet | estimated |
| Commands | 63 | measured (`trackcraft-cli commands`) |

The catalog is young: it lists the headline features of both products, not their full depth, so
the percentage will *fall* as rows are added. Read it as "of what we have catalogued", not as
parity.

## Milestones

| | Milestone | Status |
| --- | --- | --- |
| M0 | Foundation: crates, command registry, JQL, `.track` store, MCP, CLI, quality gates | done |
| M1 | Desktop app (egui): sidebar, issue list + JQL bar, issue view/edit, board, page tree, page view/edit | landed; polish open |
| M2 | Tracker depth: backlog, sprints, rank, versions, components, custom fields, workflow editor, activity history, saved filters | |
| M3 | Wiki depth: rich editor (tables, panels, tasks, macros), comments, version diff, templates, attachments, export | |
| M4 | Team server: accounts, permissions, real-time sync, notifications, web client (WASM) | REST + tokens (M4.1), live sync (M4.2), web client in local mode (M4.4) landed; permissions, notifications, browser sync open |
| M5 | Compatibility: Jira REST v3 and Confluence REST v2 subsets, Jira/Confluence importers | REST subsets and importers landed; depth open |
| M6 | Releases through craft-common-actions (macOS, Windows, Linux, web) | |

## Progress log

- **2026-11-01 · M4.2, M4.4, fonts, UI fixes** (four agents in parallel) — Live sync: a server
  journal streamed over SSE, deterministic replay in the engine, desktop remote mode
  (`trackcraft --server … --workspace … --token …`) with optimistic edits, an offline queue and
  snapshot resync (`docs/sync.md`, protocol types ready to share with Chatcraft); the web client
  (`apps/trackcraft-web`, trunk, served by `trackcraft-server --web`); real glyphs for CJK,
  Arabic, Devanagari and emoji via system fonts and craft-fonts; File › Import, rich compare
  view, peek pane, cross-space page moves.

- **2026-10-31 · UI-fixes** — peek pane on the issue list, board and backlog (`Space`, hover
  "Peek", follows the selection, `ui.peek`); File › Import… for Jira CSV / JSON and Confluence
  exports with a report dialog (`ui.import`); the version compare view renders blocks as rich
  text; inline-comment quotes end at a word with an ellipsis; `page.move` takes `space` and moves
  a subtree (comments, history) between spaces, refusing title clashes; Move to… and sidebar drops
  work across spaces.

- **2026-10-31 · UI-7, UI-9/13, UI-27..30, M4.1, M5.1** (five agents in parallel) — Settings
  (theme, density, workspace, people, autosave after the last change); issue view polish
  (activity tabs, link/unlink, comment edit, pickers, label chips, create dialog); wiki screens
  (page and block comments, version compare, templates, page-tree drag and copy);
  `trackcraft-server` with token auth and Jira/Confluence-compatible REST subsets, built as a
  mountable library; importers for Jira CSV/JSON and Confluence exports
  (`trackcraft-cli import`, `workspace.import`). Verified end to end: CSV import → server →
  curl search/create/transition/changelog.

- **2026-10-31 · M2.3, M3.1, UI-5, UI-6, UI-8** (five agents in parallel) — Tracker commands
  (workspace rename, user edit, unlink, comment edit, issue type/parent changes, sprint fixes,
  `Session::revision`); wiki depth in the engine (page and inline comments, version diff,
  templates, page copy and ordering); toast stack with Undo and a status bar; empty states and a
  hostile-data pass with a stress fixture; a sidebar that scales (filter, starred, recent,
  directories) with back/forward and palette routing.

- **2026-10-31 · UI-1, UI-2, UI-3, UI-4, M2.2, UI-11/12** (agents in parallel) — Unsaved-edit
  guard; UI crate split into modules; virtualised issue list with paging and a column chooser;
  one scoped keymap from the command registry (redo-while-typing fixed, G-chords, palette shows
  shortcuts); backlog, rank and sprints in the engine with JQL `sprint`/`openSprints()`/
  `ORDER BY rank`; backlog screen with drag rank, sprint start/complete/edit dialogs, sprint board.

- **2026-10-31 · M1.7, M1.8, M2.1** (three agents in parallel) — Control channel for the running
  app (`trackcraft --control`, `trackcraft-cli send`), autosave and recent files; keyboard
  navigation (list j/k, board h/j/k/l and Shift+Arrow transitions, issue a/m/e, `?` shortcut
  help) and sortable list columns; issue activity history (`issue.history`) and JQL
  `WAS`/`CHANGED`. UI plan written: [`docs/ui-plan.md`](docs/ui-plan.md).

- **2026-10-31 · M1** — Desktop app (`apps/trackcraft`) and `crates/ui-egui` on the suite's egui
  fork with craft-ui: sidebar (filters, projects, page trees), JQL issue list, board with
  drag-to-transition, issue view (inline edits, status, assignee, priority, labels, points,
  sub-tasks, links, comments), create dialog, wiki page view, Markdown editor with live preview
  and conflict handling, version history with restore, command palette, light/dark themes, icons
  drawn in code; `ui_shot` headless screenshots; Markdown in for issue and page text.

- **2026-10-31 · M0** — Workspace in the Craft layout; `core`, `markup` (ADF-shaped rich text),
  `jql` (parser + evaluator), `model` (tracker + wiki), `store` (atomic `.track` files),
  `engine` (38 commands, undo/redo, catalog, sample), `mcp`, `trackcraft-cli`; `cargo xtask ci`.
