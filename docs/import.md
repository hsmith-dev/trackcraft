# Imports

> **Status:** 2026-10-31 · M5.1 — Jira CSV, Jira REST v3 JSON, Confluence HTML export and
> storage-format pages.

TrackCraft reads the export formats Atlassian **publicly documents** — the "export issues to
CSV" columns, the Jira Cloud REST API v3 `search/jql` response, and the Confluence space export
(HTML) and storage format — into a workspace. The code is in `crates/import`
(`trackcraft-import`, layer L3). All test fixtures are synthetic and written in code; no
exported file is ever committed.

## Running it

```sh
trackcraft-cli import --into team.track --jira-csv export.csv
trackcraft-cli import --into team.track --jira-json page.json
trackcraft-cli import --into team.track --confluence space-export.zip   # or the unpacked folder
trackcraft-cli import --into team.track --confluence runbook.xml --space OPS  # one storage-format page
```

`--into` is created when missing. The workspace is saved atomically and the report is printed as
JSON (`projects`, `issues`, `users`, `sprints`, `comments`, `links`, `spaces`, `pages`,
`warnings`). `--as USER` sets the importing user (default: the workspace's first user, else
`importer`, created if needed).

From any front end, the engine command does the same on the open workspace, as one undo step:
`workspace.import {"kind": "jiraCsv" | "jiraJson" | "confluence", "path": "…", "space": "KEY"}`.
Library callers use `trackcraft_import::{jira_csv, jira_json, confluence_html}` with an `Actor
{ user, now }`.

## Safety

Every input is treated as hostile.

