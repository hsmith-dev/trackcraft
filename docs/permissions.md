# Permissions

> **Status:** 2026-10-31 · M4.3 — workspace, project and space roles, access levels, page restrictions; enforced on the server (REST, native commands, live sync).

Who may see and change what in a workspace. The concepts follow Atlassian's public
documentation of project roles and project access levels, permission schemes, space
permissions and page restrictions; the roles, levels and rules below are TrackCraft's own,
simpler model of them, described in our words.

- Data and checks: `crates/model/src/access.rs` (`Workspace::access`, one field, absent in older files).
- Command → permission mapping, sync delivery: `crates/engine/src/authz.rs`.
- Commands: `crates/engine/src/cmd/access.rs`.

## Where it is enforced

| Where | Enforced? |
| --- | --- |
| `trackcraft-server` (Jira/Confluence REST, `/api/command`, sync snapshot and events) | Yes: `Hosted::new` calls `Session::set_enforce(true)`. |
| Desktop app on a local `.track` file, CLI, MCP | No: one person owns the file. The roles are stored and travel with the file to a server. |
| Desktop client of a server (remote mode) | The server decides; the client's own copy doesn't check. A refused command is dropped with a toast (`sync.md`). |
| Replays (`Session::apply_remote`) | Never: the server already checked. |

## Roles

**Workspace role** — `admin`, `member`, `viewer`.

- **Admins** see everything (restricted pages included) and may do everything. At least one
  admin always remains: demoting the last one is refused (`access.setWorkspaceRole`, the CLI).
- **Members** get what their project and space roles say.
- **Viewers** are read-only everywhere: every project and space role is capped at viewer.
- A user with no stored role is an **admin**, so every user of a file written before
  permissions existed keeps full rights and nothing breaks. `user.add` and imports store new
  users as **members**. The sample workspace: riley admin, sam and jo members (jo administers OPS).

**Project role** — `administrator`, `member`, `viewer` per user, plus the project's **access
level** for members without a role (the three levels of a team-managed project in Jira Cloud):

| Level | Members without a project role |
| --- | --- |
| `open` (default) | member |
| `limited` | viewer |
| `private` | nothing: the project, its issues and sprints are hidden |

**Space role** — `admin`, `editor`, `viewer`, with the same levels (`open` = editor,
`limited` = viewer, `private` = hidden).

Whoever creates a project or space administers it (stored as an explicit role unless they are
a workspace admin already).

## What each role may do

| | Viewer | Member / Editor | Administrator / Admin |
| --- | --- | --- | --- |
| Browse the project (issues, boards, backlog, search) | ✓ | ✓ | ✓ |
| Comment; edit and delete own comments | ✓ | ✓ | ✓ |
| Create, edit, transition, link issues | own issues¹ | ✓ | ✓ |
| Assign, rank, plan sprints, delete issues | | ✓ | ✓ |
| Delete other people's comments | | | ✓ |
| Change project roles and access level | | | ✓ |
| View the space and its pages | ✓ | ✓ | ✓ |
| Comment on pages | ✓ | ✓ | ✓ |
| Create, edit, move, copy, delete, label, restore pages | | ✓ | ✓ |
| Restrict a page | | pages they can edit² | ✓ (any page) |
| Change space roles and access level, space templates | | | ✓ |

Workspace-wide: renaming the workspace, adding users, workspace templates, workspace roles —
admins. Editing a user profile — the user themself or an admin. Creating projects and spaces —
members and admins.

1. **Reporter and assignee.** A Jira permission scheme can grant "edit" and "transition" to the
   issue's reporter or current assignee as well as to roles. TrackCraft has one fixed scheme and
   bakes that grant in: a project **viewer** who is a workspace **member** may edit, transition
   and link issues they reported or are assigned. Workspace viewers stay read-only.
2. Restricting a page needs edit access to it, and the restricting user must stay on every list
   they set (so nobody locks themself out); space admins may set any lists, which is also how a
   page whose restrictions exclude everyone is recovered.

**Deleting issues** is a member right (team-managed projects let members delete issues); in a
company-managed Jira scheme it is usually administrators only. Configurable schemes are later work.

## Page restrictions

- A page has an optional **view list** and **edit list** of users (`page.restrict {"id",
  "view": [...] | null, "edit": [...] | null}`; a missing key leaves that list alone, `null`
  removes it). At most 1000 users each; every one must be a workspace member.
- **View restrictions are inherited**: a user must be on the view list of the page and of every
  restricted ancestor. **Edit restrictions are not**: they apply to that page only. This is how
  Atlassian's public docs describe page restrictions.
- Someone on a page's edit list may view that page even if its view list leaves them out.
- Restrictions only narrow what the space role allows; they never grant more.
- Moving, copying with children, and deleting take the subtree along, so they need view (copy)
  or edit (move's page, delete's whole subtree) access to every page in it.

## Hidden things read as missing

