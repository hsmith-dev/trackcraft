# Notifications and watchers

> **Status:** 2026-10-31 · M4.5 — in-app notifications, watchers and `@mentions`. Email is out of
> scope for now.

## Model

Notifications live **in the workspace** (`Workspace::inbox`, `crates/model/src/notify.rs`), so they
sync like everything else (`sync.md`): a command journal entry replays on every client and
produces the same inboxes there, with no server-side notification service. The edit primitives
generate them from workspace state only — the actor, the command's pinned `now`, the issue or
page — so replay is deterministic. Notification ids come from the inbox's own counter, so they
never shift workspace ids (`idsBefore` in the journal).

The inbox holds, per user, a list of notifications (newest kept, at most 500; the oldest are
dropped) and a watch list of issue keys and page ids (at most 10 000).

| Kind (`kind`) | When |
| --- | --- |
| `assigned` | An issue is created with, or assigned to, you. |
| `mentioned` | `@you` newly appears in an issue description or comment, a page body (create or edit) or a page comment. |
| `commented` | Someone comments on an issue or page you watch. |
| `statusChanged` | An issue you watch moves to another status. |
| `resolved` | An issue you reported enters a Done-category status (instead of `statusChanged`). |

- **Auto-watch:** reporters, assignees, commenters, page authors and editors.
- **Never the actor:** nobody is notified about their own action.
- **Once per event:** a watcher who is also mentioned gets the mention only.
- Deleting an issue or page removes it from every watch list; notifications about it stay (the
  Inbox says it no longer exists).
- Imported issues go through the same primitives, so an import notifies assignees too.

### Mentions

A mention is a structured `Inline::Mention` (ADF `mention`) or Markdown text `@id` where `id` is
a known user (`trackcraft_markup::Doc::mentions`, `Doc::from_markdown_mentions`,
`Doc::link_mentions`). `@` must start the text or follow a non-alphanumeric character (so
`ann@example.org` is not a mention); trailing `.`, `-`, `_` are dropped until a user matches; code
spans and link text never mention.

## Commands

| Command | Params | Result |
| --- | --- | --- |
| `inbox.list` (query) | `{"unreadOnly"?: bool, "limit"?: 0–500 (50)}` | `{user, unread, total, notifications: [{id, kind, target: {type, key \| id}, actor, at, title, excerpt, read}]}`, newest first, for the acting user |
| `inbox.markRead` | `{"ids": [..]}` or `{"all": true}` | `{marked, unread}` |
| `issue.watch` / `issue.unwatch` | `{"key": "LUME-1"}` | `{changed, watching, watchCount}` |
| `page.watch` / `page.unwatch` | `{"id": 14}` | same |
| `watchers.list` (query) | `{"key"}` or `{"id"}` | `{watchers, watchCount, watching}` |

Marking read is a workspace change (it syncs, so another device sees it read); it has an undo
step like any other change.

## UI

- A bell in the top bar, next to the avatar, with the unread count; it opens the Inbox
  (`G` then `N`, or `ui.navigate {"view": "inbox"}`).
- The Inbox lists notifications newest first: avatar, who did what, an excerpt, relative time and
  an unread dot. Clicking one marks it read and opens its issue or page. "Unread only" and
  "Mark all read" sit in the header.
- Issue and page headers have a Watch / Watching toggle.
- Screenshot script: `crates/ui-egui/shots/inbox.jsonl`.
