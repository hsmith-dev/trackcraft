# Gaps: the ranked work list

> **Status:** 2026-10-31. Pick from the top. When one lands, update it here, in the parity docs
> and in the `ROADMAP.md` progress log.

| # | Gap | Why it ranks here | Milestone |
| --- | --- | --- | --- |
| 1 | No UI: only the CLI and MCP can use TrackCraft | nobody but agents and developers can try it | M1 |
| 2 | Shared server/sync design with Chatcraft undecided | blocks M4; deciding late means two implementations | owners' decision |
| 3 | Snapshot undo is O(workspace) per change; no op log | needed for activity history, sync and large workspaces | M2/M4 |
| 4 | No sprints, backlog or rank | the core Scrum loop | M2 |
| 5 | Rich-text editing is plain text in, ADF/Markdown out; no tables, panels, tasks | Confluence pages need them | M1/M3 |
| 6 | No activity/change history on issues; JQL `WAS`/`CHANGED` missing | expected by every Jira user | M2 |
| 7 | No import from Jira/Confluence | the migration path | M5 |
| 8 | No REST compatibility | existing scripts and integrations | M5 |
| 9 | No custom fields, components, versions | most real projects use them | M2 |
| 10 | JQL ranked comparisons by name (`priority > Low`) only work with numbers | small, visible | M2 |
| 11 | Issue ↔ page links (mention an issue on a page, list pages on an issue) | the reason to have both in one app | M2 |
| 12 | No attachments | images and files on issues and pages | M2/M3 |
