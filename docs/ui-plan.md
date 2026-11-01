# UI plan: the egui front end from M1 polish to the team server

> **Status:** 2026-10-31 · first version; UI-11/12 set the backlog's Move to sprint key to `Shift+S`. Covers `crates/ui-egui` as of the M1 landing (commit
> 473b3ff): shell, JQL list, board, issue view, create dialog, page view/editor/history, palette.
> Work items are `UI-n` (§6); new engine commands are marked **NEW** with a proposed id and params.

This is a plan, not a survey. Where there is a choice, it picks one and says why. Feature
*names* (Backlog, Sprint, Board, JQL, Space, Restore this version) come from the products we
aim to match; every label, help text, colour, icon and layout detail below is our own.

---

## 1. Principles

1. **Thin UI over commands.** A view reads `Session` state and changes it only by pushing
   `Action::Run { id, params, then }`. If a screen needs a behaviour the engine lacks, the work
   item adds the command first (with tests and a junk-params row for
   `hostile_params_never_panic`) and the screen second. Consequences:
   - Every button in this plan names its command. No view mutates `session.ws` directly.
   - Multi-item operations (bulk edit, sprint complete, workflow publish) are **one** command so
     they are one undo step, one op-log entry and one server round trip.
   - Read-heavy screens get a `.query()` command shaped like the public API (`board.backlog`,
     `issue.history`, `page.diff`) so the CLI, MCP and the future web client draw the same data.
     Views may still read `session.ws` directly for cheap lookups (names, avatars).
   - Machine-local preferences (theme, density, column widths, sidebar width, recent files) are
     *not* workspace data: they are `ui.*` control methods backed by `craft-settings`. Anything
     another user or an agent should see is a command.
2. **Keyboard-first.** Every screen is fully usable without a mouse; every drag has a keyboard
   equivalent (board: `Shift+Arrow` moves a card; backlog: `Mod+Shift+Up/Down` ranks; tree:
   "Move…" picker). Single-letter shortcuts come from the `CommandSpec` registry, dispatched by
   scope (§4) through `craft-keymap`, never hard-coded per view. `?` shows the map.
3. **Both themes, always.** All colour comes from `theme::Tokens`. Each new token gets a light and
   a dark value in the same commit. `every_view_renders_in_both_themes` grows a row for every new
   `View`. State is never colour-only: diff inserts are underlined, deletes struck through; WIP
   limit breaches get an icon and text, not just a red column.
4. **Accessibility is a test, not a review.** egui_kittest panics on unlabelled widgets and
   missing glyphs — keep it that way. Every `TextEdit`, `ComboBox`, `DragValue`, toggle and icon
   button gets `labelled_by` or an `on_hover_text` + accessible name. Shortcuts are written as
   text via `crate::key("Mod+K")`. Focus order follows reading order; modals trap focus and
   return it to the opener on close. Toasts are announced (accesskit live region).
5. **Hostile-data resilience.** Design every screen for: a 2 000-character unbreakable summary,
   10 000 issues in one project, 300 users, a 24-deep page tree with 500 siblings, a status
   that is not in the workflow, an assignee id with no user, an empty workspace, and RTL/CJK
   text. Rules: truncate by **width** not chars (`Label::truncate()` + hover for full text);
   lists and boards **virtualise** (draw only visible rows); tree walks are bounded
   (`MAX_PAGE_DEPTH`) and lazily expanded; nothing in a draw path is O(issues × issues) (today
   the inward-links scan on the issue view is — fix in UI-9). A stress fixture
   `trackcraft_engine::sample::stress(now, StressSpec { issues: 10_000, pages: 2_000, depth: 24 })`
   (**NEW** test helper, not a command) drives the large-data tests.
6. **Our own visual identity.** Teal accent (`#0E7A6B` light / `#3CC2A8` dark), amber for
   in-progress, green for done, warm-grey neutrals (`theme.rs`). Icons are drawn in code in
   `icons.rs` (issue types, priorities, avatars, chevrons, page) — no SVG files, so no
   `ATTRIBUTION.md` rows are needed; if a file asset is ever added it gets its row in the same
   commit. Shapes: 6 px widget radius, 10 px cards, uppercase 11 px status pills. Density: a
   "Compact" preference tightens row height from 32 to 26 px; nothing else changes.
7. **Never lose work.** Unsaved text (page editor, description, comment drafts) survives
   navigation, back, palette jumps and undo; leaving asks. This outranks feature work (UI-1).

---

## 2. Information architecture

### 2.1 The frame

```
┌ top bar ────────────────────────────────────────────────────────────────────────────────┐
│ [mark] Workspace ▾  File  │ [ search: JQL or text … (/) ]  [+ Create ▾] │ ? [bell] [avatar] │
├ sidebar (resizable, Mod+\ hides) ┬ central view ──────────────────────────────┬ peek ────┤
│ [filter sidebar…]               │ breadcrumbs                                │ (optional│
│ Home                            │ heading + view actions                     │  right   │
│ Your work                       │ ───────────────────────────────────────    │  pane:   │
│ STARRED                         │ content                                    │  issue   │
│   ★ My open issues (filter)     │                                            │  or page │
│   ★ Lume (project)              │                                            │  preview)│
│ PROJECTS            All ›       │                                            │          │
│ ▾ Lume · LUME                   │                                            │          │
│     Backlog                     │                                            │          │
│     Board                       │                                            │          │
│     Issues                      │                                            │          │
│     Releases                    │                                            │          │
│     Reports                     │                                            │          │
│     Project settings            │                                            │          │
│ ▸ Ops · OPS                     │                                            │          │
│ SPACES              All ›       │                                            │          │
│ ▾ Engineering  [+]              │                                            │          │
│     ▾ Handbook                  │                                            │          │
│        Onboarding               │                                            │          │
│ ▸ Product                       │                                            │          │
├─────────────────────────────────┴────────────────────────────────────────────┴──────────┤
│ status bar: Saved 12:04 · ~/team.track · 3 selected · (M4: Online · 0 queued)            │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

Decisions:

- **Top bar** keeps one search field. Text that parses as JQL runs `search.jql`; otherwise it
  runs a unified search (issues by `text ~`, pages by `page.search`) and shows a combined
  results view with two sections. A small "JQL" badge in the field says which mode it is in.
  `+ Create` is a split button: the main part creates in the current half (issue on tracker
  views, page on wiki views), the ▾ offers the other. The "Acting as" combo moves into the
  avatar menu (it is a test/dev affordance until M4 sign-in replaces it); Undo/Redo/Theme
  buttons move to the avatar menu and Edit menu — the top bar is for navigation, not toggles.
- **Sidebar** is contextual and bounded. At most 5 starred + 5 recent projects and spaces are
  listed; "All ›" opens the Projects / Spaces directory screens (searchable tables). Only the
  *current* project expands to its sub-items; only the current space shows its page tree
  (others collapsed to one row). A filter box at the top narrows every section (type-ahead over
  project names, filter names and page titles). Starring is per user and stored in the
  workspace (**NEW** `user.favourite`, §3.1) so it syncs in M4.
- **Back stack.** `back` exists; add `forward`. Each `View` gets a remembered `ViewState`
  (scroll offset, selected row, open tab) keyed by `View`, restored on Back/Forward. Bindings:
  `Alt+Left/Right`, `Mod+[ / Mod+]`, mouse buttons 4/5. Cap 100 (as now).
- **Palette** (`Mod+K`, also `Mod+Shift+P`) stays the universal jump: issues, pages, boards,
  backlogs, filters, settings tabs and commands. A command that has a UI form (create issue,
  link issue, move page, start sprint…) opens that form instead of running with context
  params (generalise today's `issue.create` special case into a `ui_route(id) -> Option<Action>`
  table). Commands still lacking params show the params doc inline, not an error toast.
- **Peek pane** (right, `craft_ui::panes::PaneHost`): `Space` on a selected list row or board
  card opens the issue in a compact detail pane without leaving the list; `Enter` opens it full.
  Same `issue_detail` widget, `compact: true`.

### 2.2 Where things live

| Thing | Location | View |
|---|---|---|
| Backlog + sprint planning | Project › Backlog | `View::Backlog(ProjectKey)` |
| Active sprint board | Project › Board (shows active sprint if the project has one, else all) | `View::Board { project, sprint: Option<Id> }` |
| Issue search | top bar / Project › Issues / any filter | `View::Issues` (JQL in `app.jql`) |
| Saved filters | sidebar Starred; Filters directory | `View::Filters`, a filter opens `View::Issues` with its JQL and a "filter" header |
| Releases (versions), components | Project › Releases; Project settings › Components | `View::Releases(p)`, `View::Version(Id)` |
| Reports (burndown, velocity) | Project › Reports | `View::Reports { project, kind }` (late M2) |
| Dashboard | Home (Home *is* the default dashboard; more dashboards later) | `View::Home` |
| Project settings (details, issue types, workflow, board columns, fields, components) | Project › Project settings | `View::ProjectSettings { project, tab }` |
| Space settings (name, home, templates, labels) | space ⋯ menu | `View::SpaceSettings { space, tab }` |
| Preferences (theme, density, language, shortcuts, autosave) | avatar menu › Settings, `Mod+,` | `View::Settings(Tab::Preferences)` |
| Workspace admin (name, users, custom fields, priorities) | Settings › Workspace | `View::Settings(Tab::Workspace/Users/Fields)` |
| Notifications (M4) | bell | `View::Notifications` (also a popover) |
| Page diff | page History › Compare | `View::PageDiff { id, from, to }` |

`ui.navigate` gains a view name per new variant (`backlog`, `filters`, `releases`, `settings`,
`projectSettings`, `diff`, `notifications`, `projects`, `spaces`), parsed in one place
(`route.rs`, UI-2) so parallel agents add one arm each.

---

## 3. Screen-by-screen specs

Conventions in this section:
- **Cmds**: existing commands in `code`; **NEW** ones with proposed id and params.
- **Tests**: kittest test names in `crates/ui-egui/src/tests.rs` (or `tests/<area>.rs` after
  UI-2); **Shots**: lines for a `ui_shot` script (absolute paths; one script per work item,
  kept in `crates/ui-egui/examples/shots/<item>.jsonl`).
- Large-data state = the `sample::stress` fixture.

### 3.0 Gaps in what exists today (found reading the code)

| # | Where | Gap | Fix in |
|---|---|---|---|
| G1 | `lib.rs::navigate` / `Action::Back` | Leaving a page in the editor drops `page_edit` (Back drops it unconditionally): **unsaved text is lost**. Same for an open description edit (`IssueUi::default()`). | UI-1 |
| G2 | `lib.rs::logic` | `Mod+Shift+Z` runs `edit.redo` even while typing in a text field (undo is guarded, redo is not). | UI-4 |
| G3 | CommandSpecs | `E`, `A`, `I`, `M` (issue) and `L` (page) are declared shortcuts but never dispatched; only `C`, `/` and page `E` are wired. | UI-4 |
| G4 | CommandSpecs | `C` is both `issue.create` and `page.create`; `E` is both `issue.edit` and `page.edit`. Fine only if dispatch is scoped. | UI-4 |
| G5 | `issues::list` | `Grid` draws every result row each frame; truncation is by 70 chars; headers are not sortable though M1.2 says sortable; no paging though `search.jql` returns `total`; result is capped at 1000 silently. | UI-3 |
| G6 | `issues::fields` | Story points `DragValue` runs `issue.edit` on every typed digit (one undo step per keystroke). Labels are a space-separated string. | UI-9 |
| G7 | `issues::main_column` | Inward links scan every issue each frame; no way to add/remove links, edit a comment, change type or parent. | UI-9 |
| G8 | `issues::board` | Cards sorted by priority then number (no rank); issues whose status is not in the workflow vanish; no keyboard; per-column card list not virtualised. | UI-10, UI-11 |
| G9 | `shell::sidebar` | Lists every project and space with full trees: does not scale; filters hard-coded. | UI-8 |
| G10 | toast | One toast at a time, same styling for errors and success, no Undo action. | UI-5 |
| G11 | `wiki::page` | Delete page has no confirmation and no child count; it relies on undo with a toast. | UI-5 (Undo in toast), UI-6 |
| G12 | palette | Commands run with context params and report "Done: …" or a raw error; UI forms are not reused. | UI-8 |
| G13 | top bar | Theme is a two-state toggle (no "System" after first click). | UI-7 |

### 3.1 M1 polish

#### Keyboard navigation (all lists, board, tree) — UI-4, UI-10
- **Purpose:** every screen operable from the keyboard; registry-driven shortcuts.
- **Design:** `keys.rs` builds a `craft_keymap::Keymap` from `trackcraft_engine::commands()`
  at start-up. Scope comes from a **NEW** `CommandSpec::scope` field (`Global`, `IssueList`,
  `Board`, `Issue`, `Page`, `Editor`), defaulted from `location` (`Issue*` → Issue,
  `Page*` → Page). The active scope set is derived from `app.view` + focus (a focused
  `TextEdit` disables single-key chords). Chords `G then X` (two-key sequences, 1 s window) are
  UI-only navigation. User overrides come from `craft-settings` and are editable in Settings
  › Shortcuts (`craft_keymap::editor`).
- **Selection model:** list, backlog and board share `Selection { focused: Option<IssueKey>,
  selected: BTreeSet<IssueKey>, anchor }` in `ViewState`.
- **Cmds:** existing ones by shortcut; **NEW** shortcut `T` on `issue.transition` (opens the
  status picker).
- **States:** shortcut on a disabled command → toast with `enabled`'s reason ("nothing to undo").
- **Tests:** `shortcuts_dispatch_by_scope` (C on board opens create-issue, C on a page opens
  new-page), `single_keys_ignored_while_typing`, `redo_ignored_while_typing` (G2),
  `issue_view_letter_shortcuts` (A, I, M, L, E, T each do their thing on LUME-2),
  `help_overlay_lists_every_registry_shortcut`, `g_chords_navigate`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"issue","key":"LUME-2"}}`,
  `{"method":"ui.key","params":{"chord":"?"}}`, `{"shot":"/tmp/tc-shots/shortcuts.png"}`
  (**NEW** control method `ui.key {"chord": "Shift+ArrowRight"}` synthesises a key press, so
  scripts can drive keyboard flows).

