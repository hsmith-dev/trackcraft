//! Who may see and change what (M4.3): workspace roles, project and space roles with an access
//! level for everyone else, and page restrictions. The rules are summarised in
//! `docs/permissions.md`; the engine maps commands onto them (`trackcraft_engine::authz`).
//!
//! The concepts follow Atlassian's public descriptions of project roles and project access
//! levels, space permissions and page restrictions; the role names, levels and rules here are
//! our own simplification of them:
//!
//! - **Workspace role** (`Admin`, `Member`, `Viewer`): admins see and may do everything; a
//!   viewer is read-only everywhere (comments excepted); members get what their project and
//!   space roles allow. Users with no entry are admins — every user of a file written before
//!   permissions existed keeps full rights. `user.add` lists new users as members.
//! - **Project role** (`Administrator`, `Member`, `Viewer`) per user, and an **access level**
//!   for members without one: `open` (they are members), `limited` (viewers), `private` (no
//!   access; the project is hidden).
//! - **Space role** (`Admin`, `Editor`, `Viewer`) with the same three access levels.
//! - **Page restrictions**: a view list and an edit list of users. A view restriction applies
//!   to the page and every page below it; an edit restriction applies to that page only.
//!   Restrictions only narrow what the space role allows; users on a page's edit list may also
//!   view that page.
//!
//! Every check here is a pure function of the workspace, so a replica answers the same.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, ProjectKey};

use crate::{MAX_PAGE_DEPTH, ModelError, Result, Workspace, clip};

/// Most users on one page restriction list.
pub const MAX_RESTRICTION_USERS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceRole {
    Viewer,
    Member,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectRole {
    Viewer,
    Member,
    Administrator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceRole {
    Viewer,
    Editor,
    Admin,
}

/// What workspace members without a role of their own get in a project or space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessLevel {
    /// Members can browse and work (project member / space editor).
    #[default]
    Open,
    /// Members can browse and comment (viewer).
    Limited,
    /// Only users with a role (and workspace admins) see it.
    Private,
}

impl WorkspaceRole {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "admin" | "administrator" => WorkspaceRole::Admin,
            "member" | "user" => WorkspaceRole::Member,
            "viewer" => WorkspaceRole::Viewer,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            WorkspaceRole::Admin => "admin",
            WorkspaceRole::Member => "member",
            WorkspaceRole::Viewer => "viewer",
        }
    }
}

impl ProjectRole {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "administrator" | "admin" => ProjectRole::Administrator,
            "member" => ProjectRole::Member,
            "viewer" => ProjectRole::Viewer,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            ProjectRole::Administrator => "administrator",
            ProjectRole::Member => "member",
            ProjectRole::Viewer => "viewer",
        }
    }
}

impl SpaceRole {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "admin" | "administrator" => SpaceRole::Admin,
            "editor" => SpaceRole::Editor,
            "viewer" => SpaceRole::Viewer,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            SpaceRole::Admin => "admin",
            SpaceRole::Editor => "editor",
            SpaceRole::Viewer => "viewer",
        }
    }
}

impl AccessLevel {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "open" => AccessLevel::Open,
            "limited" => AccessLevel::Limited,
            "private" => AccessLevel::Private,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            AccessLevel::Open => "open",
            AccessLevel::Limited => "limited",
            AccessLevel::Private => "private",
        }
    }
    fn is_open(&self) -> bool {
        *self == AccessLevel::Open
    }
}

/// Roles in one project or space, and the level for everyone else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grants<R> {
    #[serde(default, skip_serializing_if = "AccessLevel::is_open")]
    pub access: AccessLevel,
    #[serde(default = "BTreeMap::new", skip_serializing_if = "BTreeMap::is_empty")]
    pub roles: BTreeMap<String, R>,
}

impl<R> Default for Grants<R> {
    fn default() -> Self {
        Grants { access: AccessLevel::Open, roles: BTreeMap::new() }
    }
}

impl<R> Grants<R> {
    fn is_default(&self) -> bool {
        self.access.is_open() && self.roles.is_empty()
    }
}

/// A page's restrictions; `None` = unrestricted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Restriction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<Vec<String>>,
}

