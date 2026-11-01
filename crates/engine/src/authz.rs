//! Permission checks (M4.3): which permission each command needs, on which project, space or
//! page (read from its params), and what a sync stream may tell a user about a journal entry.
//!
//! Rules (`docs/permissions.md`; data and predicates in `trackcraft_model::access`):
//!
//! - Something the actor can't browse is reported exactly as if it didn't exist (the model's
//!   own "no issue X" / "no page with id N" text), so a REST client gets 404 and learns nothing.
//! - Something they can browse but not change is [`Denied::Forbidden`] ("permission denied: …",
//!   403 over REST).
//! - Missing targets pass: the command reports them itself.
//! - Queries are never refused; under enforcement [`Session::run`] runs them on the actor's
//!   [`Workspace::view_for`], so lists and searches leave out what they can't see.
//! - A mutating command without a rule here needs a workspace admin (a test keeps every
//!   command listed).

use serde_json::Value;
use trackcraft_core::{Id, IssueKey, ProjectKey};
use trackcraft_model::{Issue, ModelError, ProjectRole, SpaceRole, Workspace, WorkspaceRole};

use crate::{LOCAL_ONLY, Session, params};

/// The prefix of every [`Denied::Forbidden`] message (the REST layer maps it to 403).
pub const DENIED: &str = "permission denied";

/// Why a command was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    /// The actor can't browse the target: the message is the "doesn't exist" one.
    Hidden(String),
    /// The actor can see the target but lacks the permission.
    Forbidden(String),
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denied::Hidden(m) => f.write_str(m),
            Denied::Forbidden(m) => write!(f, "{DENIED}: {m}"),
        }
    }
}
impl std::error::Error for Denied {}

/// Commands with a rule in [`Session::authorize`] (mutating ones; queries need none).
pub const RULED: &[&str] = &[
    "workspace.rename",
    "user.add",
    "user.edit",
    "project.create",
    "space.create",
    "issue.create",
    "issue.edit",
    "issue.assign",
    "issue.assignToMe",
    "issue.transition",
    "issue.comment",
    "issue.editComment",
    "issue.deleteComment",
    "issue.link",
    "issue.unlink",
    "issue.delete",
    "issue.rank",
    "issue.setSprint",
    "sprint.create",
    "sprint.edit",
    "sprint.delete",
    "sprint.start",
    "sprint.complete",
    "page.create",
    "page.edit",
    "page.labels",
    "page.restore",
    "page.move",
    "page.copy",
    "page.delete",
    "page.comment",
    "page.editComment",
    "page.resolveComment",
    "page.deleteComment",
    "page.restrict",
    "template.create",
    "template.delete",
    "access.setWorkspaceRole",
    "access.setProjectRole",
    "access.setProjectAccess",
    "access.setSpaceRole",
    "access.setSpaceAccess",
    // M4.5 notifications (rules added when merging with M4.3)
    "inbox.markRead",
    "issue.watch",
    "issue.unwatch",
    "page.watch",
    "page.unwatch",
    // M2.4 custom fields, components, versions: project administrators.
    "field.create",
    "field.edit",
    "field.delete",
    "component.create",
    "component.edit",
    "component.delete",
    "version.create",
    "version.edit",
    "version.delete",
    "version.release",
];

/// M2.4: the project owning the field, component or version that `command`'s `id` names.
fn definition_project(ws: &Workspace, command: &str, p: &Value) -> Option<ProjectKey> {
    let id = params::opt_id(p, "id").ok().flatten()?;
    match command.split('.').next() {
        Some("field") => ws.find_field(id).map(|(k, _)| k.clone()),
        Some("component") => ws.find_component(id).map(|(k, _)| k.clone()),
        Some("version") => ws.find_version(id).map(|(k, _)| k.clone()),
        _ => None,
    }
}

