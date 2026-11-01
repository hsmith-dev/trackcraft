//! Notifications and watchers (M4.5).
//!
//! The [`Inbox`] lives in the workspace, so it syncs like everything else: the edit primitives
//! generate notifications from workspace state only (the actor, the pinned `now`, the issue or
//! page), and replaying the same commands produces the same inboxes on every client.
//!
//! Rules:
//! - **Assigned**: an issue is created with, or assigned to, someone other than the actor.
//! - **Mentioned**: `@id` of a known user (or a structured mention) newly appears in an issue
//!   description or comment, a page body or a page comment.
//! - **Commented**: a comment on an issue or page the recipient watches.
//! - **Status changed**: a watched issue moved to another status.
//! - **Resolved**: the recipient's reported issue entered a Done-category status (instead of the
//!   status-change notification).
//! - Auto-watch: reporters, assignees, commenters, page authors and editors.
//! - The actor is never notified about their own action; each event notifies a user at most once.
//! - Each user keeps the newest [`MAX_NOTIFICATIONS`]; the oldest are dropped.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, IssueKey, Timestamp};
use trackcraft_markup::Doc;

use crate::{ModelError, Result, Workspace, clip};

/// Notifications kept per user (oldest dropped first).
pub const MAX_NOTIFICATIONS: usize = 500;
/// Issues plus pages one user can watch.
pub const MAX_WATCHES: usize = 10_000;
/// Longest excerpt kept in a notification.
pub const NOTIFY_EXCERPT: usize = 140;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifyKind {
    Assigned,
    Mentioned,
    Commented,
    StatusChanged,
    Resolved,
}

impl NotifyKind {
    /// The wire name (`inbox.list`'s `kind`).
    pub fn name(self) -> &'static str {
        match self {
            NotifyKind::Assigned => "assigned",
            NotifyKind::Mentioned => "mentioned",
            NotifyKind::Commented => "commented",
            NotifyKind::StatusChanged => "statusChanged",
            NotifyKind::Resolved => "resolved",
        }
    }
}

/// What a notification is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Target {
    Issue(IssueKey),
    Page(Id),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// Unique within the workspace (the inbox's own counter, so workspace ids don't move).
    pub id: u64,
    pub kind: NotifyKind,
    pub target: Target,
    /// Who acted.
    pub actor: String,
    pub at: Timestamp,
    /// The issue's `KEY: summary` or the page's title, as it was then.
    pub title: String,
    /// A short excerpt: the comment, the new status…
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub excerpt: String,
    #[serde(default)]
    pub read: bool,
}

/// What one user watches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watches {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub issues: BTreeSet<IssueKey>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pages: BTreeSet<Id>,
}

impl Watches {
    fn len(&self) -> usize {
        self.issues.len().saturating_add(self.pages.len())
    }

    fn has(&self, t: &Target) -> bool {
        match t {
            Target::Issue(k) => self.issues.contains(k),
            Target::Page(id) => self.pages.contains(id),
        }
    }
}

/// Every user's notifications (oldest first) and watch lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inbox {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub notifications: BTreeMap<String, Vec<Notification>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub watches: BTreeMap<String, Watches>,
    /// The id the next notification gets.
    #[serde(default)]
    pub next_id: u64,
}

impl Inbox {
    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty() && self.watches.is_empty() && self.next_id == 0
    }

    /// A user's notifications, oldest first.
    pub fn of(&self, user: &str) -> &[Notification] {
        self.notifications.get(user).map_or(&[], Vec::as_slice)
    }

    pub fn unread(&self, user: &str) -> usize {
        self.of(user).iter().filter(|n| !n.read).count()
    }

    fn watching(&self, user: &str, t: &Target) -> bool {
        self.watches.get(user).is_some_and(|w| w.has(t))
    }

    /// Adds a watch; `false` when it was already there or the user is at [`MAX_WATCHES`].
    fn add_watch(&mut self, user: &str, t: &Target) -> bool {
        let w = self.watches.entry(user.to_string()).or_default();
        if w.len() >= MAX_WATCHES {
            return false;
        }
        match t {
            Target::Issue(k) => w.issues.insert(k.clone()),
            Target::Page(id) => w.pages.insert(*id),
        }
    }

    fn remove_watch(&mut self, user: &str, t: &Target) -> bool {
        let Some(w) = self.watches.get_mut(user) else { return false };
        let gone = match t {
            Target::Issue(k) => w.issues.remove(k),
            Target::Page(id) => w.pages.remove(id),
        };
        if w.len() == 0 {
            self.watches.remove(user);
        }
        gone
    }

    fn watchers(&self, t: &Target) -> Vec<String> {
        self.watches.iter().filter(|(_, w)| w.has(t)).map(|(u, _)| u.clone()).collect()
    }

    fn push(&mut self, user: &str, n: Notification) {
        let list = self.notifications.entry(user.to_string()).or_default();
        list.push(n);
        let excess = list.len().saturating_sub(MAX_NOTIFICATIONS);
        if excess > 0 {
            list.drain(..excess);
        }
    }
}

