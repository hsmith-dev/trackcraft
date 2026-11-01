# API and import compatibility

> **Status:** 2026-10-31 · planned (M5). Nothing here is implemented yet.

Interoperability is a feature: teams move from Jira/Confluence by import, and their scripts,
CI integrations and agents keep working if TrackCraft answers the same REST calls. Everything
below comes from Atlassian's **public** API references and export documentation (clean room:
`AGENTS.md`).

## REST (served by `trackcraft-server` through `crates/api`, mapped onto engine commands)

| Public endpoint (subset) | TrackCraft command |
| --- | --- |
| `GET/POST /rest/api/3/search/jql` | `search.jql` |
| `POST /rest/api/3/issue` | `issue.create` |
| `GET/PUT/DELETE /rest/api/3/issue/{key}` | `issue.get` / `issue.edit` / `issue.delete` |
| `GET/POST /rest/api/3/issue/{key}/transitions` | `issue.transitions` / `issue.transition` |
| `POST /rest/api/3/issue/{key}/comment` | `issue.comment` |
| `PUT /rest/api/3/issue/{key}/assignee` | `issue.assign` |
| `POST /rest/api/3/issueLink` | `issue.link` |
| `GET /rest/api/3/project`, `/project/{key}` | `project.list` / `project.get` |
| `GET /rest/agile/1.0/board/{id}/issue` | `board.view` |
| `GET/POST /wiki/api/v2/pages`, `GET/PUT/DELETE /wiki/api/v2/pages/{id}` | `page.*` |
| `GET /wiki/api/v2/spaces`, `/spaces/{id}/pages` | `space.list` / `space.tree` |
| `GET /wiki/api/v2/pages/{id}/versions` | `page.history` |

Command outputs already use the public field names where they exist, so the REST layer is a
mostly mechanical reshaping.

## Imports (M5)

- **Jira:** the documented CSV export columns, and JSON from the REST API (`search/jql` pages).
- **Confluence:** the space export (HTML and XML), with the storage format converted to our
  ADF-shaped model; wiki markup for older content.
- Importers are hostile-input parsers with fuzz tests; fixtures are generated in code, never
  real exports.
