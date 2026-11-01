# API and import compatibility

> **Status:** 2026-10-31 · M4.1: the REST subset below is served by `trackcraft-server`
> ([`server.md`](server.md)); imports are planned (M5).

Interoperability is a feature: teams move from Jira/Confluence by import, and their scripts,
CI integrations and agents keep working if TrackCraft answers the same REST calls. Everything
below comes from Atlassian's **public** API references and export documentation (clean room:
`AGENTS.md`).

## REST (served by `trackcraft-server` through `crates/api`, mapped onto engine commands)

Every route is also served under `/w/{workspace}/…`. Auth: Bearer token, or Basic
`email:token` as Jira clients send it. ✅ = implemented and tested
(`crates/server/src/tests.rs`); ◐ = implemented with the noted differences; — = not yet.

### Jira Cloud REST v3

| Public endpoint | Status | TrackCraft command | Notes |
| --- | --- | --- | --- |
| `GET /rest/api/3/myself` | ✅ | — (reads the workspace) | `accountId` is the user id. |
| `GET /rest/api/3/project` | ✅ | `project.list` | Array form. `id` is derived from the key (see `crates/api/src/ids.rs`). |
| `GET /rest/api/3/project/{idOrKey}` | ✅ | `project.get` | Adds `lead`, `issueTypes`, `statuses`. |
| `GET /rest/api/3/project/{idOrKey}/components` | ✅ | `component.list` | `id`, `name`, `description`, `lead`, `projectId` (M2.4; `crates/api/src/tests_fields.rs`). |
| `GET /rest/api/3/project/{idOrKey}/versions` | ✅ | `version.list` | `id`, `name`, `description`, `startDate`, `releaseDate`, `released`, `archived`, `projectId`. |
| `GET /rest/api/3/field` | ◐ | `field.list` | System fields, then custom fields of the projects the caller can see: `id`/`key` `customfield_<id>`, `clauseNames` (`cf[<id>]`, name), `schema.type` (`string`, `number`, `date`, `option`, `array` of `option`, `user`), `scope.project`. No plugin type keys. |
| `GET/POST /rest/api/3/search/jql` | ✅ | `search.jql` | `jql`, `nextPageToken` (and `startAt`), `maxResults` (cap 100), `fields` (`*all`, `*navigable`, `-x`; default ids only). Bad JQL → 400 with the parser message. |
| `POST /rest/api/3/issue` | ✅ | `issue.create` | `fields`: project, summary, issuetype, description (ADF or text), priority, assignee, labels, parent, `storyPoints`, `sprint`, `components` / `fixVersions` / `versions` (`[{"name"}]` or `[{"id"}]`), and `customfield_<id>` of the project's fields (select `{"value"}`, multi-select `[{"value"}]`, user `{"accountId"}`, number, text, date `YYYY-MM-DD`). Other standard fields and unknown `customfield_*` are accepted and ignored; unknown fields → 400. |
| `GET /rest/api/3/issue/{idOrKey}` | ✅ | `issue.get` | `fields` selection. Story points are the non-standard `storyPoints` field; `sprint`/`closedSprints` use the Agile sprint shape. `components`, `fixVersions`, `versions` and `customfield_<id>` (null when unset) in the public shapes. |
| `PUT /rest/api/3/issue/{idOrKey}` | ✅ | `issue.edit`, `issue.assign`, `issue.comment` | `fields` (custom fields, components and versions as in POST; null clears) plus `update` verbs for labels (add/remove/set), summary/description (set), comment (add). `returnIssue=true`. All-or-nothing. |
| `DELETE /rest/api/3/issue/{idOrKey}` | ✅ | `issue.delete` | Refused for issues with sub-tasks unless `deleteSubtasks=true`. |
| `GET/POST /rest/api/3/issue/{key}/transitions` | ✅ | `issue.transitions` / `issue.transition` | Transition ids are 1-based positions in the project workflow; `update.comment` add is applied. |
| `GET/POST /rest/api/3/issue/{key}/comment` | ✅ | `issue.comment` | `startAt`, `maxResults`, `orderBy=-created`. |
| `GET/PUT/DELETE /rest/api/3/issue/{key}/comment/{id}` | ✅ | `issue.editComment` / `issue.deleteComment` | Only a comment's author may edit or delete it. |
| `PUT /rest/api/3/issue/{key}/assignee` | ✅ | `issue.assign` | `accountId` null unassigns; `"-1"` = the project lead. |
| `POST /rest/api/3/issueLink` | ✅ | `issue.link` | `inwardIssue` gets the type's outward text towards `outwardIssue` (Blocks, Relates, Duplicate, Cloners). Link ids are not exposed. |
| `GET /rest/api/3/issue/{key}/changelog` | ✅ | `issue.history` | Oldest first; `from`/`to` ids are null (values are display strings). |
| `GET /rest/api/3/search` (legacy), users, fields, filters, worklogs, attachments, watchers, votes | — | | |

