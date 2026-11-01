# TrackCraft roadmap

**Stage: pre-alpha** · M1 desktop UI landed; next: M1 polish, then M2 tracker depth

> **Last updated:** 2026-10-31 · **Change:** M1 UI, control channel, keyboard navigation, issue history.

TrackCraft aims for parity with Jira (Software and Work Management) and Confluence, then goes
further on speed, offline use, openness and agent control. This page is the summary; the
assessment is [`docs/target-app-parity.md`](docs/target-app-parity.md), the work list is
[`docs/gaps.md`](docs/gaps.md), and milestones and the current focus are in
[`docs/roadmap.md`](docs/roadmap.md).

## Headline numbers (2026-10-31)

| Number | Value | Kind |
| --- | --- | --- |
| Feature catalog coverage | **52 / 110 (47.3%)** of the rows listed so far | measured (`cargo xtask parity` → [`docs/parity-checklist.md`](docs/parity-checklist.md)) |
| Ready for real work | **~15%**: single-user desktop app works; no server, sprints or import | estimated |
| Commands | 39 | measured (`trackcraft-cli commands`) |

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
| M4 | Team server: accounts, permissions, real-time sync, notifications, web client (WASM) | |
| M5 | Compatibility: Jira REST v3 and Confluence REST v2 subsets, Jira/Confluence importers | |
| M6 | Releases through craft-common-actions (macOS, Windows, Linux, web) | |

## Progress log

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
