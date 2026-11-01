# TrackCraft roadmap

**Stage: pre-alpha** · next: M1 desktop UI

> **Last updated:** 2026-10-31 · **Change:** M0 foundation (workspace, engine, JQL, store, MCP, CLI, gates).

TrackCraft aims for parity with Jira (Software and Work Management) and Confluence, then goes
further on speed, offline use, openness and agent control. This page is the summary; the
assessment is [`docs/target-app-parity.md`](docs/target-app-parity.md), the work list is
[`docs/gaps.md`](docs/gaps.md), and milestones and the current focus are in
[`docs/roadmap.md`](docs/roadmap.md).

## Headline numbers (2026-10-31)

| Number | Value | Kind |
| --- | --- | --- |
| Feature catalog coverage | **49 / 109 (45.0%)** of the rows listed so far | measured (`cargo xtask parity` → [`docs/parity-checklist.md`](docs/parity-checklist.md)) |
| Ready for real work | **~5%**: no UI, no server, no import | estimated |
| Commands | 38 | measured (`trackcraft-cli commands`) |

The catalog is young: it lists the headline features of both products, not their full depth, so
the percentage will *fall* as rows are added. Read it as "of what we have catalogued", not as
parity.

## Milestones

| | Milestone | Status |
| --- | --- | --- |
| M0 | Foundation: crates, command registry, JQL, `.track` store, MCP, CLI, quality gates | done |
| M1 | Desktop app (egui): sidebar, issue list + JQL bar, issue view/edit, board, page tree, page view/edit | next |
| M2 | Tracker depth: backlog, sprints, rank, versions, components, custom fields, workflow editor, activity history, saved filters | |
| M3 | Wiki depth: rich editor (tables, panels, tasks, macros), comments, version diff, templates, attachments, export | |
| M4 | Team server: accounts, permissions, real-time sync, notifications, web client (WASM) | |
| M5 | Compatibility: Jira REST v3 and Confluence REST v2 subsets, Jira/Confluence importers | |
| M6 | Releases through craft-common-actions (macOS, Windows, Linux, web) | |

## Progress log

- **2026-10-31 · M0** — Workspace in the Craft layout; `core`, `markup` (ADF-shaped rich text),
  `jql` (parser + evaluator), `model` (tracker + wiki), `store` (atomic `.track` files),
  `engine` (38 commands, undo/redo, catalog, sample), `mcp`, `trackcraft-cli`; `cargo xtask ci`.