/// What the actor needs on an issue.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OnIssue {
    /// Browse (and comment: viewers may comment).
    Browse,
    /// Edit fields, transition, link: project members, and the reporter or assignee when
    /// they are a project viewer but a workspace member.
    Edit,
    /// Assign, rank, sprints, delete issues: project members (as in a team-managed project;
    /// deleting other people's comments needs a project administrator, checked separately).
    Work,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OnPage {
    /// View (and comment).
    View,
    Edit,
}

fn hidden(e: ModelError) -> Denied {
    Denied::Hidden(e.to_string())
}

fn short(s: &str) -> String {
    s.chars().take(80).collect()
}

struct Check<'a> {
    ws: &'a Workspace,
    actor: &'a str,
    role: WorkspaceRole,
}

impl Check<'_> {
    fn admin(&self, what: &str) -> Result<(), Denied> {
        if self.role == WorkspaceRole::Admin { Ok(()) } else { Err(Denied::Forbidden(format!("{what} needs a workspace admin"))) }
    }

    fn member(&self, what: &str) -> Result<(), Denied> {
        if self.role >= WorkspaceRole::Member { Ok(()) } else { Err(Denied::Forbidden(format!("workspace viewers can't {what}"))) }
    }

    fn issue_role(&self, i: &Issue) -> Result<ProjectRole, Denied> {
        self.ws.project_role(self.actor, &i.key.project).ok_or_else(|| hidden(ModelError::NoSuchIssue(i.key.to_string())))
    }

    /// The issue named by param `k` (a missing or malformed one passes: the command reports it).
    fn issue_key(&self, raw: &str, need: OnIssue) -> Result<(), Denied> {
        let Ok(i) = self.ws.issue(raw) else { return Ok(()) };
        let role = self.ws.project_role(self.actor, &i.key.project).ok_or_else(|| hidden(ModelError::NoSuchIssue(raw.to_string())))?;
        let own = self.role >= WorkspaceRole::Member && (i.reporter == self.actor || i.assignee.as_deref() == Some(self.actor));
        let ok = match need {
            OnIssue::Browse => true,
            OnIssue::Edit => role >= ProjectRole::Member || own,
            OnIssue::Work => role >= ProjectRole::Member,
        };
        if ok {
            return Ok(());
        }
        let what = match need {
            OnIssue::Browse => "browse",
            OnIssue::Edit => "edit",
            OnIssue::Work => "assign, rank, plan or delete",
        };
        Err(Denied::Forbidden(format!("you can't {what} issues in project {} (your role: {})", i.key.project, role.name())))
    }

    fn issue(&self, p: &Value, k: &str, need: OnIssue) -> Result<(), Denied> {
        match p.get(k) {
            Some(Value::String(raw)) => self.issue_key(raw, need),
            _ => Ok(()),
        }
    }

    fn issue_keys(&self, p: &Value, k: &str, need: OnIssue) -> Result<(), Denied> {
        for key in params::keys(p, k).unwrap_or_default() {
            self.issue_key(&key, need)?;
        }
        Ok(())
    }

    fn project(&self, raw: &str, need: ProjectRole) -> Result<(), Denied> {
        let Ok(proj) = self.ws.project(raw) else { return Ok(()) };
        let role = self.ws.project_role(self.actor, &proj.key).ok_or_else(|| hidden(ModelError::NoSuchProject(raw.to_string())))?;
        if role >= need {
            return Ok(());
        }
        let what = match need {
            ProjectRole::Administrator => "administer",
            _ => "create or plan issues in",
        };
        Err(Denied::Forbidden(format!("you can't {what} project {} (your role: {})", proj.key, role.name())))
    }

    fn project_param(&self, p: &Value, k: &str, need: ProjectRole) -> Result<(), Denied> {
        match p.get(k) {
            Some(Value::String(raw)) => self.project(raw, need),
            _ => Ok(()),
        }
    }

    fn sprint(&self, id: Id, need: ProjectRole) -> Result<(), Denied> {
        let Some(sp) = self.ws.sprints.get(&id) else { return Ok(()) };
        if !self.ws.can_browse_project(self.actor, &sp.project) {
            return Err(Denied::Hidden(format!("no sprint with id {id}")));
        }
        self.project(sp.project.as_str(), need)
    }

    fn sprint_param(&self, p: &Value, k: &str, need: ProjectRole) -> Result<(), Denied> {
        match params::opt_id(p, k) {
            Ok(Some(id)) => self.sprint(id, need),
            _ => Ok(()),
        }
    }

    fn space(&self, raw: &str, need: SpaceRole) -> Result<(), Denied> {
        let Ok(sp) = self.ws.space(raw) else { return Ok(()) };
        let role = self.ws.space_role(self.actor, &sp.key).ok_or_else(|| hidden(ModelError::NoSuchSpace(raw.to_string())))?;
        if role >= need {
            return Ok(());
        }
        let what = if need == SpaceRole::Admin { "administer" } else { "add pages to" };
        Err(Denied::Forbidden(format!("you can't {what} space {} (your role: {})", sp.key, role.name())))
    }

    fn space_param(&self, p: &Value, k: &str, need: SpaceRole) -> Result<(), Denied> {
        match p.get(k) {
            Some(Value::String(raw)) => self.space(raw, need),
            _ => Ok(()),
        }
    }

    fn page(&self, id: Id, need: OnPage) -> Result<(), Denied> {
        if !self.ws.pages.contains_key(&id) {
            return Ok(());
        }
        if !self.ws.page_visible(self.actor, id) {
            return Err(hidden(ModelError::NoSuchPage(id)));
        }
        if need == OnPage::Edit && !self.ws.page_editable(self.actor, id) {
            return Err(Denied::Forbidden(format!("you can't edit page {id}")));
        }
        Ok(())
    }

    fn page_param(&self, p: &Value, k: &str, need: OnPage) -> Result<(), Denied> {
        match params::opt_id(p, k) {
            Ok(Some(id)) => self.page(id, need),
            _ => Ok(()),
        }
    }

    /// A page that becomes a parent or sibling anchor: viewable, in a space the actor may add to.
    fn destination(&self, p: &Value, k: &str) -> Result<(), Denied> {
        let Ok(Some(id)) = params::opt_id(p, k) else { return Ok(()) };
        self.page(id, OnPage::View)?;
        match self.ws.pages.get(&id) {
            Some(pg) => self.space(pg.space.as_str(), SpaceRole::Editor),
            None => Ok(()),
        }
    }

    /// Every page in the subtree of `id` meets `need` (moves, copies and deletes take children
    /// with them).
    fn subtree(&self, id: Id, need: OnPage, what: &str) -> Result<(), Denied> {
        let ok = |c: &Id| match need {
            OnPage::View => self.ws.page_visible(self.actor, *c),
            OnPage::Edit => self.ws.page_editable(self.actor, *c),
        };
        if self.ws.pages.contains_key(&id) && !self.ws.subtree(id).iter().all(ok) {
            return Err(Denied::Forbidden(format!("page {id} has child pages you can't {what}")));
        }
        Ok(())
    }

    /// M2.4 `version.release`: the version unresolved issues move to must be browsable.
    fn release_target(&self, p: &Value) -> Result<(), Denied> {
        let Ok(Some(to)) = params::opt_id(p, "moveUnresolvedTo") else { return Ok(()) };
        match self.ws.find_version(to) {
            Some((k, _)) if !self.ws.can_browse_project(self.actor, k) => Err(Denied::Hidden(format!("no version with id {to}"))),
            _ => Ok(()),
        }
    }

    /// A page restriction list must keep the actor on it, unless they administer the space.
    fn keeps_self(&self, p: &Value, space_admin: bool) -> Result<(), Denied> {
        if space_admin {
            return Ok(());
        }
        for k in ["view", "edit"] {
            if let Some(Value::Array(list)) = p.get(k)
                && !list.iter().any(|u| u.as_str() == Some(self.actor))
            {
                return Err(Denied::Forbidden(format!(
                    "include yourself in the \"{k}\" list (only space admins can restrict a page so they lose access)"
                )));
            }
        }
        Ok(())
    }
}