- **Caps:** input file 64 MiB; 20,000 issues; 2,000 CSV columns (a wider header is refused);
  1 MiB per field (longer ones are clipped, with a warning); 1,000 comments / links / sprints
  per issue; 10,000 pages; 50,000 archive or folder entries; 256 MiB unpacked from a .zip
  (16 MiB per page); HTML nesting 128 elements and 500,000 nodes per page; JSON nesting 128
  (serde_json's limit — a deeper file is refused).
- **Zip archives** with an unsafe entry name (`..`, an absolute path, a drive letter, a NUL)
  are refused as a whole. Nothing is ever written to disk from an archive; only `.html` pages
  are read. Symlinks in an export folder are skipped.
- **All or nothing for fatal errors:** an import runs on a copy of the workspace and replaces it
  only on success, so a refused file changes nothing.
- **Row-level problems are warnings,** never failures: a bad key, an unknown parent, a date we
  can't read, an invalid label… The issue is imported without the bad part when possible.
  Warnings are listed up to 500, then counted.
- **Markup bombs** (unclosed `*`, `[[[[…`, `{{{{…`, thousands of nested list markers or
  `{quote}`s) are bounded: list nesting stops at 10 levels, emphasis at 6, quote/panel nesting
  at 16, and every scan runs once over the text.

## Jira CSV

Columns are matched by name, ignoring case; repeated columns (`Labels`, `Sprint`, `Comment`,
issue links) are all read.

| Column(s) | Becomes |
|---|---|
| `Issue key` | the issue's key, kept as is (`ABC-17`). The project's next number moves past the highest imported one. |
| `Summary` | summary (control characters → spaces, clipped to 255; empty → "(no summary)") |
| `Issue id` | used to resolve `Parent` when it holds an id |
| `Project name`, `Project type` | the project's name and kind when the project is created |
| `Issue Type` | issue type; unknown types are added to the project (a sub-task type when the name says "sub-task") |
| `Status`, `Status Category` | status; unknown statuses are added to the workflow (see guessing rules) |
| `Priority` | priority (see mapping) |
| `Assignee` + `Assignee Id`, `Reporter` + `Reporter Id` | users (see people) |
| `Created`, `Updated`, `Resolved` | dates (see dates) |
| `Description` | rich text, converted from Jira wiki markup |
| `Labels` (repeated) | labels (split at spaces, de-duplicated, at most 100) |
| `Sprint` (repeated) | sprints, by name (see sprints) |
| `Parent` / `Parent id` / `Parent key`, else `Custom field (Epic Link)` | the parent, by key or issue id |
| `Custom field (Story Points)` / `Custom field (Story point estimate)` | story points |
| `Comment` (repeated) | comments, each `date;author;body` (the body may contain `;`) |
| `Inward issue link (Type)` / `Outward issue link (Type)` | links (see links) |
| `Component/s` (repeated) | components, created by name in the project (M2.4) |
| `Fix Version/s`, `Affects Version/s` (repeated) | fix and affects versions, created by name (unreleased) |
| `Custom field (Name)` (repeated for several values) | a custom field `Name` of the project (an existing one keeps its type; see custom fields) |

**Custom fields** (M2.4): the type of a new field is guessed from every value in the file —
all numbers → number; all dates → date; several values on one issue (a repeated column) →
multi-select over the values seen (at most 500); otherwise text. `Rank`, `Sprint`, `Epic …` and
story-point columns are not custom fields here. A name that is a system field's (`Status`) is a
warning and the column is skipped. Values are set as the issue's first values (no history);
one that doesn't fit its field is a warning.

Not imported yet: due dates, time tracking, watchers, votes, attachments, version release
dates and states, resolution names (only resolved/unresolved),
issue security, change history (the import creates a `created` entry; comments and links add
their own).

## Jira JSON (REST API v3)

Accepted shapes: one `search/jql` page (`{"issues": […]}`), an array of pages, an array of issue
objects, or a single issue. Per issue: `key`, `id`, `fields.summary`, `fields.issuetype.name` /
`.subtask`, `fields.project.name` / `.projectTypeKey`, `fields.status.name` and
`.statusCategory.key` (`new` / `indeterminate` / `done`), `fields.priority.name`,
`fields.assignee` / `fields.reporter` (`accountId`, `displayName`, `emailAddress`),
`fields.created` / `updated` / `resolutiondate`, `fields.description` (ADF; a v2 string is read
as wiki markup), `fields.labels`, `fields.parent.key`, `fields.comment.comments[]` (`author`,
`created`, `body`), `fields.issuelinks[]` (`type.name` with `outwardIssue` / `inwardIssue`),
`fields.components[].name`, `fields.fixVersions[].name`, `fields.versions[].name` (M2.4).

Custom fields: when the page has `names` (`expand=names`), the fields named "Story Points" /
"Story point estimate" and "Sprint" are read. Without `names`, the first `customfield_*` holding
sprint objects (`name`, `state`, dates) is the sprint field; story points are skipped.

## Rules and guesses

**Status category** for a status the project doesn't have: the export's category when it gives
one (CSV `Status Category`, JSON `statusCategory.key`). Otherwise from the name:
Done if it contains *done, closed, resolved, complete, fixed, released, shipped, cancel, won't,
wont, declined, rejected, duplicate*; In Progress if it contains *progress, review, doing,
develop, test, qa, verif, implement, active, started, working, blocked*; otherwise To Do — except
that an issue with a `Resolved` date makes an unrecognised status Done. Added statuses get a
global transition (reachable from any status).

**Priority:** our five names match directly (Highest, High, Medium, Low, Lowest). Blocker /
Critical / Urgent → Highest; Major → High; Normal → Medium; Minor → Low; Trivial → Lowest;
anything else → Medium, with one warning per name.

**People:** a person matches an existing user by email, then display name (case-insensitive),
then by id = account id. Otherwise a user is created with a stable id: the email's local part,
else the display name lowercased with spaces as dots (`Ann Lee` → `ann.lee`), else
`acct-<account id>`; a clash with a different user adds `-2`, `-3`…. A comment author given
only as an account id takes the name seen with that id elsewhere in the export (`Assignee Id`,
`Reporter Id`). Missing reporters and comment authors fall back to the importing user.

**Dates:** RFC 3339 / ISO 8601 (`2024-03-21T15:07:00.000+0000`), the CSV default
`21/Mar/24 3:07 PM`, `2024-03-21 15:07`, `2024-03-21`. Dates without a zone are read as UTC.
An unreadable date is a warning; the import time is used instead.

**Parents and sub-tasks:** parents are created before their children. A parent that isn't in
the export or the workspace is dropped with a warning; a sub-task left without a parent is
imported as a Task. Cycles are broken (the first issue reached loses its parent).

**Links:** `Blocks` → blocks, `Relates` → relates to, `Duplicate` → duplicates, `Cloners` →
clones; other types → relates to (one warning per type). Outward on A naming B means A → B;
inward means B → A. Links to issues outside the workspace are dropped with a warning; a link
already present (either direction, for relates) is not duplicated.

**Sprints:** created per project by name (an existing sprint of the same name is reused).
JSON gives each sprint's state and dates. CSV gives names only, so the state is guessed: a
sprint that isn't the last one listed on some issue was carried over from, so it is **closed**;
so is a sprint whose issues are all in a Done-category status. Of the remaining sprints, the one
with the lowest trailing number (then first seen) is **active** — unless the project already has
an active sprint — and the rest are **future**. An active sprint without dates starts at the
import time and ends 14 days later. Each issue goes in its last open sprint, with closed ones in
its past sprints; sub-tasks follow their parent's open sprint.

**Re-importing** the same export skips every key that already exists (with a warning); nothing
is overwritten.

## Jira wiki markup

Descriptions and comments in CSV (and v2 JSON strings) are converted:
`h1.`–`h6.` headings; `*` / `#` / `-` lists, nested by repeating the marker (`**`, `#*`);
`{code[:lang]}`…`{code}` and `{noformat}`…`{noformat}` code blocks; `{quote}`…`{quote}` and
`bq.` quotes; `{panel}`, `{info}`, `{note}`, `{tip}`, `{warning}` as quotes; `----` rules;
`||head||` / `|cell|` tables; `*strong*`, `_emphasis_`, `??citation??`, `-strike-`,
`+underline+`, `{{monospace}}`; `[text|url]` and `[url]` links (http, https, mailto, `#` and
`/` only — other schemes keep just their text); `[~user]` / `[~accountid:…]` mentions; `\\`
line breaks and `\` escapes. A single line break inside a paragraph is a hard break.
`^sup^` / `~sub~` and `{color}` keep their text; `!image.png!` becomes `[image: image.png]`.

## Confluence

**HTML export** (a folder or a .zip; the export may sit inside one top folder): the folder of the
shallowest `index.html` is the root, and every `.html` file next to it is a page.

- **Space:** the name from `index.html`'s title; the key from `--space`, else the "Key" row of
  the overview's details table, else derived from the name (initials of a multi-word name, else
  its first letters, up to 10), else `IMPORT`. A missing space is created; an existing one is
  added to.
- **Hierarchy and order:** the nested page list in `index.html` (each list item linking to a
  page file is that page; its parent is the enclosing such item). Pages it doesn't list are
  placed by their breadcrumbs (`#breadcrumbs`: the last link to another page file is the parent).
  When exactly one page is at the top, it becomes the space's homepage (if the homepage is still
  blank); otherwise top-level pages go under the homepage. A parent that can't be imported, or a
  tree deeper than 64 levels, puts the page under the homepage, with a warning.
