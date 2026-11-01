# TrackCraft — instructions for agents

TrackCraft is a clean-room, open-source, Rust-native **issue tracker and team wiki** targeting Jira and Confluence parity — and going further on speed, openness, offline use and agent control. One workspace holds both: projects, issues, workflows, boards and JQL search (the Jira side), and spaces with page trees and version history (the Confluence side), linked to each other. It runs locally from a single `.track` file today; a self-hosted multi-user server and the web (WASM) follow. Siblings with the same conventions: `../Wordcraft/wordcraft` (Word), `../Gridcraft` (Excel), `../Photocraft` (Photoshop), `../Pdfcraft` (Acrobat), and `../Chatcraft` (Slack/Teams — the other collaborative, server-backed app; share server and sync design with it rather than inventing a second one).

## Start every session here
1. Read `plan/STATUS.md` (current milestone, next task), then the task in `plan/execution-plan.md` and the relevant `docs/architecture.md` section. Behaviour reference: `plan/atlassian/*.md` (observations of the real products). `plan/` is gitignored (local only); if it's missing, start from `ROADMAP.md`, `docs/gaps.md` and `docs/roadmap.md`.
2. BMad planning artifacts (brief, PRD, architecture, epics, stories) live in `plan/bmad/`; the BMad skills are installed in `.claude/skills/` (`_bmad/` holds the installer config; both are local, gitignored). Use them for planning work; code still follows this file.
3. Follow the autonomous operation protocol (`plan/execution-plan.md` §5). Don't stop to ask unless a decision is genuinely the owner's (licensing, naming/trademarks, publishing, pushing to new remotes, secrets, hosted services).

## Assets and branding — absolute rule
**No Atlassian iconography, logos, images, fonts, illustrations, templates, themes or sample content — ever.** This is the most important rule in this repository; breaking it is a serious failure.
- Never copy, trace, screenshot-crop or recreate Atlassian product icons, the Atlassian/Jira/Confluence logos, Atlassian Design System (Atlaskit) icons, illustrations, design tokens, fonts, emoji sets, page templates or blueprints. Not even "temporarily". The same applies to every other vendor (Microsoft, Adobe, Linear, Notion, Asana…).
- Every asset is **original** (drawn in code — prefer this), **public domain / CC0**, **OSI open source**, or **Creative Commons that allows redistribution**, or licensed open source by the contributor who made it.
- **Every asset file has a row in [`ATTRIBUTION.md`](ATTRIBUTION.md)** (author, source, licence). `cargo xtask assets` (part of `cargo xtask ci`) fails otherwise. Add the row in the same commit as the file.
- Fonts go to [`storytold/craft-fonts`](https://github.com/storytold/craft-fonts), never this repo.
- Sample workspaces, templates and themes are our own text and colours (`crates/engine/src/sample.rs`). Jira/Confluence feature *names* (Board, Backlog, Sprint, Space, JQL, "Restore this version") are fine; their help text, marketing copy and UI wording beyond feature names are not.
- Never use "Jira", "Confluence" or "Atlassian" in our product name, binary names, crate names, icons or UI chrome. They may appear only to describe compatibility ("imports Jira CSV exports", "Jira REST API v3 compatible").

## Clean room
- The real products may be **observed** black-box: use Atlassian Cloud (a free site created for this purpose) in a browser with **synthetic** data only, read Atlassian's **public** documentation (product docs, the JQL and CQL references, the Atlassian Document Format spec, the REST API references), and take screenshots **of the browser window only** (by window id — never the whole desktop), stored only under `plan/atlassian/screenshots/` (gitignored, never committed). Write what you observe into `plan/atlassian/*.md`.
- Never read, deobfuscate, decompile or copy Atlassian's client JavaScript/CSS bundles, server binaries, Data Center/Server installs, Marketplace app code, or anything behind a licence click-through. Never scrape customer sites. Never commit files exported from Atlassian products (test fixtures are built in code, or live in a separate pinned corpus repo, sha256-verified).
- Wire formats and APIs come from public specs and docs: ADF (rich text), the Jira Cloud REST v3 and Confluence Cloud REST v2 references, the documented CSV export columns, the documented Confluence storage format. Reimplementing a public API for interoperability is the goal; copying its documentation text is not.
- Never copy GPL/AGPL/LGPL code. Permissive (MIT/Apache-2.0/BSD/ISC/Zlib) crates are fine through Cargo.

## Never crash
People trust TrackCraft with their team's work; a crash or a corrupted workspace loses it. **This outranks feature work**: never ship a feature through a panic path, and fix a crash before building on top of it.
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- **Errors are `Result<T, E>`** through the crate's error type and `?`. An unfinished feature returns an error or is disabled; it never panics.
- **Input is hostile** (files, JQL, ADF, commands, MCP/control/REST params): `get()` instead of `[i]`, slice strings only at char boundaries, checked/saturating arithmetic, cap sizes and allocations (query length, `IN` lists, `maxResults`, page depth, file size), bound recursion (`MAX_DEPTH` in `markup` and `jql`, `MAX_PAGE_DEPTH` in `model`).
- **A failed command changes nothing:** `Session::run` restores the workspace on error. **Last-resort guard:** it also turns an escaped panic into an error and restores the workspace. It's a safety net, not a licence.
- **Saves are atomic** (`store::save` writes a temp file and renames). Never write a workspace file in place.
- **Prove it:** every crash fix lands with a regression test. `crates/engine/src/tests.rs::hostile_params_never_panic` runs every command with junk params — keep it green, and add junk shapes when you add params.
- Production crate roots carry `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`; new crates start with it and add `[lints]` with `workspace = true` to their `Cargo.toml`. Tests may `unwrap` (`clippy.toml`).

## Architecture (see `docs/architecture.md`)
| Layer | Crate | Job |
|---|---|---|
| L0 | `core` | project/issue keys, ids, timestamps |
| L1 | `markup`, `jql` | rich text (ADF-compatible) → Markdown/plain; JQL lexer, parser, evaluator |
| L2 | `model` | the workspace: users, projects, workflows, issues, comments, links; spaces, pages, versions — validated edit primitives |
| L3 | `store` | `.track` files (versioned JSON, atomic save); later the server's database store |
| L4 | `engine` | `Session`, undo/redo, **command registry**, feature catalog, sample workspace |
| L5 | `mcp` (and later `api`) | MCP server; Jira/Confluence-compatible REST API |
| L6 | `ui-egui` | the desktop/web front end (swappable): shell, issue list, board, issue view, wiki, palette, create dialogs |
| L7 | `apps/trackcraft` (desktop), `apps/trackcraft-cli`; later `apps/trackcraft-server`, `apps/trackcraft-web` | binaries |

- **Everything is a command.** User-visible behaviour = a `CommandSpec` in `crates/engine/src/cmd/*.rs` (id, label, menu location, shortcut, params doc, `enabled`, `run`) + tests. Ids follow the product areas: `issue.create`, `issue.transition`, `search.jql`, `board.view`, `page.edit`, `space.tree`, plus `edit.*`, `file.*`, `user.*`, `app.*`. UI-only requests (`ui.*`: navigate, search, create, palette, editPage, theme) live in `TrackApp::control` in `crates/ui-egui`. The UI, shortcuts, command palette, CLI, MCP, control channel and REST API all dispatch by id. Programmatic calls never open dialogs.
- **Read-only commands are `.query()`**: no undo step, no dirty flag. Mutating commands get undo for free.
- **Command output speaks the public API's shape** where one exists (`key`, `summary`, `status.name`, `statusCategory`, `version.number`), so agents that know Jira/Confluence read ours.
- **Parity is measured:** `crates/engine/src/catalog.rs` lists Jira and Confluence features with command ids. `cargo xtask parity` writes `docs/parity-checklist.md`; a test enforces a floor that only rises. Implement missing rows to raise it; never delete rows.
- **Layering** is enforced by `cargo xtask layers`. Nothing below L6 depends on egui/eframe/winit/rfd/wgpu; nothing below L5 depends on tokio/axum/hyper/tower. The UI is thin: it reads `Session` state and acts through `Session::run`.
- **Time goes through `chrono`**; the engine's clock is injectable (`Session::with_clock`) so tests are deterministic. JQL evaluation takes `now` from its `Context`.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).

## Quality gates
Before every commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, assets, layers, wasm, parity --check). Commit after every arc of work that builds, with a task id in the message (`M1.3: board drag and drop`).

