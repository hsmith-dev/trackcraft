# Gaps: the ranked work list

> **Status:** 2026-10-31. Pick from the top. When one lands, update it here, in the parity docs
> and in the `ROADMAP.md` progress log.

| # | Gap | Why it ranks here | Milestone |
| --- | --- | --- | --- |
| 1 | UI polish from [`ui-plan.md`](ui-plan.md): unsaved-edit guard (UI-1), split UI files / shared widgets (UI-2), virtualised list + paging (UI-3), scoped shortcuts from the registry (UI-4), Open Recent menu, dirty-on-close prompt | control channel, autosave, recents and keyboard nav landed 2026-10-31 | M1 |
| 2 | Server as a mountable library + identity/event-bus traits, so the optional suite server can host TrackCraft and Chatcraft together | decided 2026-10-31 (architecture §1); design before M4 code | M4 |
| 3 | Snapshot undo is O(workspace) per change; no op log | needed for activity history, sync and large workspaces | M2/M4 |
| 4 | No sprints, backlog or rank | the core Scrum loop | M2 |
| 5 | Rich-text editing is plain text in, ADF/Markdown out; no tables, panels, tasks | Confluence pages need them | M1/M3 |
| 6 | History gaps: deletes don't record link/parent removals on other issues; no parent/type edit commands; `WAS`/`CHANGED` only for status, assignee, priority; no activity tab in the UI | history + `WAS`/`CHANGED` landed 2026-10-31 (M2.1) | M2 |
| 7 | No import from Jira/Confluence | the migration path | M5 |
| 8 | No REST compatibility | existing scripts and integrations | M5 |
| 9 | No custom fields, components, versions | most real projects use them | M2 |
| 10 | JQL ranked comparisons by name (`priority > Low`) only work with numbers | small, visible | M2 |
| 11 | Issue ↔ page links (mention an issue on a page, list pages on an issue) | the reason to have both in one app | M2 |
| 12 | No attachments | images and files on issues and pages | M2/M3 |