impl Restriction {
    fn lists(&self, user: &str) -> (bool, bool) {
        let on = |l: &Option<Vec<String>>| l.as_ref().is_some_and(|l| l.iter().any(|u| u == user));
        (on(&self.view), on(&self.edit))
    }
}

/// Everything about access in a workspace (one field of [`Workspace`]; empty in older files).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Access {
    /// Workspace roles; a user without an entry is an admin (see the module docs).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub users: BTreeMap<String, WorkspaceRole>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<ProjectKey, Grants<ProjectRole>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub spaces: BTreeMap<ProjectKey, Grants<SpaceRole>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pages: BTreeMap<Id, Restriction>,
}

impl Access {
    pub fn is_empty(&self) -> bool {
        self.users.is_empty() && self.projects.is_empty() && self.spaces.is_empty() && self.pages.is_empty()
    }
}

fn base<R: Copy>(level: AccessLevel, open: R, limited: R) -> Option<R> {
    match level {
        AccessLevel::Open => Some(open),
        AccessLevel::Limited => Some(limited),
        AccessLevel::Private => None,
    }
}

impl Workspace {
    /// The user's workspace role; `None` = not a member.
    pub fn workspace_role(&self, user: &str) -> Option<WorkspaceRole> {
        if !self.users.contains_key(user) {
            return None;
        }
        Some(self.access.users.get(user).copied().unwrap_or(WorkspaceRole::Admin))
    }

    /// How many users are workspace admins.
    pub fn admin_count(&self) -> usize {
        self.users.keys().filter(|u| self.workspace_role(u) == Some(WorkspaceRole::Admin)).count()
    }

    /// The user's effective role in a project; `None` = can't browse it (or no such project).
    pub fn project_role(&self, user: &str, key: &ProjectKey) -> Option<ProjectRole> {
        let wr = self.workspace_role(user)?;
        if !self.projects.contains_key(key) {
            return None;
        }
        if wr == WorkspaceRole::Admin {
            return Some(ProjectRole::Administrator);
        }
        let g = self.access.projects.get(key);
        let role = match g.and_then(|g| g.roles.get(user)) {
            Some(r) => *r,
            None => base(g.map(|g| g.access).unwrap_or_default(), ProjectRole::Member, ProjectRole::Viewer)?,
        };
        Some(if wr == WorkspaceRole::Viewer { role.min(ProjectRole::Viewer) } else { role })
    }

    /// The user's effective role in a space; `None` = can't view it (or no such space).
    pub fn space_role(&self, user: &str, key: &ProjectKey) -> Option<SpaceRole> {
        let wr = self.workspace_role(user)?;
        if !self.spaces.contains_key(key) {
            return None;
        }
        if wr == WorkspaceRole::Admin {
            return Some(SpaceRole::Admin);
        }
        let g = self.access.spaces.get(key);
        let role = match g.and_then(|g| g.roles.get(user)) {
            Some(r) => *r,
            None => base(g.map(|g| g.access).unwrap_or_default(), SpaceRole::Editor, SpaceRole::Viewer)?,
        };
        Some(if wr == WorkspaceRole::Viewer { role.min(SpaceRole::Viewer) } else { role })
    }

    pub fn can_browse_project(&self, user: &str, key: &ProjectKey) -> bool {
        self.project_role(user, key).is_some()
    }

    /// Whether the user may view a page: they can view its space, and every view restriction on
    /// the page and its ancestors lists them (an edit list counts for its own page).
    pub fn page_visible(&self, user: &str, id: Id) -> bool {
        let Some(page) = self.pages.get(&id) else { return false };
        let Some(_) = self.space_role(user, &page.space) else { return false };
        if self.workspace_role(user) == Some(WorkspaceRole::Admin) {
            return true;
        }
        let mut cur = Some(id);
        for _ in 0..=MAX_PAGE_DEPTH.saturating_add(1) {
            let Some(c) = cur else { return true };
            if let Some(r) = self.access.pages.get(&c)
                && r.view.is_some()
            {
                let (v, e) = r.lists(user);
                if !(v || e) {
                    return false;
                }
            }
            cur = self.pages.get(&c).and_then(|p| p.parent);
        }
        // Deeper than the model allows: a corrupt tree is treated as hidden.
        false
    }