#### Issue list — UI-3
- **Purpose:** the JQL result table; the workhorse for triage and bulk work.
- **Layout:**
```
Issues · My open issues ★              [Save filter ▾] [Columns ▾] [Export ▾]
[ project = LUME AND statusCategory != Done ORDER BY priority  ] [Basic|JQL] [Search]
 ⚠ line 1, col 17: expected a value after "="              (inline error banner)
 142 issues · 1–50                                                    ‹ 1 2 3 ›
┌─┬──┬────────┬─────────────────────────────────┬──────────┬────────┬─────────┬────────┐
│☐│T │ Key ▲  │ Summary                         │ Status   │ Prio   │Assignee │Updated │
├─┼──┼────────┼─────────────────────────────────┼──────────┼────────┼─────────┼────────┤
│☐│▣ │LUME-12 │ Sync crashes when the network…  │IN PROGR. │ ▲ High │ (AL) Ann│ 2 h ago│
```
- **Interactions:** `egui_extras::TableBuilder` (already in the suite fork's patch list; add it
  to `ui-egui` deps) with `body.rows()` virtualisation and resizable columns. Header click
  cycles asc → desc → none by rewriting the query's `ORDER BY` through **NEW**
  `search.orderBy {"jql", "field", "direction": "ASC"|"DESC"|null} → {"jql"}` (query; the JQL
  stays the single source of truth, so a sorted list can be saved as a filter). Column set
  from a chooser (Key and Summary fixed; Type, Status, Priority, Assignee, Reporter, Updated,
  Created, Resolved, Labels, Story points, Sprint, Fix version, Due, custom fields later),
  stored per machine (`ui.listColumns`). Paging 50/100/200 per page via `startAt`/`maxResults`,
  "Showing first 1000" note when `total` > cap. `J/K`/arrows move focus, `Enter`/`O` open,
  `Space` peek, `X` toggle select, `Shift+X`/`Shift+Click` range, `Mod+A` all on page,
  `A`/`I`/`E` act on the focused row.
- **Cmds:** `search.jql`, **NEW** `search.orderBy`.
- **States:** empty result → empty state ("No issues match", buttons: clear query, create
  issue); JQL error → inline banner under the field with the parser's position (the current
  red box, plus caret marker); 10k → virtualised, header sort on 10k must stay under one frame
  (the sort is in the engine, the table only draws ~25 rows).
- **Tests:** `list_header_click_rewrites_order_by`, `list_draws_only_visible_rows_with_10k`
  (count accessible rows < 60), `list_paging_moves_start_at`, `list_long_summary_truncates_by_width`,
  `list_keyboard_open_and_peek`, `list_empty_state_offers_create`.
- **Shots:** `{"method":"ui.search","params":{"jql":"project = LUME ORDER BY key DESC"}}`,
  `{"shot":"/tmp/tc-shots/list.png"}`, `{"method":"ui.theme","params":{"dark":true}}`,
  `{"shot":"/tmp/tc-shots/list-dark.png"}`.

#### Empty, loading and error states — UI-6
- One widget, `widgets::empty_state(ui, t, icon, title, body, actions: &[(&str, Action)])`,
  used by: Home with an empty workspace, empty list, empty board column ("Drop issues here"),
  empty backlog, page with empty body, empty history, no search results, no notifications.
- **Loading:** local commands are synchronous today; design for M4 now with
  `Loadable<T> { Ready(T), Pending { since }, Failed(String) }` in view caches (list, backlog,
  history). Pending > 150 ms shows skeleton rows (grey bars, no spinner); never blocks input.
- **Errors:** view-local errors render inline (banner with "Try again"); command failures are
  error toasts (UI-5); modal dialogs only for data-loss decisions.
- **Hostile data:** board adds a trailing "Other statuses" column for statuses not in the
  workflow; unknown user ids render as a grey avatar with the raw id; deleted parent keys
  render as plain struck text, not links.
- **Tests:** `empty_workspace_renders_every_view`, `orphan_status_gets_other_column`,
  `unknown_assignee_renders_raw_id`, `deep_tree_is_bounded` (24-deep + cycle fixture).
- **Shots:** `{"method":"engine.execute","params":{"command":"workspace.new","params":{"name":"Empty"}}}`
  is a query today — the shot script uses the stress/empty fixtures via a **NEW** ui_shot flag
  `{"fixture":"empty"|"stress"}` on the first line, then `{"shot":"/tmp/tc-shots/empty-home.png"}`.