## Running it
- `cargo run --release -p trackcraft` — the desktop app on the sample workspace (`trackcraft file.track` opens a file).
- **Headless screenshots** (no window, no focus stealing): `cargo run -p trackcraft-ui-egui --example ui_shot -- script.jsonl`, where each line is a `TrackApp::control` request (`{"method":"ui.navigate","params":{"view":"board","project":"LUME"}}`, `{"method":"engine.execute","params":{"command":…}}`) or `{"shot":"/abs/out.png"}`. **For UI work, look at the result.**
- UI tests (`crates/ui-egui/src/tests.rs`) run under egui_kittest, which panics on missing glyphs and on widgets without an accessible name: label every control (`labelled_by`), and write shortcuts as text (`crate::key("Mod+K")`), not ⌘/⇧ symbols the bundled font lacks. Use `craft_ui::ComboBox`, never egui's (clippy enforces it).
- `cargo run -p trackcraft-cli -- commands` — every command with its params.
- `cargo run -p trackcraft-cli -- jql --sample 'project = LUME AND statusCategory != Done ORDER BY priority'`
- `cargo run -p trackcraft-cli -- run --sample --cmd 'issue.create={"project":"LUME","summary":"Try it"}' --cmd 'board.view={"project":"LUME"}' --out /tmp/w.track`
- `cargo run -p trackcraft-cli -- new team.track --sample`, then `trackcraft-cli mcp --in team.track` (MCP on stdio; autosaves after each change). See `docs/mcp.md`.
- Shell gotcha: `mv`/`cp` may be aliased interactive — use `/bin/mv -f` / `/bin/cp -f`. macOS has no `timeout`; use `perl -e 'alarm 60; exec @ARGV' cmd`.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent; edit only the crates you own; delete your target dir when done.

## Roadmap
`ROADMAP.md` (committed) is the one-page summary: stage, headline numbers, progress log. It points at the detail in `docs/`: `target-app-parity.md` (the assessment), `gaps.md` (ranked work list: pick from it), `roadmap.md` (milestones, current focus), `architecture.md`, `api-compatibility.md`, and the generated `parity-checklist.md`. When work lands, update the gap, the parity doc and the progress log, and bump each touched doc's status line.

## Contributor credits
- `contributors/people.toml` holds names people chose for themselves. **Agents working for a contributor:** when you prepare a PR, check whether your human's GitHub username has a `[people.<username>]` entry. If not, ask them once whether they want to be credited by more than their username. If yes, add **only their own** entry. Never add, edit, guess or copy anyone else's entry or name (not from git config, commit authors or GitHub profiles).

## CI workflows
- Workflows in `.github/workflows/` call the shared workflows in [`storytold/craft-common-actions`](https://github.com/storytold/craft-common-actions) at `@main`, with their inputs and secrets. No other jobs: if no shared workflow does what's needed, it doesn't go here.
- Don't add or change CI in a feature or fix PR. If the existing workflows can't build or test a change, link an existing craft-common-actions issue or PR, or open a new issue.