### Jira Software Agile 1.0

| Public endpoint | Status | TrackCraft command | Notes |
| --- | --- | --- | --- |
| `GET /rest/agile/1.0/board` | ✅ | `project.list` | One board per project (scrum for software, kanban for business); board id = project id. `projectKeyOrId`, `type`, `name`. |
| `GET /rest/agile/1.0/board/{id}` | ✅ | | |
| `GET /rest/agile/1.0/board/{id}/sprint` | ✅ | `sprint.list` | `state` filter; kanban boards → 400. |
| `GET /rest/agile/1.0/board/{id}/backlog` | ✅ | `board.backlog` | `jql`, `fields`, paging. |
| `GET /rest/agile/1.0/board/{id}/issue` | ✅ | `board.view` | |
| `GET /rest/agile/1.0/sprint/{id}`, `/sprint/{id}/issue` | ✅ | — (reads the workspace) | `jql` filter, rank order. |
| Sprint create/update/move issues, rank, epics | — | (`sprint.*`, `issue.rank` exist; reachable via `/api/command`) | |

### Confluence Cloud REST v2

| Public endpoint | Status | TrackCraft command | Notes |
| --- | --- | --- | --- |
| `GET /wiki/api/v2/spaces`, `/spaces/{id}` | ✅ | `space.list` | `ids`, `keys`, `sort`, cursor paging (`_links.next` + `Link` header). Space id derived from the key. |
| `GET /wiki/api/v2/spaces/{id}/pages` | ✅ | — (reads the workspace) | Same filters as `GET /pages`. |
| `GET/POST /wiki/api/v2/pages` | ◐ | `page.create` | Filters `id`, `space-id`, `title`, `status`; `sort`. Only `status=current` (no drafts). |
| `GET/PUT/DELETE /wiki/api/v2/pages/{id}` | ◐ | `page.get` / `page.edit` / `page.delete` | PUT needs `version.number` = current + 1 (else 409). `?version=N` reads an old version. DELETE removes the page **and its subtree** (no trash yet). |
| `GET /wiki/api/v2/pages/{id}/versions` | ✅ | `page.history` | Newest first; `sort=modified-date` for oldest first. |
| `GET /wiki/api/v2/pages/{id}/children` | ✅ | — (reads the tree) | |
| `GET /wiki/api/v2/pages/{id}/footer-comments`, `POST /wiki/api/v2/footer-comments` | ✅ | `page.comments` / `page.comment` | Top-level footer comments; `parentCommentId` replies. |
| Blog posts, attachments, labels, inline comments, content properties, CQL search | — | | |

**Body formats (◐).** `body-format=atlas_doc_format` returns ADF (as a JSON string in `value`,
like the public API) and is the lossless format. TrackCraft does not store storage-format XHTML:
`body-format=storage` returns the page's **Markdown** in `value`. On input, a storage value with
tags is converted to plain paragraphs (tags dropped, entities decoded — lossy); one without tags
is read as Markdown, so GET-then-PUT keeps formatting. `view`, `wiki` and other formats → 400.

### TrackCraft native

| Endpoint | Status | Notes |
| --- | --- | --- |
| `POST /api/command {"command", "params"}` | ✅ | Any engine command (as MCP/CLI), except server-blocked `file.*`, `workspace.new`, `edit.undo/redo`, `user.switch`. |
| `GET /api/commands` | ✅ | `app.commands`, blocked ones marked. |
| `GET /api/workspace`, `GET /api/health` | ✅ | Health needs no auth. |

Errors use each surface's public error shape; see [`server.md`](server.md#limits-and-errors).

## Imports (M5)

- **Jira:** the documented CSV export columns, and JSON from the REST API (`search/jql` pages).
- **Confluence:** the space export (HTML and XML), with the storage format converted to our
  ADF-shaped model; wiki markup for older content.
- Importers are hostile-input parsers with fuzz tests; fixtures are generated in code, never
  real exports.