Something a user can't browse is reported exactly like something that doesn't exist (the
model's own "no issue SEC-1", "no page with id 12"), so REST answers **404** and reveals
nothing; Jira and Confluence answer 404 for issues and pages you can't see too. Something they
can see but not change is **403** with `permission denied: …` (Jira and Confluence error shapes as
usual). Over the native endpoint, the same statuses with `{"error", "status"}`.

Queries are never refused; they are filtered. Under enforcement `Session::run` runs every query
on `Workspace::view_for(user)` — a copy without hidden projects (and their issues, sprints and
links to them), hidden spaces, restricted pages (with their subtrees) and access entries about
them. So `search.jql`, `board.*`, `project.list`, `space.tree`, `page.search`, `template.list`,
`access.get` and `workspace.info` show only what the user can see, and `issue.get` /
`page.get` of a hidden one fail as missing. The REST handlers read through the same view
(`Tx::ws`). For users who can see everything (admins, or no restrictions in play) no copy is
made; otherwise the copy costs O(workspace) per request — fine at team scale.

## Commands

| Command | Who |
| --- | --- |
| `access.get {}` | anyone: your role, everyone's workspace role, each visible project's and space's access level, roles and `myRole`, restrictions on visible pages |
| `access.check {"command", "params", "user"?}` | anyone: `{"allowed", "reason"}` — what a UI uses to enable controls |
| `access.setWorkspaceRole {"user", "role"}` | workspace admins (never the last admin) |
| `access.setProjectRole {"project", "user", "role" \| null}`, `access.setProjectAccess {"project", "access"}` | project administrators |
| `access.setSpaceRole {"space", "user", "role" \| null}`, `access.setSpaceAccess {"space", "access"}` | space admins |
| `page.restrict {"id", "view"?, "edit"?}` | page editors (keeping themselves listed), space admins |

`trackcraft-server user role --user U --role admin|member|viewer [--workspace NAME]` sets a
workspace role in the file (run it while the server is stopped; a running server would save
over it — online, use `access.setWorkspaceRole`).

A mutating command without a rule in `authz::authorize` needs a workspace admin; a test
(`every_mutating_command_has_a_rule`) fails until a new command gets one.

## Live sync

A user must only receive what they can browse (`sync.md`):

- **Snapshot**: `GET /api/sync/snapshot` is the user's `view_for` copy.
- **Events**: when the server journals a command it also records what it touched (projects,
  spaces, pages; whether it reshapes a page tree; whether it changes access) — the *scope*,
  taken before the command runs. Streaming entry by entry, the server judges each scope against
  the user's current access:
  - everything visible → the entry as it is;
  - nothing visible → a **`sync.skip` placeholder** `{rev, command: "sync.skip", params:
    {"idsAfter": n}, at, idsBefore}` (no actor, no params), which `apply_remote` applies by
    moving the revision and the id counter — the replica stays deterministic without the
    hidden command;
  - anything else — access changed (`access.*`, `page.restrict`), visible and hidden things
    mixed (a link from a visible issue to a hidden one), a page-tree change in a space that has
    pages hidden from the user (sibling positions would differ), a target that no longer exists
    → **`resync`**, and the stream ends; the client reloads its (filtered) snapshot.
  - Workspace admins get every entry unchanged.
- `crates/engine/src/tests_access.rs::filtered_replica_follows_the_journal` replays a mixed
  journal for each user and requires their replica to equal their view after every entry.

### Trade-offs and what still leaks

The design is simple and correct for content, at the price of extra resyncs (every access change
resyncs every non-admin client; page moves in spaces with restricted pages do too). Known
leaks, all metadata:

- **Revisions and timing**: a skip placeholder reveals that *something* the user can't see
  changed, when, and how many ids it allocated (`idsBefore`/`idsAfter`) — so activity volume in
  hidden projects and spaces is visible to a determined client.
- **Ids**: page, comment and sprint ids come from one workspace counter, so gaps hint at hidden
  objects; issue keys are per project (a visible project's numbers reveal nothing hidden).
- **History text**: an issue's history keeps "linked to SEC-1" style entries naming a hidden
  issue key (the link itself is removed from the view).
- **Users and roles**: every user and their workspace role are visible to all members; project
  and space roles are visible for projects and spaces the user can see.
- **Error wording**: a copy with children, or a delete, refused because of child pages the user
  can't see says so (without ids), revealing that hidden children exist.
- **Results of allowed commands** count hidden children (`page.move`'s `pages`), and replies
  are not further filtered.
- **New fields**: `view_for` removes what it knows about; a workspace field added later that can
  point at hidden things (e.g. notifications) must be filtered there too, or it leaks.
- A user whose access shrinks keeps whatever their client already downloaded.

## For the UI (later round)

The desktop UI doesn't show permissions yet. It needs: an Admin › Permissions screen
(`access.get`; `access.setWorkspaceRole`), project settings → Access (`access.setProjectAccess`,
`access.setProjectRole`), space settings → Permissions (`access.setSpaceAccess`,
`access.setSpaceRole`), a page "Restrictions" dialog (`page.restrict`) with a lock badge on
restricted pages in the tree, and disabled controls with a reason from `access.check` (or
`Session::authorize`) when connected to a server — plus a toast when a resync follows an access
change.
