# Reports

> **Status:** 2026-11-01 · M2.5 / UI-24 — burndown, velocity, cumulative flow, created vs resolved, pie; Reports view and Home dashboard gadgets.

Read-only queries (`.query()`: no undo step, no dirty flag) in `crates/engine/src/cmd/report.rs`,
tested in `crates/engine/src/tests_reports.rs` over hand-built histories with a fixed clock. The
desktop UI draws them in `crates/ui-egui/src/reports.rs` with the painter-based charts in
`crates/ui-egui/src/charts.rs`. The report kinds follow Atlassian's public descriptions of
Jira's agile reports; the rules below are TrackCraft's own.

## Commands

| Command | Params | Answer |
| --- | --- | --- |
| `report.burndown` | `{"sprint": 7 \| "active", "project": "LUME" (with "active"), "unit": "points" \| "issues"}` | `committed`, `scope`, `remaining`, `done`, `days` (`date`, `at`, `offset`, `remaining`, `scope`, `done`, `ideal`), `ideal` (start point, then each day to the end date), `events`, `truncated` |
| `report.velocity` | `{"project", "sprints": 6 (1–50), "unit"}` | `sprints` (oldest first: `id`, `name`, `start`, `completed`, `committed`, `completedValue`), `average` |
| `report.cfd` | `{"project", "from"?, "to"?, "days"?: 30}` | `days` (`date`, `toDo`, `inProgress`, `done`) |
| `report.createdResolved` | `{"project"?, "jql"?, "days": 30 (1–366)}` | `days` (`date`, `created`, `resolved`), totals `created`, `resolved` |
| `report.pie` | `{"jql", "field": "status" \| "assignee" \| "priority" \| "type" \| "labels"}` | `total`, `slices` (`value`, `name`, `count`, `percent`), largest first |

## How history is read

Every report is rebuilt from the issues' change history (`Issue::history`, fields `status`,
`storyPoints`, `sprint`). The **current** value of each field is known; walking the history
backwards, before each entry the field held that entry's `from`. So a field's value at any moment
comes from today's data plus the `from` side of later entries. Entries recorded at exactly a
moment count as having happened at it.

- **Done** means the issue's status, at that moment, was a Done-category status of its project's
  workflow (today's workflow; a status it no longer has counts as To Do, as `statusCategory` does).
- **Estimate** is the story points at that moment; no estimate counts as 0 in points mode. In
  issues mode every issue counts 1.
- **Sprint membership** is anchored where it is known exactly: for a closed sprint at its
  completion (every issue in it then keeps it in `past_sprints`), for an open sprint now
  (`issue.sprint`). Earlier membership walks back through `sprint` entries. History stores sprint
  *names*, so a renamed sprint is matched by its current name plus every name an entry moved an
  issue *into* it under (learnt across all issues). Entries at a closed sprint's completion — the
  completion moving open issues on — happen after its end.
- **Sub-tasks** are left out of sprint reports (they follow their parent, which carries the
  estimate). **Deleted issues** are gone from every report.
- An issue doesn't exist before its `created` time.

## Burndown

- From the sprint's start to its completion (closed) or to now (active — an overdue sprint keeps
  burning down until someone completes it). A future sprint has no burndown ("hasn't started").
- `days`: one row per UTC calendar day, sampled at the end of the day (or at the series end, for
  the last). `remaining` = estimates of issues in the sprint that aren't done; `scope` = all
  issues in the sprint; `done` = the done ones.
- `committed` = the scope at the start instant (issues planned at the moment the sprint started
  count).
- **Ideal**: a straight line from `committed` at the start to 0 at the end date, linear in time.
  `offset` = days since the start, so a chart can place points on a time axis.
- **Events** after the start, one per issue per moment, with `remainingDelta` and `scopeDelta`:
  `added` (entered the sprint), `removed` (left it), `completed` (entered Done), `reopened` (left
  Done), `estimateChanged` (`from`, `to`). Added/removed win over completed/reopened, which win
  over an estimate change at the same moment.

## Velocity

Per closed sprint (the last `sprints`, by completion): **committed** = the sprint's scope at its
start; **completed** (`completedValue`) = estimates of issues in it and done at its completion.
Work finished after a sprint closed doesn't count for it. `average` = mean completed.

## Cumulative flow

Issues of the project that exist at the end of each UTC day, by their status category then.
`to` defaults to today, `from` to `days` (30) days back; at most 366 days.

## Created vs resolved

Per UTC day of the last `days` (ending today): issues **created** that day, and issues whose
**current** resolution date (`resolved`: when they last entered a Done-category status) falls on
it — a reopened and re-resolved issue counts once, on its latest date. Scope: `project`, `jql`,
or both (`project = X AND (jql)`); `"jql": ""` alone = every issue.

## Pie

Counts of the JQL's issues by field. Labels: one count per label (none = "No labels"). Assignee:
display names (none = "Unassigned"). At most 50 slices; the rest are summed as "Other".

## Bounds and determinism

- At most 366 days per series, 20 000 issues per report (more: the first 20 000 and
  `"truncated": true`), 2 000 events, 50 velocity sprints, 50 pie slices.
- Date arithmetic is checked; extreme dates (year 1 to year 9000) answer within the day cap.
- Time comes from the session clock (`Session::with_clock`) and days are UTC calendar days: the
  same workspace and clock give the same report on every machine.

## Permissions

Reports need no authorization rule: they are queries. Under enforcement (servers) `Session::run`
runs every query on `Workspace::view_for(user)`, so a report only counts what the acting user may
browse: a private project's sprints, issues and history are absent — asking for them fails as
"no sprint" / "no project", and JQL-scoped reports and pies leave them out. Proven by
`tests_reports::reports_only_count_what_the_user_can_browse`.

## UI (UI-24)

- **Reports** view (`View::Reports(project)`; sidebar → project → Reports; `ui.navigate
  {"view": "reports", "project": "LUME"}`; `ui.reports {"tab", "sprint", "unit", "days", "data"}`):
  Burndown (sprint and unit pickers, scope-change table), Velocity (unit), Cumulative flow and
  Created vs resolved (14, 30 or 90 days).
- Charts are painted with egui's painter (no plotting crate; Gridcraft's chart crate is tied to
  its spreadsheet model). Three categorical colours per theme (blue, orange, aqua), validated for
  colour-blind separation and contrast against our light and dark surfaces; reference lines
  (ideal, committed) in neutral grey; 2 px lines with an end dot, bars at most 24 px with 4 px
  rounded data ends, hairline grid. Every chart has an accessible name, a legend, a hover tooltip
  with exact values, and a **Show data** toggle listing every value as a table.
- **Home** is a two-column dashboard of fixed gadgets: Assigned to me, Created vs resolved
  (14 days, every visible issue), Active sprint (mini burndown), Recently updated pages.
  User-arranged dashboards come later.