    /// Whether the user may edit a page: they can view it, are an editor of its space, and the
    /// page's own edit restriction (if any) lists them.
    pub fn page_editable(&self, user: &str, id: Id) -> bool {
        if !self.page_visible(user, id) {
            return false;
        }
        if self.workspace_role(user) == Some(WorkspaceRole::Admin) {
            return true;
        }
        let Some(page) = self.pages.get(&id) else { return false };
        if self.space_role(user, &page.space).is_none_or(|r| r < SpaceRole::Editor) {
            return false;
        }
        match self.access.pages.get(&id).and_then(|r| r.edit.as_ref()) {
            Some(list) => list.iter().any(|u| u == user),
            None => true,
        }
    }

    /// Sets a workspace role. Refuses to leave the workspace without an admin.
    pub fn set_workspace_role(&mut self, user: &str, role: WorkspaceRole) -> Result<()> {
        let cur = self.workspace_role(user).ok_or_else(|| ModelError::NoSuchUser(clip(user, 128)))?;
        if cur == WorkspaceRole::Admin && role != WorkspaceRole::Admin && self.admin_count() <= 1 {
            return Err(ModelError::Invalid(format!("{} is the last workspace admin: make someone else an admin first", clip(user, 128))));
        }
        self.access.users.insert(user.to_string(), role);
        Ok(())
    }

    fn project_key(&self, key: &str) -> Result<ProjectKey> {
        Ok(self.project(key)?.key.clone())
    }

    fn space_key(&self, key: &str) -> Result<ProjectKey> {
        Ok(self.space(key)?.key.clone())
    }

    /// Gives a user a role in a project, or with `None` removes it (back to the access level).
    pub fn set_project_role(&mut self, project: &str, user: &str, role: Option<ProjectRole>) -> Result<()> {
        let key = self.project_key(project)?;
        self.user(user)?;
        let g = self.access.projects.entry(key.clone()).or_default();
        match role {
            Some(r) => {
                g.roles.insert(user.to_string(), r);
            }
            None => {
                g.roles.remove(user);
            }
        }
        if g.is_default() {
            self.access.projects.remove(&key);
        }
        Ok(())
    }

    pub fn set_project_access(&mut self, project: &str, level: AccessLevel) -> Result<()> {
        let key = self.project_key(project)?;
        let g = self.access.projects.entry(key.clone()).or_default();
        g.access = level;
        if g.is_default() {
            self.access.projects.remove(&key);
        }
        Ok(())
    }

    pub fn set_space_role(&mut self, space: &str, user: &str, role: Option<SpaceRole>) -> Result<()> {
        let key = self.space_key(space)?;
        self.user(user)?;
        let g = self.access.spaces.entry(key.clone()).or_default();
        match role {
            Some(r) => {
                g.roles.insert(user.to_string(), r);
            }
            None => {
                g.roles.remove(user);
            }
        }
        if g.is_default() {
            self.access.spaces.remove(&key);
        }
        Ok(())
    }

    pub fn set_space_access(&mut self, space: &str, level: AccessLevel) -> Result<()> {
        let key = self.space_key(space)?;
        let g = self.access.spaces.entry(key.clone()).or_default();
        g.access = level;
        if g.is_default() {
            self.access.spaces.remove(&key);
        }
        Ok(())
    }

    /// Changes a page's restrictions. For each list: `None` = unchanged, `Some(None)` = remove
    /// the restriction, `Some(Some(users))` = only those users (each a workspace member; at most
    /// [`MAX_RESTRICTION_USERS`]; duplicates dropped).
    pub fn restrict_page(&mut self, id: Id, view: Option<Option<Vec<String>>>, edit: Option<Option<Vec<String>>>) -> Result<()> {
        self.page(id)?;
        let check = |list: Option<Option<Vec<String>>>| -> Result<Option<Option<Vec<String>>>> {
            let Some(Some(users)) = list else { return Ok(list) };
            if users.len() > MAX_RESTRICTION_USERS {
                return Err(ModelError::Invalid(format!("a restriction lists at most {MAX_RESTRICTION_USERS} users")));
            }
            let mut seen = BTreeSet::new();
            let mut out = Vec::new();
            for u in users {
                let u = u.trim().to_string();
                self.user(&u).map_err(|_| ModelError::NoSuchUser(clip(&u, 128)))?;
                if seen.insert(u.clone()) {
                    out.push(u);
                }
            }
            Ok(Some(Some(out)))
        };
        let (view, edit) = (check(view)?, check(edit)?);
        let r = self.access.pages.entry(id).or_default();
        if let Some(v) = view {
            r.view = v;
        }
        if let Some(e) = edit {
            r.edit = e;
        }
        if r.view.is_none() && r.edit.is_none() {
            self.access.pages.remove(&id);
        }
        Ok(())
    }