impl Session {
    /// Server mode: check every local command against the acting user's permissions and run
    /// queries on their view of the workspace. Off by default (a desktop file is single-user);
    /// replays ([`Session::apply_remote`]) are never checked — the server already did.
    pub fn set_enforce(&mut self, on: bool) {
        self.enforce = on;
    }

    pub fn is_enforcing(&self) -> bool {
        self.enforce
    }

    /// Whether `actor` may run `command` with `params` (see the module docs). Queries and
    /// [`LOCAL_ONLY`] commands always pass.
    pub fn authorize(&self, actor: &str, command: &str, params: &Value) -> Result<(), Denied> {
        authorize(&self.ws, actor, command, params)
    }
}

/// [`Session::authorize`] on any workspace.
pub fn authorize(ws: &Workspace, actor: &str, command: &str, p: &Value) -> Result<(), Denied> {
    let Some(spec) = crate::command(command) else { return Ok(()) };
    if !spec.mutates || LOCAL_ONLY.contains(&spec.id) {
        return Ok(());
    }
    let role = ws.workspace_role(actor).ok_or_else(|| Denied::Forbidden(format!("{} is not a member of this workspace", short(actor))))?;
    let c = Check { ws, actor, role };
    match spec.id {
        "workspace.rename" | "user.add" | "access.setWorkspaceRole" => c.admin(spec.id),
        // Marks only the actor's own notifications; watching needs only sight of the target.
        "inbox.markRead" => Ok(()),
        "issue.watch" | "issue.unwatch" => c.issue(p, "key", OnIssue::Browse),
        "page.watch" | "page.unwatch" => c.page_param(p, "id", OnPage::View),
        "user.edit" => match p.get("id") {
            Some(Value::String(id)) if id == actor => Ok(()),
            _ => c.admin("editing another user's profile"),
        },
        "project.create" => c.member("create projects"),
        "space.create" => c.member("create spaces"),
        "issue.create" => {
            c.project_param(p, "project", ProjectRole::Member)?;
            c.issue(p, "parent", OnIssue::Browse)?;
            c.sprint_param(p, "sprint", ProjectRole::Member)
        }
        "issue.edit" => {
            c.issue(p, "key", OnIssue::Edit)?;
            c.issue(p, "parent", OnIssue::Browse)
        }
        "issue.transition" => c.issue(p, "key", OnIssue::Edit),
        "issue.assign" | "issue.assignToMe" => c.issue(p, "key", OnIssue::Work),
        "issue.comment" | "issue.editComment" => c.issue(p, "key", OnIssue::Browse),
        "issue.deleteComment" => {
            c.issue(p, "key", OnIssue::Browse)?;
            let own = match (p.get("key").and_then(Value::as_str).and_then(|k| ws.issue(k).ok()), params::opt_id(p, "id")) {
                (Some(i), Ok(Some(id))) => i.comments.iter().find(|cm| cm.id == id).is_none_or(|cm| cm.author == actor),
                _ => true,
            };
            if own {
                Ok(())
            } else {
                let i = p.get("key").and_then(Value::as_str).and_then(|k| ws.issue(k).ok());
                match i {
                    Some(i) if c.issue_role(i)? >= ProjectRole::Administrator => Ok(()),
                    _ => Err(Denied::Forbidden("only project administrators can delete other people's comments".into())),
                }
            }
        }
        "issue.link" | "issue.unlink" => {
            c.issue(p, "key", OnIssue::Edit)?;
            c.issue(p, "target", OnIssue::Browse)
        }
        "issue.delete" => c.issue(p, "key", OnIssue::Work),
        "issue.rank" => {
            c.issue_keys(p, "keys", OnIssue::Work)?;
            c.issue(p, "before", OnIssue::Browse)?;
            c.issue(p, "after", OnIssue::Browse)
        }
        "issue.setSprint" => {
            c.issue_keys(p, "keys", OnIssue::Work)?;
            c.sprint_param(p, "sprint", ProjectRole::Member)?;
            c.issue(p, "before", OnIssue::Browse)?;
            c.issue(p, "after", OnIssue::Browse)
        }
        "sprint.create" => c.project_param(p, "project", ProjectRole::Member),
        "sprint.edit" | "sprint.delete" | "sprint.start" => c.sprint_param(p, "id", ProjectRole::Member),
        "sprint.complete" => {
            c.sprint_param(p, "id", ProjectRole::Member)?;
            c.sprint_param(p, "moveOpenTo", ProjectRole::Member)
        }
        "page.create" => {
            c.space_param(p, "space", SpaceRole::Editor)?;
            c.page_param(p, "parent", OnPage::View)
        }
        "page.edit" | "page.labels" | "page.restore" => c.page_param(p, "id", OnPage::Edit),
        "page.move" => {
            c.page_param(p, "id", OnPage::Edit)?;
            c.space_param(p, "space", SpaceRole::Editor)?;
            for k in ["parent", "before", "after"] {
                c.destination(p, k)?;
            }
            Ok(())
        }
        "page.copy" => {
            c.page_param(p, "id", OnPage::View)?;
            let id = params::opt_id(p, "id").ok().flatten();
            if let Some(id) = id
                && p.get("children").and_then(Value::as_bool) == Some(true)
            {
                c.subtree(id, OnPage::View, "view")?;
            }
            c.space_param(p, "space", SpaceRole::Editor)?;
            c.destination(p, "parent")?;
            // Beside the original, by default.
            match (p.get("space"), p.get("parent"), id.and_then(|i| ws.pages.get(&i))) {
                (None | Some(Value::Null), None | Some(Value::Null), Some(pg)) => c.space(pg.space.as_str(), SpaceRole::Editor),
                _ => Ok(()),
            }
        }
        "page.delete" => {
            c.page_param(p, "id", OnPage::Edit)?;
            match params::opt_id(p, "id") {
                Ok(Some(id)) => c.subtree(id, OnPage::Edit, "edit"),
                _ => Ok(()),
            }
        }
        "page.comment" | "page.editComment" | "page.resolveComment" | "page.deleteComment" => c.page_param(p, "id", OnPage::View),
        "page.restrict" => {
            let Ok(Some(id)) = params::opt_id(p, "id") else { return Ok(()) };
            let Some(pg) = ws.pages.get(&id) else { return Ok(()) };
            let space_admin = ws.space_role(actor, &pg.space) == Some(SpaceRole::Admin);
            if !space_admin {
                c.page(id, OnPage::Edit)?;
            }
            c.keeps_self(p, space_admin)
        }
        "template.create" => match p.get("space") {
            Some(Value::String(k)) => c.space(k, SpaceRole::Admin),
            _ => c.admin("a workspace-wide template"),
        },
        "template.delete" => {
            let t = params::opt_id(p, "id").ok().flatten().and_then(|id| ws.templates.get(&id));
            match t.map(|t| t.space.clone()) {
                None => Ok(()),
                Some(Some(k)) if ws.space_role(actor, &k).is_none() => {
                    Err(Denied::Hidden(format!("no template with id {}", t.map(|t| t.id.0).unwrap_or_default())))
                }
                Some(Some(k)) => c.space(k.as_str(), SpaceRole::Admin),
                Some(None) => c.admin("deleting a workspace-wide template"),
            }
        }
        "access.setProjectRole" | "access.setProjectAccess" => c.project_param(p, "project", ProjectRole::Administrator),
        // M2.4: definitions belong to project administrators; a hidden project's ones read as missing.
        "field.create" | "component.create" | "version.create" => c.project_param(p, "project", ProjectRole::Administrator),
        "field.edit" | "field.delete" | "component.edit" | "component.delete" | "version.edit" | "version.delete" | "version.release" => {
            match definition_project(ws, spec.id, p) {
                None => Ok(()),
                Some(k) if !ws.can_browse_project(actor, &k) => {
                    let what = spec.id.split('.').next().unwrap_or("item");
                    Err(Denied::Hidden(format!("no {what} with id {}", params::opt_id(p, "id").ok().flatten().unwrap_or_default())))
                }
                Some(k) => {
                    c.project(k.as_str(), ProjectRole::Administrator)?;
                    if spec.id == "version.release" { c.release_target(p) } else { Ok(()) }
                }
            }
        }
        "access.setSpaceRole" | "access.setSpaceAccess" => c.space_param(p, "space", SpaceRole::Admin),
        other => c.admin(other),
    }
}