#### Toasts and status bar — UI-5
- **Toasts:** a stack (max 3, newest at bottom centre), kinds `Info`, `Success`, `Error`.
  `Then::Toast` gains `undoable: bool`; after a mutating command, the toast carries an
  **Undo** button that runs `edit.undo` (only while the undo top is still that command — the
  session exposes `undo_depth()`; a stale toast hides its button). Success fades after 4 s,
  errors stay until dismissed and offer "Copy details". Hover pauses the timer.
- **Status bar** (22 px, bottom): save state (`Saved 12:04` / `Unsaved changes` / `Autosaved`),
  file path (middle-truncated), selection count, and in M4 connection + queued-op count.
- **Cmds:** `edit.undo`, `file.save`.
- **Tests:** `toast_undo_reverts_transition`, `stale_toast_hides_undo`, `error_toast_persists`,
  `status_bar_shows_unsaved_after_edit`.
- **Shots:** drag-free: `{"method":"engine.execute","params":{"command":"issue.transition","params":{"key":"LUME-2","to":"Done"}}}`
  then `{"method":"ui.toast","params":{"text":"LUME-2 → Done","undo":true}}` (**NEW** control
  method for shots), `{"shot":"/tmp/tc-shots/toast.png"}`.

#### Settings — UI-7
- **Layout:** left tab list, right form. Tabs: **Preferences** (Theme: System/Light/Dark; Density:
  Comfortable/Compact; Language (craft-i18n, English only until catalogs land); Autosave: off /
  every change / every minute; Open last workspace at start), **Shortcuts** (craft-keymap
  editor: search, record chord, conflicts shown inline, reset), **Workspace** (name, users),
  later **Fields**, **Priorities**, **Issue types** (M2).
- **Cmds:** Preferences are `ui.settings.get/set` (machine-local, craft-settings). Workspace:
  **NEW** `workspace.rename {"name"}`, `user.add`, **NEW** `user.edit {"id", "name", "email"?,
  "active"?}` (no user delete: deactivate, so history keeps its authors).
- **States:** a settings file that fails to load uses defaults (craft-settings never fails) and
  shows a one-time info toast.
- **Tests:** `settings_theme_system_follows_os`, `settings_rename_workspace_updates_title`,
  `settings_shortcut_conflict_is_flagged`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"settings","tab":"preferences"}}`,
  `{"shot":"/tmp/tc-shots/settings.png"}`.

#### Sidebar, directories, palette routing — UI-8
- Bounded sidebar per §2.1; **Projects** and **Spaces** directory screens (name, key, lead /
  page count, starred toggle, updated) using `project.list` / `space.list`.
- **Cmds:** **NEW** `user.favourite {"kind": "project"|"space"|"filter"|"page", "id": "LUME"|12,
  "on": true}` (mutating, per acting user). Recents are machine-local (`ui.*`).
- **Tests:** `sidebar_shows_at_most_five_recent_projects` (with 40 projects),
  `sidebar_filter_narrows_tree`, `palette_routes_create_to_dialog`, `palette_shows_params_doc_for_incomplete_command`,
  `back_forward_restore_scroll_and_selection`.
- **Shots:** stress fixture, `{"method":"ui.navigate","params":{"view":"projects"}}`,
  `{"shot":"/tmp/tc-shots/projects.png"}`.

#### Issue view polish — UI-9
- **Layout:**
```
Lume / LUME-1 / ▣ LUME-4                                   (AL)(RS) viewing  ← M4
Sync crashes when the network drops                         (click or E to edit)
[In Progress ▾]  [Assign to me]  [(eye) Watch 2]  [Link]  [Attach]  [⋯]
┌ main ───────────────────────────────────────┐ ┌ Details ──────────────────┐
│ Description                          [Edit] │ │ Assignee   (AL) Ann ▾     │
│ …                                           │ │ Reporter   (RS) Riley     │
│ Sub-tasks                     [+ Add] 2/3 ▮▮▯│ │ Priority   ▲ High ▾       │
│ Linked issues                      [+ Link] │ │ Labels     [sync] [+]     │
│   blocks  LUME-9  Offline queue     TO DO ✕ │ │ Sprint     Lume 4 ▾       │
│ Pages                                [+]    │ │ Fix version 1.2 ▾         │
│ Attachments                       [+]       │ │ Story pts  [ 3 ]          │
│ ─ Comments | History | All ─                │ │ Due        2026-11-14 ▾   │
│ (avatar) [Add a comment… (M)]               │ │ Created / Updated / Res.  │
└─────────────────────────────────────────────┘ └───────────────────────────┘
```
  Note: "(eye)" above stands for an icon drawn in code, not a glyph — kittest bans
  missing glyphs; all icons come from `icons.rs`.
- **Interactions:** inline link form (type combo + issue picker with key/summary search,
  Enter adds); `✕` on a link row removes it; comment edit (own comments) inline with
  `Mod+Enter` save / `Esc` cancel; type change and parent change in `⋯`; story points via
  `craft_ui::localized::NumberField` committing on Enter/blur only (G6); labels via the label
  editor widget (chips + autocomplete from existing labels); inward links read from a
  per-frame cached reverse index built once per workspace change (G7).
- **Cmds:** `issue.edit` (**NEW** params `type`, `parent` (key or null)), `issue.link`,
  **NEW** `issue.unlink {"key", "type", "target"}`, **NEW** `issue.editComment {"key", "id",
  "body"}`, `issue.transition`, `issue.assign`, `issue.assignToMe`, `issue.comment`,
  `issue.deleteComment`, `issue.delete`.
- **Tests:** `issue_link_add_and_remove`, `story_points_one_undo_step_per_commit`,
  `label_editor_adds_and_removes_chips`, `comment_edit_own_only`, `issue_view_with_2000_char_summary`,
  `inward_links_cached_not_rescanned` (10k fixture frame-time guard).
- **Shots:** `{"method":"ui.navigate","params":{"view":"issue","key":"LUME-4"}}`,
  `{"shot":"/tmp/tc-shots/issue.png"}`; `{"method":"ui.peek","params":{"key":"LUME-4"}}` on the
  list (**NEW** control method), `{"shot":"/tmp/tc-shots/peek.png"}`.

#### Create dialog polish (part of UI-9)
- Add Labels, Story points, Due date, Sprint (M2), Components (M2); parent becomes an issue
  picker; `Mod+Enter` creates; "Create another" checkbox keeps the dialog open with project,
  type and sprint kept; the form remembers the last project/type per machine. Draft survives
  `Esc` for the session (re-open restores) — Esc should not destroy a typed description.
- **Tests:** `create_dialog_mod_enter_submits`, `create_another_keeps_project`,
  `create_dialog_escape_keeps_draft`.

#### Board polish — UI-10
- Keyboard: arrows move focus between cards and columns, `Enter` open, `Space` peek,
  `Shift+Left/Right` transition to the adjacent column (runs `issue.transition`; if no
  transition leads there, the toast says so with the workflow's reason), `Mod+Shift+Up/Down`
  rank within the column once `issue.rank` exists (UI-11). `F` focuses the filter field.
- Column cards virtualised (`ScrollArea::show_rows` with fixed card height estimate; cards
  with long summaries clamp to 3 lines + ellipsis). Column header shows count and, after M2
  board config, WIP limit (`5 / 4` with a warning icon when over).
- **Cmds:** `board.view`, `issue.transition`.
- **Tests:** `board_keyboard_move_card_right`, `board_drop_on_same_column_is_noop`,
  `board_10k_cards_virtualised`, `board_card_summary_clamped`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"board","project":"LUME"}}`,
  `{"method":"ui.key","params":{"chord":"ArrowDown"}}`, `{"shot":"/tmp/tc-shots/board-focus.png"}`.

### 3.2 M2 tracker

#### Backlog with drag rank — UI-11
- **Purpose:** plan sprints: order the backlog, pull work into sprints, start the next sprint.
- **Layout:**
```
Lume backlog                       [filter…] [Only mine] [Epics ▸]      [+ Create sprint]
▾ Lume 4 · ACTIVE · Nov 3 – Nov 14 · 18 pts · 7 issues                 [Complete sprint]
   ⠿ ▣ LUME-12  Sync crashes when…     IN PROGRESS  ▲  3  (AL)
   ⠿ ◆ LUME-15  Login copy             TO DO        ●  1  (RS)
▾ Lume 5 · planned · 0 pts                         [Start sprint] [⋯ edit / delete]
   (empty) Drag issues here, or select one and press Shift+S to move it.
▾ Backlog · 42 issues · 96 pts                                     [+ Create issue]
   ⠿ ◆ LUME-20  …
   + Create issue (inline row: type ▾ + summary, Enter)
```
- **Interactions:** drag handle `⠿` (drawn) reorders within a section and moves between
  sprints/backlog; multi-select drag moves the whole selection keeping relative order; drop
  indicator is a 2 px accent line between rows. Keyboard: `Mod+Shift+Up/Down` rank one step,
  `Mod+Shift+Home/End` top/bottom of section, `Shift+S` "Move to sprint…" picker (`Mod+Shift+S`
  is Save As). Inline
  create row at the bottom of each section. Sections collapse; sprint headers show points and
  issue counts per status category as a small three-segment bar (grey/amber/green).