- **Page:** the title from `#title-text` (the "Space name : " prefix removed), else `<title>`,
  else the file name; the body from `#main-content` (else `.wiki-content`, else `<body>`); the
  author and date from the `.page-metadata` line ("Created by …, last modified by … on Mar 21,
  2024": the last modifier, or the creator, and that date). Titles are unique in a space: a
  clash becomes "Title (2)".

**Storage format** (one page's XHTML, `--confluence page.xml --space KEY`; also any non-HTML
file holding `<ac:`/`<ri:` elements): one page under the space's homepage, titled after the file.

**HTML → rich text:** `h1`–`h6`, `p`, `ul`/`ol`/`li`, `pre` (the export's `brush:` language
kept), `blockquote`, `hr`, `br`, `strong`/`b`, `em`/`i`, `code`/`tt`, `s`/`del`, `u`, `a href`
(http, https, mailto only — links to other export files keep their text). `div`, `span` and
other containers are see-through. Tables become ADF `table` nodes, kept as unknown blocks until
tables are modelled (they round-trip through ADF but don't render yet). `<ac:structured-macro>`:
`code` / `noformat` become code blocks; every other macro becomes an unknown block — an ADF
`extension` (or `inlineExtension` inside a paragraph) with its parameters and the original
storage XHTML verbatim under `storage`. `ac:link` keeps its link text (or the target page's
title); images become `[image: name]`; emoticons are dropped.

Not imported yet: attachments and images (counted in a warning), page labels, comments,
restrictions, page history (each page imports as version 1), blog posts, and the XML export.