/// What a journal entry touched, captured when it was recorded (before it ran) so a sync stream
/// can decide per user what to tell them ([`delivery`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub projects: Vec<ProjectKey>,
    pub spaces: Vec<ProjectKey>,
    pub pages: Vec<Id>,
    /// Changes the page tree (positions of siblings): replicas missing hidden pages of the
    /// space would compute different positions.
    pub structure: bool,
    /// Changes who can see what.
    pub access: bool,
    /// Its targets couldn't be worked out: every non-admin resyncs.
    pub unresolved: bool,
}

/// What a sync stream sends a user for one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The entry as it is: the user can see everything it touched.
    Send,
    /// A placeholder (`sync.skip`): it touched only things the user can't see.
    Skip,
    /// The user's copy can't follow by replay (access changed, the entry mixes visible and
    /// hidden things, or the page tree moved around hidden pages): end the stream with `resync`.
    Resync,
}

fn project_of(raw: &str) -> Option<ProjectKey> {
    ProjectKey::parse(&raw.trim().to_ascii_uppercase()).ok()
}

fn issue_project(raw: &str) -> Option<ProjectKey> {
    IssueKey::parse(raw.trim()).ok().map(|k| k.project)
}

/// The [`Scope`] of a command about to run on `ws`.
pub fn scope(ws: &Workspace, command: &str, p: &Value) -> Scope {
    let mut s = Scope::default();
    let need = |opt: Option<ProjectKey>, s: &mut Vec<ProjectKey>, required: bool| -> bool {
        match opt {
            Some(k) => {
                if !s.contains(&k) {
                    s.push(k);
                }
                true
            }
            None => !required,
        }
    };
    let str_param = |k: &str| p.get(k).and_then(Value::as_str);
    let page_space = |id: Id| ws.pages.get(&id).map(|pg| pg.space.clone());
    let id_param = |k: &str| params::opt_id(p, k).ok().flatten();
    let mut ok = true;
    match command {
        // `inbox.markRead` touches only the actor's own inbox: sent to everyone as is (replaying
        // it on a filtered copy is harmless; ids missing there are ignored).
        "workspace.rename" | "user.add" | "user.edit" | "inbox.markRead" => {}
        c if c.starts_with("access.") || c == "page.restrict" => s.access = true,
        "project.create" => ok &= need(str_param("key").and_then(project_of), &mut s.projects, true),
        "space.create" => ok &= need(str_param("key").and_then(project_of), &mut s.spaces, true),
        "issue.create" | "sprint.create" => {
            ok &= need(str_param("project").and_then(project_of), &mut s.projects, true);
            need(str_param("parent").and_then(issue_project), &mut s.projects, false);
        }
        c if c.starts_with("issue.") => {
            ok &= need(str_param("key").and_then(issue_project), &mut s.projects, p.get("keys").is_none());
            for k in ["target", "parent", "before", "after"] {
                need(str_param(k).and_then(issue_project), &mut s.projects, false);
            }
            for k in params::keys(p, "keys").unwrap_or_default() {
                ok &= need(issue_project(&k), &mut s.projects, true);
            }
            if let Some(id) = id_param("sprint") {
                ok &= need(ws.sprints.get(&id).map(|sp| sp.project.clone()), &mut s.projects, true);
            }
        }
        c if c.starts_with("sprint.") => {
            for k in ["id", "moveOpenTo"] {
                if let Some(id) = id_param(k) {
                    ok &= need(ws.sprints.get(&id).map(|sp| sp.project.clone()), &mut s.projects, true);
                }
            }
            ok &= !s.projects.is_empty();
        }
        "page.create" => {
            s.structure = true;
            ok &= need(str_param("space").and_then(project_of), &mut s.spaces, true);
            if let Some(parent) = id_param("parent") {
                s.pages.push(parent);
            }
        }
        "page.move" | "page.copy" | "page.delete" => {
            s.structure = true;
            match id_param("id") {
                Some(id) => {
                    ok &= need(page_space(id), &mut s.spaces, true);
                    if command != "page.delete" {
                        s.pages.push(id);
                    }
                    if command == "page.copy" && p.get("children").and_then(Value::as_bool) == Some(true) {
                        s.pages.extend(ws.subtree(id).into_iter().filter(|c| *c != id));
                    }
                }
                None => ok = false,
            }
            need(str_param("space").and_then(project_of), &mut s.spaces, false);
            for k in ["parent", "before", "after"] {
                if let Some(id) = id_param(k) {
                    need(page_space(id), &mut s.spaces, false);
                    s.pages.push(id);
                }
            }
        }
        c if c.starts_with("page.") => match id_param("id") {
            Some(id) => s.pages.push(id),
            None => ok = false,
        },
        "template.create" => {
            need(str_param("space").and_then(project_of), &mut s.spaces, false);
        }
        // M2.4: definitions touch one project (by "project", or the one owning "id").
        "field.create" | "component.create" | "version.create" => ok &= need(str_param("project").and_then(project_of), &mut s.projects, true),
        c if c.starts_with("field.") || c.starts_with("component.") || c.starts_with("version.") => {
            ok &= need(definition_project(ws, c, p), &mut s.projects, true);
            if let Some(to) = id_param("moveUnresolvedTo") {
                ok &= need(ws.find_version(to).map(|(k, _)| k.clone()), &mut s.projects, true);
            }
        }
        "template.delete" => {
            if let Some(Some(k)) = id_param("id").and_then(|id| ws.templates.get(&id)).map(|t| t.space.clone()) {
                s.spaces.push(k);
            }
        }
        _ => ok = false,
    }
    s.unresolved = !ok;
    s
}

