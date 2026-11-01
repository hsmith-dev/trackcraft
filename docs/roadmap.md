# Roadmap: milestones and current focus

> **Status:** 2026-10-31 · M0 done; M1.1–M1.6 landed; M1.7 and polish open.

## Current focus: M1 — desktop app (landed 2026-10-31; finishing)

Done: M1.1–M1.6 below. Landed later the same day: M1.7 (control channel, autosave, recent files) and M1.8 (keyboard navigation). Open (see `ui-plan.md` UI-1…UI-10): unsaved-edit guard, UI file split, virtualised list, Open Recent menu, window geometry, interface translations (craft-i18n), and board column order by rank (needs M2 rank).

Bring up `crates/ui-egui` and `apps/trackcraft` on the suite's egui fork (same pins and patch
section as `../Wordcraft/wordcraft/Cargo.toml`), with the siblings' conventions: theme tokens,
icons drawn in code, `ui_shot` offscreen screenshots, a control channel, command palette.

1. **M1.1** App shell: left sidebar (Projects / Spaces / Filters), top bar with the search field,
   open/save `.track`, command palette (`app.commands`), undo/redo.
2. **M1.2** Issue list: JQL bar with error display, sortable columns, paging.
3. **M1.3** Board: columns per status, cards, drag a card to transition (through `issue.transition`).
4. **M1.4** Issue view: fields panel, description (Markdown-ish editor over the ADF model),
   comments, links, sub-tasks, transition button with available transitions.
5. **M1.5** Create-issue dialog (`C`), assign to me (`I`), keyboard shortcuts from the registry.
6. **M1.6** Wiki: page tree sidebar, page view, page editor with conflict prompt, history panel
   with restore.
7. **M1.7** Control channel + `ui_shot` example; snapshot tests of each screen.

Gate for M1: someone can run a small team's tracker and handbook from one `.track` file without
the CLI.

## Later milestones

- **M2 tracker depth:** backlog and sprints, rank, versions/releases, components, due dates,
  custom fields (`cf[…]` in JQL), workflow editor, activity history (needs the op log), watchers,
  attachments, saved filters, JQL `WAS`/`CHANGED`, CSV export, dashboards.
- **M3 wiki depth:** tables, panels, task lists, macros (TOC, children, issue lists), page and
  inline comments, version diff, templates, attachments/images, PDF export (craft-pdf), DOCX
  export, blog posts, CQL.
- **M4 team server:** `trackcraft-server` (standalone) and, optionally, the suite server that
  installs TrackCraft and Chatcraft together (architecture §1); accounts, permissions, real-time
  sync, notifications, web client.
- **M5 compatibility:** REST API subsets and importers (`api-compatibility.md`).
- **M6 releases:** packaging and signing through craft-common-actions.