    /// The workspace as `user` may see it: projects they can't browse (with their issues and
    /// sprints), spaces they can't view and pages hidden by restrictions are removed, and so are
    /// links to removed issues, templates of removed spaces and access entries about removed
    /// things. `None` when nothing is hidden from them (no copy needed). A non-member sees no
    /// projects or spaces.
    ///
    /// Fields not listed here pass through unchanged: a new workspace field that can refer to
    /// hidden things must be filtered here too (`docs/permissions.md`).
    pub fn view_for(&self, user: &str) -> Option<Workspace> {
        if self.workspace_role(user) == Some(WorkspaceRole::Admin) {
            return None;
        }
        let hidden_projects: BTreeSet<ProjectKey> = self.projects.keys().filter(|k| !self.can_browse_project(user, k)).cloned().collect();
        let hidden_spaces: BTreeSet<ProjectKey> = self.spaces.keys().filter(|k| self.space_role(user, k).is_none()).cloned().collect();
        let hidden_pages: BTreeSet<Id> = self.pages.keys().filter(|id| !self.page_visible(user, **id)).copied().collect();
        if hidden_projects.is_empty() && hidden_spaces.is_empty() && hidden_pages.is_empty() {
            return None;
        }
        let mut w = self.clone();
        w.projects.retain(|k, _| !hidden_projects.contains(k));
        w.issues.retain(|k, _| !hidden_projects.contains(&k.project));
        for i in w.issues.values_mut() {
            i.links.retain(|l| !hidden_projects.contains(&l.target.project));
        }
        w.sprints.retain(|_, s| !hidden_projects.contains(&s.project));
        w.spaces.retain(|k, _| !hidden_spaces.contains(k));
        w.pages.retain(|id, _| !hidden_pages.contains(id));
        w.templates.retain(|_, t| t.space.as_ref().is_none_or(|k| !hidden_spaces.contains(k)));
        w.access.projects.retain(|k, _| !hidden_projects.contains(k));
        w.access.spaces.retain(|k, _| !hidden_spaces.contains(k));
        w.access.pages.retain(|id, _| w.pages.contains_key(id));
        // M4.5 inbox: notifications and watches about hidden issues or pages would leak their
        // titles and excerpts. Other users' entries about visible things stay (watcher lists
        // are visible anyway), so replaying visible commands keeps the copy identical.
        let visible = |t: &crate::notify::Target| match t {
            crate::notify::Target::Issue(k) => !hidden_projects.contains(&k.project) && w.issues.contains_key(k),
            crate::notify::Target::Page(id) => !hidden_pages.contains(id) && w.pages.contains_key(id),
        };
        let mut inbox = std::mem::take(&mut w.inbox);
        for list in inbox.notifications.values_mut() {
            list.retain(|n| visible(&n.target));
        }
        for watches in inbox.watches.values_mut() {
            watches.issues.retain(|k| !hidden_projects.contains(&k.project));
            watches.pages.retain(|id| !hidden_pages.contains(id));
        }
        w.inbox = inbox;
        Some(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewPage, ProjectKind};

    fn ws() -> Workspace {
        let mut w = Workspace::new("t");
        for u in ["ann", "bob", "cy"] {
            w.add_user(u, u).unwrap();
        }
        w.create_project("P", "P", ProjectKind::Software, None).unwrap();
        let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
        w.create_space("S", "S", "ann", now).unwrap();
        w
    }

    fn pk(k: &str) -> ProjectKey {
        ProjectKey::parse(k).unwrap()
    }

    #[test]
    fn unlisted_users_are_admins_and_last_admin_is_kept() {
        let mut w = ws();
        assert_eq!(w.workspace_role("ann"), Some(WorkspaceRole::Admin));
        assert_eq!(w.workspace_role("nobody"), None);
        w.set_workspace_role("bob", WorkspaceRole::Member).unwrap();
        w.set_workspace_role("cy", WorkspaceRole::Viewer).unwrap();
        assert_eq!(w.admin_count(), 1);
        assert!(w.set_workspace_role("ann", WorkspaceRole::Member).is_err());
        assert!(w.set_workspace_role("ghost", WorkspaceRole::Admin).is_err());
        assert_eq!(w.workspace_role("ann"), Some(WorkspaceRole::Admin));
    }

    #[test]
    fn levels_roles_and_viewer_cap() {
        let mut w = ws();
        w.set_workspace_role("bob", WorkspaceRole::Member).unwrap();
        w.set_workspace_role("cy", WorkspaceRole::Viewer).unwrap();
        assert_eq!(w.project_role("bob", &pk("P")), Some(ProjectRole::Member));
        assert_eq!(w.project_role("cy", &pk("P")), Some(ProjectRole::Viewer));
        w.set_project_access("P", AccessLevel::Private).unwrap();
        assert_eq!(w.project_role("bob", &pk("P")), None);
        w.set_project_role("p", "cy", Some(ProjectRole::Administrator)).unwrap();
        assert_eq!(w.project_role("cy", &pk("P")), Some(ProjectRole::Viewer), "workspace viewers stay read-only");
        w.set_space_access("S", AccessLevel::Limited).unwrap();
        assert_eq!(w.space_role("bob", &pk("S")), Some(SpaceRole::Viewer));
        w.set_space_role("S", "bob", Some(SpaceRole::Admin)).unwrap();
        assert_eq!(w.space_role("bob", &pk("S")), Some(SpaceRole::Admin));
        w.set_space_role("S", "bob", None).unwrap();
        w.set_space_access("S", AccessLevel::Open).unwrap();
        assert!(!w.access.spaces.contains_key(&pk("S")), "default grants are not stored");
    }

    #[test]
    fn view_restrictions_inherit_and_edit_ones_do_not() {
        let mut w = ws();
        w.set_workspace_role("bob", WorkspaceRole::Member).unwrap();
        w.set_workspace_role("cy", WorkspaceRole::Member).unwrap();
        let now = chrono::DateTime::from_timestamp(0, 0).unwrap();
        let parent = w.create_page(NewPage { space: "S".into(), title: "A".into(), ..Default::default() }, "ann", now).unwrap();
        let child = w.create_page(NewPage { space: "S".into(), parent: Some(parent), title: "B".into(), ..Default::default() }, "ann", now).unwrap();
        w.restrict_page(parent, Some(Some(vec!["bob".into(), "bob".into()])), None).unwrap();
        assert_eq!(w.access.pages.get(&parent).and_then(|r| r.view.clone()), Some(vec!["bob".to_string()]));
        assert!(w.page_visible("bob", child) && !w.page_visible("cy", child) && w.page_visible("ann", child));
        w.restrict_page(parent, Some(None), Some(Some(vec!["bob".into()]))).unwrap();
        assert!(w.page_visible("cy", parent) && !w.page_editable("cy", parent) && w.page_editable("cy", child));
        assert!(w.restrict_page(parent, Some(Some(vec!["ghost".into()])), None).is_err());
        let view = w.view_for("cy").map(|v| v.pages.len());
        assert_eq!(view, None, "nothing hidden: no copy");
        w.restrict_page(child, Some(Some(vec![])), None).unwrap();
        let v = w.view_for("cy").unwrap();
        assert!(v.pages.contains_key(&parent) && !v.pages.contains_key(&child) && !v.access.pages.contains_key(&child));
    }

    #[test]
    fn serde_round_trip_and_empty_default() {
        let mut w = ws();
        w.set_workspace_role("bob", WorkspaceRole::Viewer).unwrap();
        w.set_project_access("P", AccessLevel::Limited).unwrap();
        let text = serde_json::to_string(&w.access).unwrap();
        let back: Access = serde_json::from_str(&text).unwrap();
        assert_eq!(back, w.access);
        let empty: Access = serde_json::from_str("{}").unwrap();
        assert!(empty.is_empty());
    }
}