/// What `user` should be told about an entry with `scope`, judged on the workspace as it is now
/// (the server's, after the entry). An access change after an entry always forces a resync too,
/// so judging by the current state never leaves a client stale for long.
pub fn delivery(ws: &Workspace, user: &str, scope: &Scope) -> Delivery {
    match ws.workspace_role(user) {
        Some(WorkspaceRole::Admin) => return Delivery::Send,
        None => return Delivery::Resync,
        Some(_) => {}
    }
    if scope.access || scope.unresolved {
        return Delivery::Resync;
    }
    let (mut seen, mut hid) = (0usize, 0usize);
    let mut count = |v: bool| if v { seen += 1 } else { hid += 1 };
    for k in &scope.projects {
        if !ws.projects.contains_key(k) {
            return Delivery::Resync;
        }
        count(ws.can_browse_project(user, k));
    }
    for k in &scope.spaces {
        if !ws.spaces.contains_key(k) {
            return Delivery::Resync;
        }
        let v = ws.space_role(user, k).is_some();
        count(v);
        if v && scope.structure && ws.pages.values().any(|pg| &pg.space == k && !ws.page_visible(user, pg.id)) {
            return Delivery::Resync;
        }
    }
    for id in &scope.pages {
        if !ws.pages.contains_key(id) {
            return Delivery::Resync;
        }
        count(ws.page_visible(user, *id));
    }
    match (seen, hid) {
        (_, 0) => Delivery::Send,
        (0, _) => Delivery::Skip,
        _ => Delivery::Resync,
    }
}