- **Cmds (all NEW):**
  - `board.backlog {"project", "jql"?}` → `{"sprints":[{"id","name","state":"future"|"active"|"closed","goal","startDate","endDate","issues":[…]}], "backlog":[…]}` (query; ordered by rank).
  - `issue.rank {"keys":[…], "before": "LUME-3" | "after": "LUME-3"}` (one undo step; ranks are
    a LexoRank-style string per issue in the model, so moves are O(moved)).
  - `sprint.create {"project", "name"?, "goal"?, "startDate"?, "endDate"?}`,
    `sprint.edit {"id", …same}`, `sprint.delete {"id"}` (issues return to the backlog).
  - `issue.setSprint {"keys":[…], "sprint": 7 | null, "before"?|"after"?}` (move + rank in one step).
- **States:** project without sprints (kanban kind) → backlog has only the Backlog section and
  a "Create sprint" button; 10k backlog virtualised per section; drag across a collapsed
  section expands it after 600 ms hover.
- **Tests:** `backlog_drag_ranks_issue`, `backlog_multi_drag_keeps_order`,
  `backlog_keyboard_rank_matches_drag`, `backlog_move_to_sprint_picker`,
  `backlog_inline_create_lands_in_section`, `backlog_10k_virtualised`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"backlog","project":"LUME"}}`,
  `{"shot":"/tmp/tc-shots/backlog.png"}`.

#### Sprints: start / complete, sprint board — UI-12
- **Start dialog:** name, duration preset (1/2/3/4 weeks, custom), start/end date pickers, goal.
  **NEW** `sprint.start {"id", "startDate", "endDate", "goal"?}` (refuses if another sprint in
  the project is active — one active sprint per project until parallel sprints are wanted).
- **Complete dialog:** "N issues done, M open"; open issues go to: next planned sprint ▾ / new
  sprint / backlog. **NEW** `sprint.complete {"id", "moveOpenTo": 8 | "new" | "backlog"}`.
- **Sprint board:** `View::Board { project, sprint }`; header shows sprint name, dates, days
  left, goal; `board.view` gains **NEW** param `"sprint": "active" | 7`. Swimlanes (later in
  M2) by assignee / epic / none: **NEW** `board.configure {"project", "columns":[{"name",
  "statuses":[…], "wipLimit"?}], "swimlanes": "none"|"assignee"|"epic"}`.
- **Tests:** `sprint_start_requires_dates`, `second_active_sprint_refused_with_message`,
  `sprint_complete_moves_open_issues`, `board_shows_active_sprint_only`, `wip_limit_warning_has_text`.
- **Shots:** `{"method":"engine.execute","params":{"command":"sprint.start","params":{"id":1,"startDate":"2026-11-03","endDate":"2026-11-14"}}}`,
  `{"method":"ui.navigate","params":{"view":"board","project":"LUME","sprint":"active"}}`,
  `{"shot":"/tmp/tc-shots/sprint-board.png"}`.

#### Activity / history tab — UI-13
- Tabs under the issue main column: **Comments** (default), **History**, **All** (merged,
  newest first, comment-only toggle remembered). History rows: avatar, "changed Status",
  `To Do → In Progress`, relative time with exact time on hover. Field values render with the
  same widgets as the fields panel (pills, avatars) so changes read at a glance.
- **Cmds:** **NEW** `issue.history {"key", "startAt"?, "maxResults"?}` →
  `{"values":[{"id","author","created","items":[{"field","from","fromString","to","toString"}]}]}`
  (query; the public changelog shape). Depends on the op log (gaps #3).
- **States:** issue created before the op log → one "History starts here" row.
- **Tests:** `history_tab_lists_transition`, `all_tab_merges_in_time_order`, `history_paginates`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"issue","key":"LUME-2","tab":"history"}}`,
  `{"shot":"/tmp/tc-shots/history.png"}`.

#### Saved filters — UI-14
- "Save filter ▾" on the list: Save as… (name, description, star), Save (if the list came from
  a filter you own and the JQL changed — header shows "Edited" with Revert), Rename, Delete.
  Filters directory: name, owner, starred, JQL preview; click runs.
- **Cmds (NEW):** `filter.create {"name", "jql", "description"?}` → `{"id"}`,
  `filter.edit {"id", "name"?, "jql"?, "description"?}`, `filter.delete {"id"}`,
  `filter.list {"owner"?}` (query), `filter.get {"id"}` (query); starring via `user.favourite`.
  The four built-in sidebar filters become "system filters" returned by `filter.list` with
  negative/reserved ids, not hard-coded in `shell.rs`.
- **Tests:** `save_filter_then_star_shows_in_sidebar`, `edited_filter_shows_revert`,
  `delete_filter_from_directory_toast_undo`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"filters"}}`, `{"shot":"/tmp/tc-shots/filters.png"}`.

#### Basic search builder over JQL — UI-15
- **Layout:** a row of field chips above the list: `Project: Lume ▾  Type: All ▾  Status: 2 ▾
  Assignee: Me ▾  [text contains…]  + More ▾` — each chip a multi-select popover with search.
  `Basic | JQL` switch. Basic → JQL is always possible; JQL → Basic only when the query is a
  conjunction of `IN`/`=`/`~` clauses on supported fields (otherwise the switch is disabled
  with a tooltip giving the reason).
- **JQL mode** gets autocomplete: fields after start/`AND`, operators after a field, values after
  an operator (users, statuses, labels, versions), functions; `Tab` accepts. Errors underline
  the bad token.
- **Cmds (NEW, all queries):** `search.fields {"project"?}` → fields with operators and value
  kinds; `search.suggest {"jql", "cursor"}` → `{"replaceFrom","replaceTo","items":[{"text","label","kind"}]}`;
  `search.toBasic {"jql"}` → `{"clauses":[{"field","op","values"}],"text"?}` or error with
  reason; `search.fromBasic {"clauses", "text"?, "orderBy"?}` → `{"jql"}`.
- **Tests:** `basic_builder_round_trips_simple_query`, `complex_query_disables_basic_with_reason`,
  `jql_autocomplete_suggests_statuses`, `autocomplete_tab_accepts`.
- **Shots:** `{"method":"ui.search","params":{"jql":"project = LUME AND assignee = currentUser()"}}`,
  `{"method":"ui.searchMode","params":{"mode":"basic"}}` (**NEW**), `{"shot":"/tmp/tc-shots/basic-search.png"}`.

#### Versions and components — UI-16
- **Releases screen:** table of versions (name, status Unreleased/Released/Archived, start,
  release date, progress bar by status category, description) with `+ Create version`;
  version detail lists its issues (via `search.jql` `fixVersion = "1.2"`) and a **Release**
  button with "move N open issues to ▾" choice.
- **Components:** Project settings › Components table (name, lead, default assignee, issue count).
- Issue fields panel and create dialog gain Fix versions / Affects versions / Components
  multi-pickers.
- **Cmds (NEW):** `version.create {"project","name","description"?,"startDate"?,"releaseDate"?}`,
  `version.edit {"id",…}`, `version.release {"id","moveOpenTo"?: id}`, `version.archive {"id"}`,
  `version.delete {"id","moveTo"?: id}`, `version.list {"project"}` (query);
  `component.create {"project","name","lead"?,"defaultAssignee"?}`, `component.edit`,
  `component.delete {"id","moveTo"?}`, `component.list {"project"}` (query);
  `issue.edit` **NEW** params `fixVersions`, `affectsVersions`, `components`, `dueDate`.
- **Tests:** `release_version_moves_open_issues`, `component_default_assignee_applies_on_create`,
  `version_progress_bar_has_text_alternative`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"releases","project":"LUME"}}`, `{"shot":"/tmp/tc-shots/releases.png"}`.

#### Custom fields — UI-17
- **Admin:** Settings › Fields: list (name, type, projects, used by N issues), create dialog
  (name, type: text, paragraph, number, date, single select, multi select, user, labels, URL;
  options editor for selects with drag order; which projects; shown on create yes/no).
- **Issue:** custom fields render in the Details card below built-ins, in the admin's order,
  each with the matching widget (field row component); empty fields are hidden behind
  "Show N more fields".
- **Cmds (NEW):** `field.create {"name","type","options"?,"projects"?}` → `{"id":"customfield_10001"}`,
  `field.edit {"id",…}`, `field.delete {"id"}` (refuses while values exist unless `"force": true`),
  `field.list` (query); `issue.edit` accepts `{"fields": {"customfield_10001": value}}`.
  JQL `cf[10001]` support is engine work alongside.
- **Tests:** `custom_select_field_edits_issue`, `custom_fields_hidden_when_empty`,
  `field_delete_in_use_warns`.

#### Workflow editor — UI-18
- **Layout:** Project settings › Workflow. Left: statuses list (name, category pill, issue
  count) with add/rename/reorder/remove; right: transitions as a from→to grid (rows = from,
  columns = to, cells toggle; a row "Any status" for global transitions) — chosen over a node
  graph because it is keyboard-operable, accessible and readable at 20 statuses. Edits are a
  local **draft**; **Publish** validates and asks how to map issues in removed statuses.
  Board columns tab (same screen): columns ↔ statuses mapping with drag, WIP limits.
- **Cmds (NEW):** `workflow.get {"project"}` (query) → `{"statuses":[…],"transitions":[…]}`;
  `workflow.publish {"project","statuses":[{"name","category"}],"transitions":[{"name","from":[…],"to"}],"statusMap":{"Old":"New"}}`
  (one validated replace, one undo step); `board.configure` (UI-12).
- **States:** removing a status with issues and no mapping → Publish disabled, row flagged with
  text; a workflow with 0 Done statuses → warning (resolution never sets).