/// One event: who acted, on what, when, and how it reads.
struct Event<'a> {
    actor: &'a str,
    target: Target,
    at: Timestamp,
    title: String,
    excerpt: String,
}

/// A short single-line excerpt of rich text.
fn excerpt(d: &Doc) -> String {
    short(&d.plain_text())
}

fn short(s: &str) -> String {
    let text = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > NOTIFY_EXCERPT { format!("{}…", clip(&text, NOTIFY_EXCERPT)) } else { text }
}

impl Workspace {
    /// The current user ids a document mentions (structured or `@id` text).
    pub fn mentioned_in(&self, d: &Doc) -> Vec<String> {
        d.mentions(&|id| self.users.contains_key(id))
    }

    fn issue_event<'a>(&self, key: &IssueKey, actor: &'a str, at: Timestamp, excerpt: String) -> Option<Event<'a>> {
        let i = self.issues.get(key)?;
        Some(Event { actor, target: Target::Issue(key.clone()), at, title: format!("{}: {}", i.key, i.summary), excerpt })
    }

    fn page_event<'a>(&self, id: Id, actor: &'a str, at: Timestamp, excerpt: String) -> Option<Event<'a>> {
        let p = self.pages.get(&id)?;
        Some(Event { actor, target: Target::Page(id), at, title: p.title.clone(), excerpt })
    }

    /// Notifies each of `to` once (skipping the actor, unknown users and anyone in `done`), and
    /// adds them to `done`.
    fn send(&mut self, e: &Event<'_>, kind: NotifyKind, to: &[String], done: &mut BTreeSet<String>) {
        for u in to {
            if u == e.actor || !self.users.contains_key(u) || !done.insert(u.clone()) {
                continue;
            }
            let id = self.inbox.next_id;
            self.inbox.next_id = id.saturating_add(1);
            let n = Notification {
                id,
                kind,
                target: e.target.clone(),
                actor: clip(e.actor, 128),
                at: e.at,
                title: clip(&e.title, crate::MAX_TITLE.saturating_add(20)),
                excerpt: e.excerpt.clone(),
                read: false,
            };
            self.inbox.push(u, n);
        }
    }

    fn auto_watch(&mut self, user: Option<&str>, t: &Target) {
        if let Some(u) = user.filter(|u| self.users.contains_key(*u)) {
            self.inbox.add_watch(u, t);
        }
    }

    // ---- hooks called by the edit primitives (one line each) ----

    /// After `create_issue`: reporter and assignee watch; the assignee is told; mentions.
    pub(crate) fn notify_issue_created(&mut self, key: &IssueKey, now: Timestamp) {
        let Some(i) = self.issues.get(key) else { return };
        let (reporter, assignee, mentions) = (i.reporter.clone(), i.assignee.clone(), self.mentioned_in(&i.description));
        let t = Target::Issue(key.clone());
        self.auto_watch(Some(&reporter), &t);
        self.auto_watch(assignee.as_deref(), &t);
        let Some(e) = self.issue_event(key, &reporter, now, String::new()) else { return };
        let mut done = BTreeSet::new();
        self.send(&e, NotifyKind::Assigned, &assignee.into_iter().collect::<Vec<_>>(), &mut done);
        self.send(&e, NotifyKind::Mentioned, &mentions, &mut done);
    }

    /// After `assign` changed the assignee: they watch and are told.
    pub(crate) fn notify_assigned(&mut self, key: &IssueKey, actor: &str, now: Timestamp) {
        let Some(assignee) = self.issues.get(key).and_then(|i| i.assignee.clone()) else { return };
        self.auto_watch(Some(&assignee), &Target::Issue(key.clone()));
        if let Some(e) = self.issue_event(key, actor, now, String::new()) {
            self.send(&e, NotifyKind::Assigned, &[assignee], &mut BTreeSet::new());
        }
    }

    /// After `set_description`: users mentioned now but not in `had`.
    pub(crate) fn notify_description(&mut self, key: &IssueKey, had: &[String], actor: &str, now: Timestamp) {
        let Some(i) = self.issues.get(key) else { return };
        let new: Vec<String> = self.mentioned_in(&i.description).into_iter().filter(|m| !had.contains(m)).collect();
        let Some(e) = self.issue_event(key, actor, now, excerpt(&i.description)) else { return };
        self.send(&e, NotifyKind::Mentioned, &new, &mut BTreeSet::new());
    }

    /// After `transition`: the reporter hears "resolved" when it entered Done, other watchers
    /// "status changed".
    pub(crate) fn notify_transition(&mut self, key: &IssueKey, actor: &str, now: Timestamp, resolved: bool) {
        let Some(i) = self.issues.get(key) else { return };
        let (reporter, status) = (i.reporter.clone(), i.status.clone());
        let Some(e) = self.issue_event(key, actor, now, status) else { return };
        let mut done = BTreeSet::new();
        if resolved {
            self.send(&e, NotifyKind::Resolved, &[reporter], &mut done);
        }
        let watchers = self.inbox.watchers(&e.target);
        self.send(&e, NotifyKind::StatusChanged, &watchers, &mut done);
    }

    /// After `add_comment`: the commenter watches; mentioned users, then watchers, are told.
    pub(crate) fn notify_issue_comment(&mut self, key: &IssueKey, comment: Id, actor: &str, now: Timestamp) {
        let Some(c) = self.issues.get(key).and_then(|i| i.comments.iter().find(|c| c.id == comment)) else { return };
        let (mentions, text) = (self.mentioned_in(&c.body), excerpt(&c.body));
        let t = Target::Issue(key.clone());
        self.auto_watch(Some(actor), &t);
        let Some(e) = self.issue_event(key, actor, now, text) else { return };
        let mut done = BTreeSet::new();
        self.send(&e, NotifyKind::Mentioned, &mentions, &mut done);
        let watchers = self.inbox.watchers(&t);
        self.send(&e, NotifyKind::Commented, &watchers, &mut done);
    }

    /// After `create_page` / `edit_page`: the author watches; users mentioned now but not in
    /// the previous version are told.
    pub(crate) fn notify_page_saved(&mut self, id: Id, actor: &str, now: Timestamp) {
        let Some(p) = self.pages.get(&id) else { return };
        let had = p.history.last().map(|v| self.mentioned_in(&v.body)).unwrap_or_default();
        let new: Vec<String> = self.mentioned_in(&p.body).into_iter().filter(|m| !had.contains(m)).collect();
        let text = excerpt(&p.body);
        self.auto_watch(Some(actor), &Target::Page(id));
        if let Some(e) = self.page_event(id, actor, now, text) {
            self.send(&e, NotifyKind::Mentioned, &new, &mut BTreeSet::new());
        }
    }

    /// After `add_page_comment`: the commenter watches; mentioned users, then watchers.
    pub(crate) fn notify_page_comment(&mut self, page: Id, comment: Id, actor: &str, now: Timestamp) {
        let Some(c) = self.pages.get(&page).and_then(|p| p.comments.iter().find(|c| c.id == comment)) else { return };
        let (mentions, text) = (self.mentioned_in(&c.body), excerpt(&c.body));
        let t = Target::Page(page);
        self.auto_watch(Some(actor), &t);
        let Some(e) = self.page_event(page, actor, now, text) else { return };
        let mut done = BTreeSet::new();
        self.send(&e, NotifyKind::Mentioned, &mentions, &mut done);
        let watchers = self.inbox.watchers(&t);
        self.send(&e, NotifyKind::Commented, &watchers, &mut done);
    }

    /// After a delete: nobody watches what is gone (their notifications stay).
    pub(crate) fn forget_watched(&mut self, gone: &[Target]) {
        for w in self.inbox.watches.values_mut() {
            for t in gone {
                match t {
                    Target::Issue(k) => w.issues.remove(k),
                    Target::Page(id) => w.pages.remove(id),
                };
            }
        }
        self.inbox.watches.retain(|_, w| w.len() > 0);
    }

    // ---- commands ----

    fn target_exists(&self, t: &Target) -> Result<()> {
        match t {
            Target::Issue(k) => self.issue(&k.to_string()).map(|_| ()),
            Target::Page(id) => self.page(*id).map(|_| ()),
        }
    }

    /// `user` starts watching; `Ok(false)` if they already did.
    pub fn watch(&mut self, user: &str, t: &Target) -> Result<bool> {
        let user = self.user(user)?.id.clone();
        self.target_exists(t)?;
        if self.inbox.watching(&user, t) {
            return Ok(false);
        }
        if self.inbox.watches.get(&user).is_some_and(|w| w.len() >= MAX_WATCHES) {
            return Err(ModelError::Invalid(format!("a user can watch at most {MAX_WATCHES} issues and pages")));
        }
        Ok(self.inbox.add_watch(&user, t))
    }

    /// `user` stops watching; `Ok(false)` if they didn't.
    pub fn unwatch(&mut self, user: &str, t: &Target) -> Result<bool> {
        let user = self.user(user)?.id.clone();
        self.target_exists(t)?;
        Ok(self.inbox.remove_watch(&user, t))
    }

    pub fn is_watching(&self, user: &str, t: &Target) -> bool {
        self.inbox.watching(user, t)
    }

    /// Who watches an issue or page, by id.
    pub fn watchers(&self, t: &Target) -> Result<Vec<String>> {
        self.target_exists(t)?;
        Ok(self.inbox.watchers(t))
    }

    /// Marks `ids` (or, with `None`, all) of `user`'s notifications read. Unknown ids are
    /// ignored. Returns how many changed.
    pub fn mark_read(&mut self, user: &str, ids: Option<&[u64]>) -> usize {
        let Some(list) = self.inbox.notifications.get_mut(user) else { return 0 };
        let mut n = 0usize;
        for x in list.iter_mut().filter(|x| !x.read && ids.is_none_or(|ids| ids.contains(&x.id))) {
            x.read = true;
            n = n.saturating_add(1);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewIssue, NewPage, NewPageComment, ProjectKind};
    use chrono::{TimeZone, Utc};

    fn t(m: u32) -> Timestamp {
        Utc.with_ymd_and_hms(2026, 10, 31, 9, m, 0).single().unwrap()
    }

    fn ws() -> Workspace {
        let mut w = Workspace::new("Test");
        for u in ["ann", "bob", "cy"] {
            w.add_user(u, u).unwrap();
        }
        w.create_project("ATLAS", "Atlas", ProjectKind::Software, Some("ann")).unwrap();
        w.create_space("DOC", "Docs", "ann", t(0)).unwrap();
        w
    }

    fn kinds(w: &Workspace, u: &str) -> Vec<NotifyKind> {
        w.inbox.of(u).iter().map(|n| n.kind).collect()
    }

    fn issue(w: &mut Workspace, by: &str, assignee: Option<&str>, description: &str) -> IssueKey {
        let new = NewIssue {
            project: "ATLAS".into(),
            summary: "Fix it".into(),
            assignee: assignee.map(str::to_string),
            description: Doc::from_markdown(description),
            ..NewIssue::default()
        };
        w.create_issue(new, by, t(1)).unwrap()
    }

    #[test]
    fn assigned_on_create_and_on_assign_never_to_self() {
        let mut w = ws();
        let k = issue(&mut w, "ann", Some("bob"), "");
        assert_eq!(kinds(&w, "bob"), vec![NotifyKind::Assigned]);
        assert!(kinds(&w, "ann").is_empty());
        let n = &w.inbox.of("bob")[0];
        assert_eq!((n.actor.as_str(), n.title.as_str(), n.target.clone(), n.read), ("ann", "ATLAS-1: Fix it", Target::Issue(k.clone()), false));
        // Self-assignment is silent; reassigning tells the new assignee.
        w.assign("ATLAS-1", Some("ann"), "ann", t(2)).unwrap();
        assert!(kinds(&w, "ann").is_empty());
        w.assign("ATLAS-1", Some("cy"), "bob", t(3)).unwrap();
        assert_eq!(kinds(&w, "cy"), vec![NotifyKind::Assigned]);
        // Assigning to the same person again changes nothing.
        w.assign("ATLAS-1", Some("cy"), "bob", t(4)).unwrap();
        assert_eq!(kinds(&w, "cy").len(), 1);
        // Reporter and assignees watch.
        assert_eq!(w.watchers(&Target::Issue(k)).unwrap(), vec!["ann".to_string(), "bob".to_string(), "cy".to_string()]);
    }

    #[test]
    fn mentions_in_descriptions_and_comments() {
        let mut w = ws();
        issue(&mut w, "ann", None, "cc @bob and @ann and @ghost");
        assert_eq!(kinds(&w, "bob"), vec![NotifyKind::Mentioned]);
        assert!(kinds(&w, "ann").is_empty());
        // Editing the description only tells newly mentioned users.
        w.set_description("ATLAS-1", Doc::from_markdown("cc @bob, @cy"), "ann", t(2)).unwrap();
        assert_eq!(kinds(&w, "bob").len(), 1);
        assert_eq!(kinds(&w, "cy"), vec![NotifyKind::Mentioned]);
        // A comment mentioning a watcher tells them once (as a mention).
        w.add_comment("ATLAS-1", "cy", Doc::from_markdown("@ann look"), t(3)).unwrap();
        assert_eq!(kinds(&w, "ann"), vec![NotifyKind::Mentioned]);
        assert_eq!(w.inbox.of("ann")[0].excerpt, "@ann look");
    }

    #[test]
    fn comments_tell_watchers_and_auto_watch_the_commenter() {
        let mut w = ws();
        issue(&mut w, "ann", Some("bob"), "");
        w.add_comment("ATLAS-1", "cy", Doc::plain("hello"), t(2)).unwrap();
        assert_eq!(kinds(&w, "ann"), vec![NotifyKind::Commented]);
        assert_eq!(kinds(&w, "bob"), vec![NotifyKind::Assigned, NotifyKind::Commented]);
        assert!(kinds(&w, "cy").is_empty());
        w.add_comment("ATLAS-1", "ann", Doc::plain("thanks"), t(3)).unwrap();
        assert_eq!(kinds(&w, "cy"), vec![NotifyKind::Commented]);
        assert_eq!(kinds(&w, "ann").len(), 1);
    }

    #[test]
    fn status_changes_and_resolution() {
        let mut w = ws();
        issue(&mut w, "ann", Some("bob"), "");
        w.transition("ATLAS-1", "In Progress", "bob", t(2)).unwrap();
        assert_eq!(kinds(&w, "ann"), vec![NotifyKind::StatusChanged]);
        assert_eq!(w.inbox.of("ann")[0].excerpt, "In Progress");
        assert_eq!(kinds(&w, "bob"), vec![NotifyKind::Assigned]);
        w.transition("ATLAS-1", "Done", "bob", t(3)).unwrap();
        assert_eq!(kinds(&w, "ann"), vec![NotifyKind::StatusChanged, NotifyKind::Resolved]);
        // Resolving your own issue is silent for you; the assignee hears the status change.
        w.transition("ATLAS-1", "To Do", "bob", t(4)).unwrap();
        w.transition("ATLAS-1", "Done", "ann", t(5)).unwrap();
        assert_eq!(kinds(&w, "bob"), vec![NotifyKind::Assigned, NotifyKind::StatusChanged]);
        assert_eq!(kinds(&w, "ann").len(), 3);
    }

    #[test]
    fn unwatching_stops_comment_and_status_notifications() {
        let mut w = ws();
        let k = issue(&mut w, "ann", None, "");
        assert!(w.unwatch("ann", &Target::Issue(k.clone())).unwrap());
        assert!(!w.unwatch("ann", &Target::Issue(k.clone())).unwrap());
        w.add_comment("ATLAS-1", "bob", Doc::plain("x"), t(2)).unwrap();
        w.transition("ATLAS-1", "In Progress", "bob", t(3)).unwrap();
        assert!(kinds(&w, "ann").is_empty());
        assert!(w.watch("ann", &Target::Issue(k.clone())).unwrap());
        assert!(!w.watch("ann", &Target::Issue(k.clone())).unwrap());
        assert!(w.watch("ghost", &Target::Issue(k)).is_err());
        assert!(w.watch("ann", &Target::Page(Id(9999))).is_err());
        // Deleting forgets the watches.
        w.delete_issue("ATLAS-1", "ann", t(4)).unwrap();
        assert!(w.inbox.watches.values().all(|x| x.issues.is_empty()));
    }

    #[test]
    fn pages_authors_editors_comments_and_mentions() {
        let mut w = ws();
        let id = w
            .create_page(NewPage { space: "DOC".into(), parent: None, title: "Plan".into(), body: Doc::from_markdown("for @bob") }, "ann", t(1))
            .unwrap();
        assert_eq!(kinds(&w, "bob"), vec![NotifyKind::Mentioned]);
        assert_eq!(w.inbox.of("bob")[0].target, Target::Page(id));
        // An edit keeping @bob doesn't repeat; adding @cy tells cy; the editor watches.
        let edit = crate::PageEdit { body: Some(Doc::from_markdown("for @bob and @cy")), ..Default::default() };
        w.edit_page(id, edit, "bob", t(2)).unwrap();
        assert_eq!(kinds(&w, "bob").len(), 1);
        assert_eq!(kinds(&w, "cy"), vec![NotifyKind::Mentioned]);
        assert_eq!(w.watchers(&Target::Page(id)).unwrap(), vec!["ann".to_string(), "bob".to_string()]);
        w.add_page_comment(id, NewPageComment { body: Doc::plain("ok"), ..Default::default() }, "cy", t(3)).unwrap();
        assert_eq!(kinds(&w, "ann"), vec![NotifyKind::Commented]);
        assert_eq!(kinds(&w, "bob"), vec![NotifyKind::Mentioned, NotifyKind::Commented]);
        assert_eq!(kinds(&w, "cy").len(), 1);
        w.delete_page(id).unwrap();
        assert!(w.inbox.watches.values().all(|x| !x.pages.contains(&id)));
    }

    #[test]
    fn caps_and_mark_read() {
        let mut w = ws();
        issue(&mut w, "ann", None, "");
        for i in 0..(MAX_NOTIFICATIONS + 20) {
            w.add_comment("ATLAS-1", "bob", Doc::plain(&format!("c{i}")), t(2)).unwrap();
        }
        let list = w.inbox.of("ann");
        assert_eq!(list.len(), MAX_NOTIFICATIONS);
        assert_eq!(list[0].excerpt, "c20");
        assert_eq!(w.inbox.unread("ann"), MAX_NOTIFICATIONS);
        let first = list[0].id;
        assert_eq!(w.mark_read("ann", Some(&[first, 999_999])), 1);
        assert_eq!(w.mark_read("ann", Some(&[first])), 0);
        assert_eq!(w.mark_read("ann", None), MAX_NOTIFICATIONS - 1);
        assert_eq!(w.inbox.unread("ann"), 0);
        assert_eq!(w.mark_read("nobody", None), 0);
        let long = Doc::plain(&"word ".repeat(1000));
        w.add_comment("ATLAS-1", "bob", long, t(3)).unwrap();
        assert!(w.inbox.of("ann").last().unwrap().excerpt.chars().count() <= NOTIFY_EXCERPT + 1);
    }

    #[test]
    fn inbox_round_trips_and_old_files_load() {
        let mut w = ws();
        issue(&mut w, "ann", Some("bob"), "");
        let json = serde_json::to_string(&w).unwrap();
        let back: Workspace = serde_json::from_str(&json).unwrap();
        assert_eq!(back.inbox, w.inbox);
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("inbox");
        let old: Workspace = serde_json::from_value(v).unwrap();
        assert!(old.inbox.is_empty());
    }
}
