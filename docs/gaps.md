# Gaps: the ranked work list

> **Status:** 2026-10-31. Pick from the top. When one lands, update it here, in the parity docs
> and in the `ROADMAP.md` progress log.

| # | Gap | Why it ranks here | Milestone |
| --- | --- | --- | --- |
| 1 | UI polish from [`ui-plan.md`](ui-plan.md): Open Recent menu, dirty-on-close prompt, Alt+Left while typing on macOS, keyboard move into the "Other statuses" board column fails with an error, long project names wrap in the sidebar, the 10k-issue board pages cards ("Show more") rather than virtualising; bundled font lacks CJK/Arabic/Devanagari/emoji (boxes, no crash); the issue table doesn't shrink when the peek pane is open (Priority column clipped); no import in the web client (needs an import-from-bytes entry point) | control channel, autosave, recents and keyboard nav landed 2026-10-31; peek pane, rich compare view, word-boundary quotes, cross-space page moves and File › Import landed 2026-10-31 (UI-fixes) | M1 |
| 2 | Server as a mountable library + identity/event-bus traits, so the optional suite server can host TrackCraft and Chatcraft together | decided 2026-10-31 (architecture §1); design before M4 code | M4 |
| 3 | Snapshot undo is still O(workspace) per change; no undo in remote mode (server-side per-user undo needed) | the sync journal landed 2026-11-01 (M4.2) | M4 |
| 4 | Sprint follow-ups: sprint-board keyboard navigation walks the whole project's cards (UI) | backlog, rank and sprints landed 2026-10-31 (M2.2, UI-11/12); completion guard, start name, create-in-sprint and date clearing landed (M2.3) | M2 |
| 5 | Rich-text editing is plain text in, ADF/Markdown out; no tables, panels, tasks | Confluence pages need them | M1/M3 |
| 6 | History gaps: `WAS`/`CHANGED` only for status, assignee, priority; no activity tab in the UI | history + `WAS`/`CHANGED` landed 2026-10-31 (M2.1); delete history, type/parent edits, unlink, comment edit landed (M2.3) | M2 |
| 7 | Import depth: Confluence tables and macros kept as unrendered blocks; CSV sprint states guessed; no attachments | importers landed 2026-10-31 (M5.1, `docs/import.md`) | M5 |
| 8 | Server depth: no admin UI for roles/restrictions (commands and REST exist; `access.check` ready for the UI), no email notifications, no workspace list route (the web connect dialog asks for the name), browser-side sync untested in a real browser, the `trackcraft-server user role` CLI edits the file directly (stop the server first), metadata leaks listed in `permissions.md` | REST, sync, permissions, notifications, HTTPS landed (M4.1–M4.7) | M4 |
| 9 | No custom fields, components, versions | most real projects use them | M2 |
| 13 | Marking notifications read is an undo step | Cmd+Z after reading the inbox un-reads instead of undoing work | M4 |
| 10 | JQL ranked comparisons by name (`priority > Low`) only work with numbers | small, visible | M2 |
| 11 | Issue ↔ page links (mention an issue on a page, list pages on an issue) | the reason to have both in one app | M2 |
| 12 | No attachments | images and files on issues and pages | M2/M3 |