- **Tests:** `workflow_draft_publish_maps_issues`, `workflow_grid_keyboard_toggle`,
  `workflow_without_done_warns`, `board_columns_follow_published_workflow`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"projectSettings","project":"LUME","tab":"workflow"}}`, `{"shot":"/tmp/tc-shots/workflow.png"}`.

#### Bulk edit — UI-19
- Selecting ≥ 2 rows in the list or backlog shows a **selection bar** at the bottom of the
  view: `12 selected · Transition ▾ · Assign ▾ · Priority ▾ · Labels ▾ · Sprint ▾ · More ▾ · Clear`.
  "More" opens a dialog with every editable field (set / add / remove / clear per field) and a
  preview count. Result toast: "Updated 12 issues · Undo"; partial failures are listed per key
  (the command is all-or-nothing by default; `"skipInvalid": true` applies the valid ones).
- **Cmds (NEW):** `issue.bulkEdit {"keys":[…≤1000], "set":{…}, "add":{"labels":[…]}, "remove":{…}, "skipInvalid"?}`,
  `issue.bulkTransition {"keys", "to", "skipInvalid"?}`, `issue.bulkDelete {"keys"}`.
- **Tests:** `bulk_transition_one_undo_step`, `bulk_edit_add_label_keeps_existing`,
  `bulk_invalid_transition_lists_keys`, `selection_bar_counts_cross_pages`.
- **Shots:** `{"method":"ui.select","params":{"keys":["LUME-1","LUME-2","LUME-3"]}}` (**NEW**),
  `{"shot":"/tmp/tc-shots/bulk.png"}`.

#### Issue links, watchers, votes, clone/move, due date, export — UI-20, UI-22
- Links: covered in UI-9 plus **Pages** section (UI-23).
- Watchers: header button with count; popover lists watchers, "Watch/Stop watching", "Add
  watcher" (user picker). `W` toggles. **NEW** `issue.watch {"key","user"?}`,
  `issue.unwatch {"key","user"?}`, `issue.watchers {"key"}` (query). Creating, commenting and
  being assigned auto-watch (engine rule).
- Clone (`⋯ › Clone`): dialog with summary prefill and "include sub-tasks / links" checkboxes.
  **NEW** `issue.clone {"key","summary"?,"subtasks"?,"links"?}`. Move: **NEW**
  `issue.move {"key","project","type"?,"statusMap"?}` (new key; old key resolves as an alias).
- Due date: date picker in Details; overdue shown with an icon + "Overdue" text.
- Export: list "Export ▾" → CSV (current columns / all fields). **NEW**
  `search.exportCsv {"jql","fields"?:[…]}` → `{"csv"}` (query; the host writes the file).
- **Tests:** `watch_toggle_with_w`, `clone_copies_subtasks_when_asked`, `move_issue_keeps_alias`,
  `export_csv_uses_visible_columns`.

#### Attachments — UI-21
- Issue: "Attachments" strip (thumbnail cards for images drawn from decoded bytes, file cards
  with type badge for others), `+` opens the host file dialog, drag files from the OS onto the
  issue view or a comment box (egui `dropped_files`), paste image from clipboard. Image
  click opens a lightbox (Esc closes, arrows step). Wiki pages share the widget (M3).
- **Cmds (NEW):** `attachment.add {"target":{"issue":"LUME-1"}|{"page":12}, "name", "mediaType", "path"|"data"(base64)}`
  (size cap from the store; refuses executables only by policy setting), `attachment.delete {"id"}`,
  `attachment.list {"target"}` (query), `attachment.get {"id"}` (query, bytes).
  `.track` keeps blobs in a sidecar or a bounded inline section — engine decision; UI only
  needs the commands.
- **Tests:** `attach_via_dropped_file`, `oversize_attachment_error_toast`, `image_lightbox_keyboard`.

#### Issue ↔ page links — UI-23
- Issue view "Pages" section lists pages that mention the issue key (computed) plus explicit
  links; `+` page picker. Page view shows a "Linked issues" footer card. Issue keys in any
  rendered rich text become chips with status pill (inline card) — a markup rendering change,
  no command.
- **Cmds (NEW):** `issue.linkPage {"key","page"}`, `issue.unlinkPage {"key","page"}`,
  `issue.pages {"key"}` (query: explicit + mentioned).

#### Dashboard / Home and reports — UI-24 (late M2)
- Home becomes a two-column dashboard of fixed gadgets for now: Assigned to me, Recently
  viewed (issues + pages, machine-local), Active sprints (progress bar + days left), Starred
  filters (top 5 rows each), Recently updated pages. User-built dashboards wait for M4.
- Reports: burndown (remaining points by day, ideal line) and velocity (committed vs done per
  sprint) drawn with the egui painter in our palette, each with a data table toggle for
  accessibility. **NEW** `report.burndown {"sprint"}`, `report.velocity {"project","sprints"?:6}`
  (queries over the op log).

### 3.3 M3 wiki

#### Rich editor plan — UI-25 (now), UI-31 (later)
**Phase A — Markdown source + live preview (extend what exists).** Decision: keep the 50/50
split (with a "Source only / Split / Preview" toggle) through M3's first half. It is honest,
fast, accessible and round-trips through `Doc::from_markdown`. Add:
- a toolbar (Heading ▾, Bold, Italic, Code, Link, Bullet, Numbered, Task, Quote, Code block,
  Table, Panel ▾, Macro ▾) that inserts/wraps Markdown at the caret; `Mod+B/I/K/E`
  (`Mod+E` = inline code), `Mod+Shift+7/8` lists, `Mod+Enter` publish, `Esc` asks before cancel;
- syntax for new blocks, parsed by `trackcraft-markup` into ADF nodes: GFM tables, task lists
  `- [ ]`, panels `:::info … :::` (kinds: info, note, success, warning, error — our own
  colours and drawn icons), macros as `:::toc`, `:::children`, `:::issues jql="…"`
  (ADF `extension` nodes), issue keys auto-linked;
- paste a URL over a selection → link; paste an image → attachment (UI-21) + image block;
- preview scroll-sync to the source caret line.
Markup work (not commands): `Block::{Table, TaskList, Panel, Extension, Media}` with ADF in/out
and Markdown in/out, bounded by `MAX_DEPTH`.

**Phase B — WYSIWYG block editor (UI-31, late M3).** A single-pane editor over the ADF tree:
each block is an editable unit (paragraph/heading rendered with marks through a `LayoutJob`;
tables as grids of cells; panels as framed blocks), a `/` menu inserts blocks, `⠿` handles
drag blocks, `Mod+Shift+Up/Down` moves a block. The editing model (`EditOp { path, kind }`:
insert text, delete range, toggle mark, split, join, set attrs, move block) lives in
`trackcraft-markup` (pure, L1, property-tested, no egui) so the same ops later feed the op
log and M4 live collaboration. Do **not** port Wordcraft's page-layout composer — wiki pages
are reflowing blocks, not paginated documents; reuse `craft_ui::ime` for CJK composition and
`craft-text` bidi for RTL. Markdown source stays available as a toggle forever.

#### Tables, panels, task lists, macros — UI-26
- Rendering in `markup_view` (shared with issue descriptions): tables with header row
  shading and horizontal scroll when wider than the column; panels with a 3 px left stripe in
  the kind's colour, drawn icon and soft fill; task items as clickable checkboxes.
- Ticking a task in **view mode** is a page change: **NEW** `page.toggleTask {"id","taskId","done","baseVersion"}`
  (creates a version with message "Updated a task"; conflicts as for `page.edit`).
- Macros render through a `MacroHost` the UI passes to `markup_view::show`: TOC from the
  doc's headings (links scroll to anchors), Children from `ws.children(id)` (bounded),
  Issue list runs `search.jql` (capped at 50 rows, "Open in search" link) with columns
  key/summary/status/assignee. Unknown extensions render as a grey "Unsupported block" card
  that keeps the node (round-trip preserved).
- **Tests:** `table_renders_and_scrolls`, `panel_has_text_kind_label`, `task_toggle_creates_version`,
  `toc_macro_links_headings`, `issue_list_macro_bad_jql_shows_inline_error`, `unknown_extension_preserved`.
- **Shots:** a sample page with all blocks (added to `sample.rs`, our own text):
  `{"method":"ui.navigate","params":{"view":"page","id":14}}`, `{"shot":"/tmp/tc-shots/blocks.png"}`.

#### Page and inline comments — UI-27
- **Page comments** below the content: threaded one level (reply), edit/delete own, resolve not
  applicable. **Inline comments:** select text in view mode → floating "Comment" button →
  thread in a right margin column aligned to the highlight; highlights use `accent_soft`
  with an underline; Resolve hides the highlight; "Show resolved" toggle. If the anchored text
  is edited away, the thread moves to a "Detached comments" list at the top of the margin.
- **Cmds (NEW):** `page.comment {"id","body","parent"?, "anchor"?: {"blockPath":[3,0],"start":12,"end":30,"quote":"…"}}`,
  `page.editComment {"id","comment","body"}`, `page.deleteComment {"id","comment"}`,
  `page.resolveComment {"id","comment","resolved":true}`, `page.comments {"id","inline"?}` (query).
- **Tests:** `inline_comment_anchors_selection`, `resolved_hides_highlight`,
  `detached_comment_after_text_removed`, `page_comment_reply_threads`.

#### Version diff — UI-28
- History screen: tick two versions (or "Compare with current" on a row) → `View::PageDiff`.
  Unified view by default (inserted blocks with `ok` tint + left bar + underline, deleted with
  `danger` tint + strike, changed blocks with word-level marks); "Side by side" toggle; `N`/`P`
  jump to next/previous change; title changes shown as a header row. Restore from the diff
  header (`page.restore`).
- **Cmds (NEW):** `page.diff {"id","from":2,"to":5}` → `{"blocks":[{"op":"equal"|"insert"|"delete"|"change","from"?,"to"?,"inline"?:[…]}]}` (query; block LCS then word LCS, size-bounded).
- **Tests:** `diff_marks_are_not_colour_only`, `diff_next_change_shortcut`, `diff_huge_page_bounded`.
- **Shots:** `{"method":"ui.navigate","params":{"view":"diff","id":14,"from":1,"to":2}}`, `{"shot":"/tmp/tc-shots/diff.png"}`.

#### Templates — UI-29
- New-page dialog gains a template list on the left (Blank, plus space and workspace
  templates) with a live preview on the right. Templates are our own content in `sample.rs`
  (e.g. "Meeting notes", "Decision record", "How-to", "Retrospective" — generic document
  genres; the text, structure and placeholders are written by us). Space settings › Templates
  manages them (create from current page).
- **Cmds (NEW):** `template.list {"space"?}` (query), `template.create {"space"?,"name","body","description"?}`,
  `template.edit`, `template.delete`; `page.create` **NEW** param `"template": id`.
- **Tests:** `new_page_from_template_fills_body`, `save_page_as_template`.

#### Page tree: drag move, reorder, copy — UI-30
- Tree rows are drag sources; dropping on the middle of a row makes the page a child, on the
  top/bottom quarter places it before/after (accent line indicator); hovering a collapsed
  node 600 ms expands it; dropping a page into its own subtree is refused visually (no drop
  indicator) and by the engine. Keyboard: `⋯ › Move…` opens a page picker (palette-style,
  shows breadcrumbs per result) with "as child / before / after".
  Tree is lazy (only expanded nodes are walked) and long sibling lists render with
  `show_rows`.
- **Cmds:** `page.move` **NEW** params `"before": id | "after": id` (uses `Page::position`);
  **NEW** `page.copy {"id","parent"?,"title"?,"children"?: false}`.
- **Tests:** `tree_drag_makes_child`, `tree_drag_reorders_siblings`, `tree_drop_into_own_subtree_refused`,
  `move_picker_keyboard`, `tree_500_siblings_virtualised`.

#### Drafts — UI-32
- The editor autosaves a draft every 5 s of idle typing; reopening shows "Resume your draft
  from 12:04?"; Publish clears it. This also solves G1 durably (UI-1 is the interim guard).
- **Cmds (NEW):** `page.draft.save {"id","title","body","baseVersion"}`, `page.draft.get {"id"}` (query),
  `page.draft.discard {"id"}` — drafts are per user, excluded from undo (mark them
  `.query()`-like "no undo step" but persisted: needs a CommandSpec flag `.no_undo()`).

### 3.4 M4 multi-user surfaces

#### Sign-in and workspace picker — UI-33
- Start screen when no file/session: Recent local files (craft-settings), "Open file…",
  "New workspace", and **Connect to server** (URL, then the server's sign-in: username +
  password form, or "Continue in browser" for SSO with a loopback redirect). Signed-in, a
  workspace list from the server. The avatar menu gets "Signed in as … · Sign out".
  "Acting as" disappears in server mode (kept only for local files and tests).
- Sign-in is host-level (`apps/trackcraft` + `craft-collab` identity), not an engine command;
  control methods `ui.connect {"url"}` and `ui.signOut` exist for scripts.
- **Tests:** `start_screen_lists_recent_files`, `connect_error_inline_not_toast`.

#### Presence, live updates, conflicts — UI-34
- Presence avatars (max 3 + "+N") in issue and page headers: "viewing" (outline) vs "editing"
  (filled ring). Page editor shows who else is editing; with Phase B editor + op log, edits
  merge live (remote carets as thin coloured bars with name flags); before that, the existing
  `StaleVersion` conflict bar remains.
- Issue fields changed remotely flash their row once (200 ms tint, respects reduced-motion
  preference) and the history tab updates. A remote change to a field you are editing shows
  "Changed by Ann to …" under the field with Keep mine / Take theirs.
- Presence is read-only engine/session state fed by the collab layer (**NEW** `Session::presence()`
  accessor, not a command); `presence.list {"target"}` (query) for agents.
- **Tests:** `presence_avatars_cap_at_three`, `remote_field_change_offers_choice`, `reduced_motion_disables_flash`.

#### Notifications inbox — UI-35
- Bell with unread count (99+). Popover: latest 20 grouped Today / Earlier, each row = actor
  avatar, one-line summary, target, time; click opens the target and marks read. "Open inbox"
  → full view with Direct (mentions, assigned) / Watching tabs, `J/K` navigation, `Shift+R`
  mark read, "Mark all read". Settings › Notifications: per-event toggles (in-app, email).
- **Cmds (NEW):** `notification.list {"filter":"direct"|"watching"|"all","unreadOnly"?,"startAt"?}` (query),
  `notification.markRead {"ids":[…]} | {"all":true}`, `notification.settings {"events":{…}}`.
- **Tests:** `bell_count_caps_at_99_plus`, `open_notification_marks_read`, `inbox_keyboard_nav`.

#### Suite-server cross links with Chatcraft — UI-36
- Issue and page `⋯ › Share to channel…` (channel picker from the integration API, optional
  message); a "Discussed in #channel" chip on issues/pages linked from chat; chat links pasted
  into comments/pages unfurl as cards (channel name, first line, time). Hidden entirely when
  no chat integration is configured.
- **Cmds (NEW):** `share.toChat {"target":{"issue":"LUME-1"}|{"page":12},"channel","message"?}`,
  `link.preview {"url"}` (query; returns a preview card or nothing). Both go through the
  versioned integration API, so they work against a standalone Chatcraft too.

#### Offline and sync status — UI-37
- Status bar: Online / Offline / Syncing N; offline banner on mutating screens ("Changes will
  sync when you reconnect"); queued commands list in a popover with per-item retry/discard
  after a server rejection.

---

## 4. Keyboard shortcut map

`Mod` = Cmd on macOS, Ctrl elsewhere. "Registry" = declared on the `CommandSpec`; "UI" =
`ui.*` / navigation only. Single letters never fire while a text field has focus or a modal
is open.

### Global
| Keys | Action | Source | Status |
|---|---|---|---|
| `Mod+K`, `Mod+Shift+P` | Command palette | registry `app.commands` (`Mod+Shift+P`); `Mod+K` UI | exists — add `Mod+K` as a second chord on the spec so the help lists both |
| `/` | Focus search | registry `search.jql` | exists |
| `C` | Create in current half (issue on tracker views, page on wiki views) | registry `issue.create` / `page.create`, scoped | exists for issue; page scope NEW (G4) |
| `Mod+Z` / `Mod+Shift+Z` | Undo / Redo (not while typing) | registry | exists; redo guard missing (G2) |
| `Mod+S`, `Mod+Shift+S`, `Mod+O` | Save, Save As, Open | registry / host | exists |
| `Mod+,` | Settings | UI | NEW |
| `Mod+\` | Toggle sidebar | UI | NEW |
| `Alt+Left` / `Alt+Right`, `Mod+[` / `Mod+]` | Back / Forward | UI | Back exists; Forward NEW |
| `G` then `H` / `I` / `B` / `L` / `F` / `W` / `N` | Go: Home / Issues / Board / Backlog / Filters / Spaces / Notifications | UI | NEW |
| `?` | Shortcut help | UI | NEW |
| `Esc` | Close modal / popover / peek; cancel inline edit | UI | exists for modals |

### Issue list and backlog
| Keys | Action | Status |
|---|---|---|
| `J` / `K`, `Down` / `Up` | Focus next / previous row | NEW |
| `Enter`, `O` | Open focused issue | NEW |
| `Space` | Peek pane | NEW |
| `X`, `Shift+X` | Toggle select / extend selection | NEW |
| `Mod+A` | Select all on page | NEW |
| `Mod+Enter` (in the JQL field) | Run query | NEW (Enter also runs) |
| `Mod+Shift+Up/Down`, `Mod+Shift+Home/End` | Rank (backlog) | NEW (`issue.rank`) |
| `Shift+S` (backlog, a row selected) | Move to sprint… | exists (UI-11) — `Mod+Shift+S` stays Save As everywhere; `Shift+S` is free in every scope and never fires while typing. |
| `A`, `I`, `E`, `T`, `W` | Assign, Assign to me, Edit, Transition, Watch — on focused row | registry, NEW dispatch |

### Board
| Keys | Action | Status |
|---|---|---|
| Arrows / `H J K L` | Move focus between cards and columns | NEW |
| `Shift+Left` / `Shift+Right` | Transition card to adjacent column | NEW (keyboard twin of drag) |
| `Mod+Shift+Up/Down` | Rank within column | NEW (needs `issue.rank`) |
| `Enter` / `Space` | Open / Peek | NEW |
| `F` | Focus card filter | NEW |
| `A`, `I`, `T` | Assign, Assign to me, Transition focused card | registry, NEW dispatch |

### Issue view
| Keys | Action | Source | Status |
|---|---|---|---|
| `E` | Edit summary (then `Tab` to description) | `issue.edit` | declared, not wired (G3) |
| `A` | Open assignee picker | `issue.assign` | declared, not wired |
| `I` | Assign to me | `issue.assignToMe` | declared, not wired |
| `M` | Focus comment box | `issue.comment` | declared, not wired |
| `L` | Open label editor | NEW shortcut on `issue.edit`'s labels path — conflicts with nothing in Issue scope (`page.labels` is Page scope) | NEW |
| `T` | Open status picker | `issue.transition` | NEW shortcut |
| `W` | Watch / stop watching | `issue.watch` | NEW |
| `Mod+Enter` | Save the open editor (description, comment) | UI | NEW |
| `Mod+Backspace` | Delete issue (with confirm) | `issue.delete` | NEW |

### Page view and editor
| Keys | Action | Source | Status |
|---|---|---|---|
| `E` | Edit page | `page.edit` | exists |
| `L` | Edit labels | `page.labels` | declared, not wired (G3) |
| `C` | New child page | `page.create` (Page scope) | NEW dispatch (G4) |
| `Shift+H` | History | `page.history` | NEW shortcut (plain `H` is board/list focus) |
| `N` / `P` (diff view) | Next / previous change | UI | NEW |
| `Mod+Enter` (editor) | Publish | `page.edit` | NEW |
| `Mod+B` / `Mod+I` / `Mod+K` / `Mod+E` (editor) | Bold / Italic / Link / Inline code | UI (editor scope, text field focused) | NEW — note `Mod+K` is the palette globally; in the editor scope the editor consumes it first and the palette stays on `Mod+Shift+P`. Same trade other editors make; documented in `?`. |
| `Esc` (editor) | Cancel (asks if changed) | UI | NEW |

Consistency checks performed against today's registry:
- `C` (`issue.create`, `page.create`) and `E` (`issue.edit`, `page.edit`) are duplicated
  across specs — legal only with scopes (**NEW** `CommandSpec::scope`). Add a registry test
  `no_duplicate_chord_within_a_scope`.
- `Mod+S` is `file.save`; the editor must not steal it (publishing is `Mod+Enter`; `Mod+S` in
  the editor saves the workspace file *including* a draft once UI-32 lands).
- `/` stays search even on wiki views (searches pages when the query is not JQL).

---

## 5. Component inventory

Extract into `crates/ui-egui/src/widgets/` (one file each, all taking `&Tokens`, returning
`egui::Response`, each with an accessible name and a kittest in `widgets/tests.rs`):

| Widget | Today | Notes |
|---|---|---|
| `pill(status, category)` | private in `issues.rs` | Width-capped with truncation; used by list, board, links, history, macros. |
| `avatar(user)` / `who()` / `user_chip` | `icons::avatar` + private `who` | Add unknown-user and deactivated states, presence ring (M4). |
| `priority_icon`, `issue_type_icon` | `icons.rs` | Keep drawing in code; add Bug/Story/Task/Epic/Sub-task + custom-type fallback (letter in a rounded square). |
| `issue_row` | inline in `list` | One row renderer for list, backlog, links, macros, palette (density aware). |
| `issue_card` | `card_body` | Board + backlog drag preview; 3-line clamp. |
| `card(frame)` | `shell::card` | Move as is. |
| `field_row(label, editor)` | Grid rows in `fields` | Label + value + click-to-edit + labelled_by wiring done once. |
| `user_picker` | duplicated ComboBoxes ×3 | Searchable popover (type to filter 300 users), "Unassigned", "Me" first; built on `craft_ui::ComboBox`. |
| `issue_picker` / `page_picker` | none | Key/summary or title search with breadcrumbs; for links, parent, move. |
| `label_editor` | space-separated TextEdit | Chips with ✕ (drawn), autocomplete from existing labels, Backspace removes last. |
| `date_picker` | none | Wrap `egui_extras::DatePickerButton` (suite fork, `datepicker` feature, chrono) with our tokens and a text field that accepts `2026-11-14`, `+3d`, `next fri` (parser in engine, shared with JQL relative dates). |
| `number_field` | `DragValue` | Use `craft_ui::localized::NumberField` (commit on Enter/blur, regional digits). |
| `markdown_view` | `markup_view.rs` | Gains tables, panels, tasks, media, extension host (`MacroHost`), inline cards for issue keys. |
| `markdown_editor` | inline in `wiki::editor` and description edit | One component: toolbar, shortcuts, split/preview toggle, draft hook; used by pages, descriptions, comments (comments get a compact one-line-growing variant). |
| `tree` | `shell::tree` | Generic over a node source; lazy expand, `show_rows`, drag/drop zones, keyboard (Left/Right collapse/expand, Up/Down, Enter). Used by sidebar page tree, move picker, (later) epic tree. |
| `empty_state`, `inline_banner`, `skeleton_rows` | ad hoc | §3.1. |
| `toast_stack`, `status_bar` | `show_toast` | §3.1. |
| `selection_bar` | none | Bulk actions (UI-19). |
| `segmented_progress` | none | Sprint/version progress by status category, with text alternative. |
| `tabs` | none | Use `craft_ui::tabs` (fit + accessible tab responses) for issue activity tabs, settings tabs, project settings. |

**Reuse from craft-libs (do not rebuild):** `craft_ui::ComboBox` (mandatory), `craft_ui::tabs`,
`craft_ui::panes::PaneHost` (peek pane, later a right-hand details pane on pages),
`craft_ui::layout::ResizeHandle` (sidebar / split editor), `craft_ui::buttons` (icon buttons
with our painting), `craft_ui::localized::NumberField`, `craft_ui::ime` (editor CJK input),
`craft_ui::testing::Screens` (run every screen in the suite's test languages — add one test
`screens_in_test_languages` once craft-i18n catalogs exist), `craft_ui::fonts`,
`craft-keymap` (bindings, scopes, overrides, conflict detection, shortcut editor model),
`craft-menu` (native macOS menu bar built from the same command ids — the in-window File menu
stays for Windows/Linux/web), `craft-settings` (preferences, recents, window geometry),
`craft-i18n` (every user-visible string through it from UI-7 on; strings are English source
text, no keys).

**Build ourselves:** everything in the table above that is domain-specific (pills, issue
row/card, pickers, label editor, markdown view/editor, tree, toasts) — craft-ui deliberately
stays document-agnostic. **Not used:** `craft_ui::docking` (TrackCraft has no user-arranged
dock layout; panes are enough) and `craft_ui::icons` SVG loading (our icons are code).

---

## 6. Backlog of UI work items

Sizes: **S** ≤ 1 day, **M** 2–4 days, **L** 1–2 weeks for one agent. "Engine" lists the NEW
commands/params the item needs; an item whose engine part is non-trivial should be split
into an engine PR (owner of `crates/engine`/`crates/model`) and a UI PR. "Files" are the
files the item writes, to keep parallel agents apart. After UI-2, the hot shared files are
`lib.rs` (Action/Then), `route.rs` (View + `ui.navigate`) and `tests.rs` — rule: append-only
arms, one per item, and put tests in `tests/<area>.rs`.

### Wave 0 (serial, first)
| ID | Item | Size | Engine deps | Files | Depends on |
|---|---|---|---|---|---|
| UI-1 | Unsaved-edit guard: navigating/Back/palette jump away from a dirty page editor, description or create dialog asks Keep editing / Discard; Esc in create keeps the draft | S | — | `lib.rs`, `wiki.rs`, `issues.rs`, `tests.rs` | — |
| UI-2 | Module split, no behaviour change: `issues.rs` → `issues/{list,board,detail,create}.rs`; `widgets/` with `pill`, `avatar/who`, `card`, `field_row`, `empty_state`; `route.rs` (View enum, `ui.navigate` parsing); `tests/` dir; shot scripts dir | M | — | all of `crates/ui-egui/src` | UI-1 |

### Wave 1 — M1 polish (parallel after UI-2)
| ID | Item | Size | Engine deps | Files | Depends on |
|---|---|---|---|---|---|
| UI-3 | Issue list: virtualised `TableBuilder`, header sort, column chooser, paging, selection + keyboard, width truncation, empty/error states | M | NEW `search.orderBy` | `issues/list.rs`, `widgets/issue_row.rs`, `Cargo.toml` (egui_extras), `tests/list.rs` | UI-2 |
| UI-4 | Scoped keymap from the registry (craft-keymap), wire E/A/I/M/L/T + page L/C, redo guard, G-chords, `?` overlay, `ui.key` control method | M | NEW `CommandSpec::scope`; `T` on `issue.transition`; `Mod+K` second chord on `app.commands`; test `no_duplicate_chord_within_a_scope` | `keys.rs` (new), `lib.rs::logic`, `route.rs`, `crates/engine/src/cmd.rs` + `cmd/*.rs` (shortcut metadata only), `tests/keys.rs` | UI-2 |
| UI-5 | Toast stack with Undo + error kind; status bar (save state, path, selection) | M | `Session::undo_depth()` accessor | `toast.rs` (new), `status_bar.rs` (new), `lib.rs` (Then), `tests/toast.rs` | UI-2 |
| UI-6 | Empty/loading/error states everywhere + hostile-data pass (orphan status column, unknown users, deep trees, long titles) + `sample::stress` fixture + ui_shot `fixture` flag | M | `sample::stress` (test helper) | `widgets/empty_state.rs`, `issues/board.rs` (column only), `shell.rs` (home), `wiki.rs` (empty page), `examples/ui_shot.rs`, `crates/engine/src/sample.rs`, `tests/hostile.rs` | UI-2 |
| UI-7 | Settings screen: preferences (craft-settings; tri-state theme, density, autosave), shortcuts editor, workspace name + users | M | NEW `workspace.rename`, `user.edit` | `settings.rs` (new), `route.rs`, `shell.rs` (avatar menu), `theme.rs` (density), `tests/settings.rs` | UI-2 (UI-4 for the shortcuts tab) |
| UI-8 | Sidebar scaling (filter box, starred/recent, directories), back/forward with per-view state, palette `ui_route` table | M | NEW `user.favourite` | `shell.rs`, `directory.rs` (new), `palette.rs`, `lib.rs` (forward stack), `tests/nav.rs` | UI-2 |
| UI-9 | Issue view: link add/remove, comment edit, type/parent change, NumberField points, label editor, user picker, reverse-link cache, peek pane, create-dialog polish | M | NEW `issue.unlink`, `issue.editComment`; `issue.edit` params `type`, `parent` | `issues/detail.rs`, `issues/create.rs`, `widgets/{user_picker,issue_picker,label_editor}.rs`, `tests/issue.rs` | UI-2 |
| UI-10 | Board: keyboard focus/move, virtualised columns, card clamp, WIP display hook | S | — | `issues/board.rs`, `widgets/issue_card.rs`, `tests/board.rs` | UI-2, UI-4 |

### Wave 2 — M2 tracker
| ID | Item | Size | Engine deps | Files | Depends on |
|---|---|---|---|---|---|
| UI-11 | Backlog with drag/keyboard rank, sprint sections, inline create, move-to-sprint | L | NEW `board.backlog`, `issue.rank`, `sprint.create/edit/delete`, `issue.setSprint` | `backlog.rs` (new), `route.rs`, `shell.rs` (project sub-nav), `tests/backlog.rs` | UI-3 (selection), UI-10 |
| UI-12 | Sprint start/complete dialogs, sprint board, board column config + swimlanes | M | NEW `sprint.start`, `sprint.complete`, `board.view` param `sprint`, `board.configure` | `sprints.rs` (new), `issues/board.rs`, `tests/sprints.rs` | UI-11 |
| UI-13 | Activity tabs: Comments / History / All | M | NEW `issue.history` (needs op log) | `issues/activity.rs` (new), `issues/detail.rs` (tab host only) | UI-9, op log |
| UI-14 | Saved filters: save/edit/revert, directory, system filters, sidebar stars | M | NEW `filter.create/edit/delete/list/get` | `filters.rs` (new), `issues/list.rs` (header only), `shell.rs` (filters section) | UI-3, UI-8 |
| UI-15 | Basic search builder + JQL autocomplete | L | NEW `search.fields`, `search.suggest`, `search.toBasic`, `search.fromBasic` | `search_builder.rs` (new), `issues/list.rs` (mount point) | UI-3 |
| UI-16 | Releases + components screens; version/component/due fields on issues | M | NEW `version.*`, `component.*`; `issue.edit` params `fixVersions`, `affectsVersions`, `components`, `dueDate` | `releases.rs` (new), `project_settings.rs` (new), `issues/detail.rs` (field rows), `widgets/date_picker.rs` | UI-9 |
| UI-17 | Custom fields admin + rendering | L | NEW `field.*`; `issue.edit` `fields` | `settings.rs` (Fields tab), `issues/detail.rs`, `widgets/field_row.rs` | UI-7, UI-16 |
| UI-18 | Workflow editor (draft + publish, transition grid) and board columns tab | L | NEW `workflow.get`, `workflow.publish` | `project_settings.rs` (workflow tab), `workflow_editor.rs` (new) | UI-12, UI-16 (project settings shell) |
| UI-19 | Bulk edit selection bar + dialog | M | NEW `issue.bulkEdit`, `issue.bulkTransition`, `issue.bulkDelete` | `bulk.rs` (new), `issues/list.rs` + `backlog.rs` (mount only) | UI-3, UI-5 |
| UI-20 | Watchers (+ `W`) | S | NEW `issue.watch/unwatch/watchers` | `issues/detail.rs` (header), `widgets/user_picker.rs` | UI-9 |
| UI-21 | Attachments (issues first; widget reused by pages) | M | NEW `attachment.add/delete/list/get` | `attachments.rs` (new), `issues/detail.rs` (section), host drag-drop in `apps/trackcraft` | UI-9 |
| UI-22 | Clone, move, CSV export | S | NEW `issue.clone`, `issue.move`, `search.exportCsv` | `issues/detail.rs` (⋯ menu), `issues/list.rs` (Export) | UI-9, UI-3 |
| UI-23 | Issue ↔ page links + issue-key inline cards in rich text | M | NEW `issue.linkPage`, `issue.unlinkPage`, `issue.pages` | `issues/detail.rs` (Pages section), `wiki.rs` (footer), `markup_view.rs` | UI-9 |
| UI-24 | Home dashboard gadgets; burndown + velocity | M | NEW `report.burndown`, `report.velocity` | `shell.rs` (home → `home.rs`), `reports.rs` (new) | UI-12, op log |

### Wave 3 — M3 wiki
| ID | Item | Size | Engine deps | Files | Depends on |
|---|---|---|---|---|---|
| UI-25 | Markdown editor component: toolbar, shortcuts, view toggle, paste link/image, scroll sync; used by pages, descriptions, comments | M | markup: table/task/panel/extension/media blocks (Markdown + ADF) | `widgets/markdown_editor.rs` (new), `wiki.rs` (editor mount), `issues/detail.rs` (description/comment mount) | UI-2; markup nodes |
| UI-26 | Render tables, panels, tasks, macros (TOC, children, issue list); task toggle in view mode | M | NEW `page.toggleTask` | `markup_view.rs`, `macros.rs` (new) | UI-25 (markup nodes) |
| UI-27 | Page + inline comments | M | NEW `page.comment/editComment/deleteComment/resolveComment/comments` | `wiki_comments.rs` (new), `wiki.rs` (mount) | UI-26 |
| UI-28 | Version diff view | M | NEW `page.diff` | `wiki_diff.rs` (new), `wiki.rs` (history: compare ticks), `route.rs` | UI-2 |
| UI-29 | Templates in new-page dialog + space template management | M | NEW `template.*`; `page.create` param `template` | `wiki.rs` (dialog), `space_settings.rs` (new), `crates/engine/src/sample.rs` (our template text) | UI-25 |
| UI-30 | Tree drag move/reorder, Move… picker, copy page; generic `tree` widget | M | `page.move` params `before`/`after`; NEW `page.copy` | `widgets/tree.rs` (new), `shell.rs` (sidebar uses it), `wiki.rs` (⋯ menu) | UI-8 |
| UI-31 | WYSIWYG block editor (Phase B) | L | markup `EditOp` model (pure, L1) | `editor/` (new dir), `crates/markup/src/edit.rs` | UI-25, UI-26 |
| UI-32 | Page drafts (autosave, resume) | S | NEW `page.draft.save/get/discard`; `CommandSpec::no_undo()` | `wiki.rs`, `widgets/markdown_editor.rs` | UI-25, UI-1 |

### Wave 4 — M4 team server
| ID | Item | Size | Engine deps | Files | Depends on |
|---|---|---|---|---|---|
| UI-33 | Start screen, connect + sign-in, workspace picker, account menu | M | host + `craft-collab` identity; `ui.connect`, `ui.signOut` | `start.rs` (new), `shell.rs` (avatar menu), `apps/trackcraft` | UI-7 |
| UI-34 | Presence avatars, live field updates, conflict choices, remote carets (with UI-31) | M | `Session::presence()`; NEW `presence.list` | `presence.rs` (new), `issues/detail.rs`, `wiki.rs` (headers) | UI-33 |
| UI-35 | Notifications bell, popover and inbox | M | NEW `notification.list/markRead/settings` | `notifications.rs` (new), `shell.rs` (bell), `route.rs` | UI-33 |
| UI-36 | Chatcraft cross links: share to channel, unfurls, "Discussed in" chips | M | NEW `share.toChat`, `link.preview` (integration API) | `chat_links.rs` (new), `markup_view.rs` (unfurl card) | UI-33, UI-23 |
| UI-37 | Offline/sync status, queued-ops popover | S | collab queue accessor | `status_bar.rs` | UI-5, UI-33 |

### Sequencing notes for parallel agents
- **Wave 0 is serial** (UI-1 then UI-2); everything else rebases onto UI-2's file layout.
- Wave 1 runs 4 agents comfortably: {UI-3}, {UI-4 → UI-10}, {UI-5, UI-6}, {UI-7, UI-8, UI-9}
  — UI-8 and UI-9 touch different files; UI-7 and UI-8 both touch `shell.rs` (avatar menu vs
  sidebar) so keep them with one agent or land UI-8 first.
- Engine command PRs for Wave 2 (sprints/rank, filters, versions/components, bulk) can start
  during Wave 1: they touch `crates/engine` and `crates/model` only. Each lands with catalog
  rows flipped (`catalog.rs`) and `hostile_params_never_panic` shapes.
- The op log (gaps #3) gates UI-13, UI-24 and the M4 items; schedule it as the first M2
  engine task.
- Every item ends with: its kittest tests green in both themes, its shot script run and the
  PNGs looked at, `every_view_renders_in_both_themes` extended for new views, and the parity
  doc + `docs/gaps.md` updated if a catalog row changed.

## Revision history

- 2026-10-31: first version.
