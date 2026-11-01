# TrackCraft

**An open-source, Rust-native issue tracker and team wiki in one app.** Projects, issues,
workflows, boards and JQL search live next to spaces of pages with full version history, and the
two link to each other. TrackCraft aims for parity with Jira and Confluence, then goes further:
fast, works offline from a single file, self-hostable, and every action is a command that agents
can drive (CLI and MCP today, REST soon).

> **Stage: pre-alpha (M0 foundation).** The engine, JQL and storage work headless; the desktop UI
> is next. See [`ROADMAP.md`](ROADMAP.md).

TrackCraft is an independent clean-room project, not affiliated with Atlassian. Jira and
Confluence are Atlassian trademarks, named here only to describe compatibility.

## Try it

```sh
# every command, with its parameters
cargo run -p trackcraft-cli -- commands

# search the sample workspace with JQL
cargo run -p trackcraft-cli -- jql --sample 'project = LUME AND statusCategory != Done ORDER BY priority'

# run commands and save the result
cargo run -p trackcraft-cli -- run --sample \
  --cmd 'issue.create={"project":"LUME","summary":"Try TrackCraft","type":"Story"}' \
  --cmd 'issue.transition={"key":"LUME-8","to":"In Progress"}' \
  --cmd 'board.view={"project":"LUME"}' \
  --out team.track

# let an agent drive it over MCP (stdio); changes autosave to team.track
cargo run -p trackcraft-cli -- mcp --in team.track
```

## What works today

- **Tracker:** projects, issue types (Epic, Story, Task, Bug, Sub-task), workflows with status
  categories, transitions, resolution dates, priorities, labels, story points, assignees,
  comments, issue links, sub-tasks, boards by status.
- **JQL:** `AND`/`OR`/`NOT`, `= != > >= < <= ~ !~ IN NOT IN IS [NOT] EMPTY`, `ORDER BY`,
  `currentUser()`, `now()`, `startOfDay()`, relative dates (`-7d`), `text ~`.
- **Wiki:** spaces with home pages, page trees, move/delete subtrees, labels, full version
  history with restore, conflict detection on save, page search.
- **Rich text** in the Atlassian Document Format JSON shape, with Markdown output.
- **Platform:** undo/redo for every change, atomic `.track` saves, MCP server, CLI.

## Building

Rust 1.95+. `cargo xtask ci` runs every quality gate (fmt, clippy, tests, asset attribution,
crate layering, wasm, parity checklist).

## Contributing

Read [`AGENTS.md`](AGENTS.md) first: the clean-room rules, the asset policy, and the
never-crash rules apply to every change. Pick work from [`docs/gaps.md`](docs/gaps.md).

## Licence

MIT OR Apache-2.0, at your option ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)).
