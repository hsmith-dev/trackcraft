//! Agile planning: issue rank (backlog order) and sprints.
//!
//! **Rank** is one order per project (Jira's rank is one global order that every board, the
//! backlog and each sprint show a filtered view of). New issues rank at the bottom of their
//! project. See [`crate::rank`] for the key scheme.
//!
//! **Sprints** belong to one project and go Future → Active → Closed. Behaviour follows
//! Atlassian's public docs for scrum boards:
//! - one active sprint per project (no parallel sprints yet);
//! - a sprint needs a start and an end date to start, and at least one issue that is not in a
//!   Done-category status (the docs describe the Start button staying unavailable otherwise);
//! - completing a sprint keeps every issue that was in it on record (`Issue::past_sprints`);
//!   Done-category issues stay with the closed sprint, open ones move to another open sprint, a
//!   new sprint, or the backlog;
//! - sub-tasks follow their parent: they take the parent's sprint when created or when the
//!   parent moves, and can't be moved on their own;
//! - a closed sprint can't take issues, change dates, or be deleted;
//! - a sprint can't complete while a Done-category issue in it has sub-tasks that aren't done.
//!
//! Sprint membership changes are history-worthy (field `sprint`, sprint names as values);
//! rank changes are not.

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, IssueKey, ProjectKey, Timestamp};

use crate::rank::{is_valid_rank, rank_between, ranks_between, rebalanced};
use crate::{ModelError, Result, StatusCategory, Workspace, title};

/// Most issues one rank / sprint move takes.
pub const MAX_MOVE: usize = 1000;
/// Most sprints a workspace holds.
pub const MAX_SPRINTS: usize = 100_000;
/// Longest sprint goal, in characters.
pub const MAX_GOAL: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SprintState {
    Future,
    Active,
    Closed,
}

impl SprintState {
    /// The Jira Agile REST name (`future`, `active`, `closed`).
    pub fn name(self) -> &'static str {
        match self {
            SprintState::Future => "future",
            SprintState::Active => "active",
            SprintState::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sprint {
    pub id: Id,
    pub project: ProjectKey,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub goal: String,
    pub state: SprintState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Timestamp>,
    /// When it was completed (closed sprints only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<Timestamp>,
}

/// Where a rank move puts the issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor<'a> {
    Before(&'a str),
    After(&'a str),
}

/// Where `complete_sprint` moves open issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOpenTo {
    Backlog,
    Sprint(Id),
    /// A new future sprint, created by the completion.
    New,
}

/// What `complete_sprint` did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Completion {
    pub done: Vec<IssueKey>,
    pub moved: Vec<IssueKey>,
    /// Where open issues went (`None` = backlog).
    pub to: Option<Id>,
}

/// Changes to a sprint's fields (each `None` = keep). Dates are `Some(None)` to clear them,
/// which only a future sprint allows (an active sprint needs both; a closed one keeps its own).
#[derive(Debug, Clone, Default)]
pub struct SprintEdit {
    pub name: Option<String>,
    pub goal: Option<String>,
    pub start: Option<Option<Timestamp>>,
    pub end: Option<Option<Timestamp>>,
}

fn goal(g: &str) -> Result<String> {
    let g = g.trim();
    if g.chars().count() > MAX_GOAL {
        return Err(ModelError::Invalid(format!("a sprint goal is at most {MAX_GOAL} characters")));
    }
    if g.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ModelError::Invalid("a sprint goal can't contain control characters".into()));
    }
    Ok(g.to_string())
}

fn check_dates(start: Option<Timestamp>, end: Option<Timestamp>) -> Result<()> {
    match (start, end) {
        (Some(s), Some(e)) if e <= s => Err(ModelError::Invalid("a sprint's end date must be after its start date".into())),
        _ => Ok(()),
    }
}

fn no_sprint(id: Id) -> ModelError {
    ModelError::Invalid(format!("no sprint with id {id}"))
}

impl Workspace {
    // ---- rank ----------------------------------------------------------------------------

    /// A project's issues in rank order (ties, which a repaired workspace never has, by number).
    pub fn rank_order(&self, project: &ProjectKey) -> Vec<IssueKey> {
        let mut v: Vec<(&str, u64, &IssueKey)> =
            self.issues.values().filter(|i| &i.key.project == project).map(|i| (i.rank.as_str(), i.key.number, &i.key)).collect();
        v.sort();
        v.into_iter().map(|(_, _, k)| k.clone()).collect()
    }

    /// Gives every issue of `project` a fresh, evenly spaced rank following `order`; issues of
    /// the project missing from `order` follow in their current order.
    pub fn rebalance_ranks(&mut self, project: &ProjectKey, order: &[IssueKey]) {
        let mut full: Vec<IssueKey> = order.iter().filter(|k| &k.project == project && self.issues.contains_key(k)).cloned().collect();
        let mut seen: std::collections::BTreeSet<IssueKey> = std::collections::BTreeSet::new();
        full.retain(|k| seen.insert(k.clone()));
        for k in self.rank_order(project) {
            if !seen.contains(&k) {
                full.push(k);
            }
        }
        for (k, r) in full.iter().zip(rebalanced(full.len())) {
            if let Some(i) = self.issues.get_mut(k) {
                i.rank = r;
            }
        }
    }

    /// The rank for a new issue at the bottom of `project` (rebalancing first if needed).
    pub(crate) fn bottom_rank(&mut self, project: &ProjectKey) -> String {
        let last = self.issues.values().filter(|i| &i.key.project == project).map(|i| i.rank.as_str()).max().map(str::to_string);
        match rank_between(last.as_deref(), None).filter(|r| is_valid_rank(r)) {
            Some(r) => r,
            None => {
                let order = self.rank_order(project);
                self.rebalance_ranks(project, &order);
                let last = order.last().and_then(|k| self.issues.get(k)).map(|i| i.rank.clone());
                rank_between(last.as_deref(), None).unwrap_or_else(|| "z".into())
            }
        }
    }

    /// Repairs data loaded from a file (or built by hand): projects whose ranks are missing,
    /// malformed or duplicated are rebalanced — valid ranks keep their order, the rest follow in
    /// key order (so files saved before ranks existed load in key order); sprint references to
    /// missing sprints are dropped, references to closed sprints move to `past_sprints`.
    pub fn repair(&mut self) {
        let projects: Vec<ProjectKey> = self.projects.keys().cloned().collect();
        for p in &projects {
            let mut ranks = std::collections::BTreeSet::new();
            let ok = self.issues.values().filter(|i| &i.key.project == p).all(|i| is_valid_rank(&i.rank) && ranks.insert(i.rank.clone()));
            if !ok {
                let mut v: Vec<(bool, String, u64, IssueKey)> = self
                    .issues
                    .values()
                    .filter(|i| &i.key.project == p)
                    .map(|i| {
                        let valid = is_valid_rank(&i.rank);
                        (!valid, if valid { i.rank.clone() } else { String::new() }, i.key.number, i.key.clone())
                    })
                    .collect();
                v.sort();
                let order: Vec<IssueKey> = v.into_iter().map(|(_, _, _, k)| k).collect();
                self.rebalance_ranks(p, &order);
            }
        }
        self.sprints.retain(|id, s| s.id == *id && projects.contains(&s.project));
        let sprints = self.sprints.clone();
        for i in self.issues.values_mut() {
            if let Some(id) = i.sprint {
                match sprints.get(&id) {
                    Some(s) if s.project == i.key.project && s.state != SprintState::Closed => {}
                    Some(s) if s.project == i.key.project => {
                        i.sprint = None;
                        if !i.past_sprints.contains(&id) {
                            i.past_sprints.push(id);
                        }
                    }
                    _ => i.sprint = None,
                }
            }
            i.past_sprints.retain(|id| sprints.get(id).is_some_and(|s| s.project == i.key.project && s.state == SprintState::Closed));
            let mut seen = std::collections::BTreeSet::new();
            i.past_sprints.retain(|id| seen.insert(*id));
        }
    }

    /// Resolves a list of issue keys (deduplicated, at most [`MAX_MOVE`]), all in one project.
    fn resolve_keys(&self, keys: &[String]) -> Result<(ProjectKey, Vec<IssueKey>)> {
        if keys.is_empty() {
            return Err(ModelError::Invalid("no issues given".into()));
        }
        if keys.len() > MAX_MOVE {
            return Err(ModelError::Invalid(format!("at most {MAX_MOVE} issues at a time")));
        }
        let mut out: Vec<IssueKey> = Vec::with_capacity(keys.len());
        for k in keys {
            let key = self.issue(k)?.key.clone();
            if !out.contains(&key) {
                out.push(key);
            }
        }
        let project = out.first().map(|k| k.project.clone()).ok_or_else(|| ModelError::Invalid("no issues given".into()))?;
        if out.iter().any(|k| k.project != project) {
            return Err(ModelError::Invalid("all issues must be in the same project".into()));
        }
        Ok((project, out))
    }

    /// Moves `keys` (kept in the given order) to sit immediately before or after the anchor.
    pub fn rank_issues(&mut self, keys: &[String], anchor: Anchor<'_>) -> Result<Vec<IssueKey>> {
        let (project, keys) = self.resolve_keys(keys)?;
        let (a, before) = match anchor {
            Anchor::Before(a) => (a, true),
            Anchor::After(a) => (a, false),
        };
        let anchor = self.issue(a)?.key.clone();
        if anchor.project != project {
            return Err(ModelError::Invalid(format!("{anchor} is in another project")));
        }
        if keys.contains(&anchor) {
            return Err(ModelError::Invalid(format!("{anchor} can't be ranked relative to itself")));
        }
        let rest: Vec<IssueKey> = self.rank_order(&project).into_iter().filter(|k| !keys.contains(k)).collect();
        let pos = rest.iter().position(|k| k == &anchor).ok_or_else(|| ModelError::NoSuchIssue(anchor.to_string()))?;
        let at = if before { pos } else { pos.saturating_add(1) };
        let rank_of = |k: Option<&IssueKey>| k.and_then(|k| self.issues.get(k)).map(|i| i.rank.clone());
        let lo = if at == 0 { None } else { rank_of(rest.get(at.saturating_sub(1))) };
        let hi = rank_of(rest.get(at));
        match ranks_between(lo.as_deref(), hi.as_deref(), keys.len()) {
            Some(ranks) => {
                for (k, r) in keys.iter().zip(ranks) {
                    if let Some(i) = self.issues.get_mut(k) {
                        i.rank = r;
                    }
                }
            }
            None => {
                let mut order = rest;
                let tail = order.split_off(at.min(order.len()));
                order.extend(keys.iter().cloned());
                order.extend(tail);
                self.rebalance_ranks(&project, &order);
            }
        }
        Ok(keys)
    }

    // ---- sprints -------------------------------------------------------------------------

    pub fn sprint(&self, id: Id) -> Result<&Sprint> {
        self.sprints.get(&id).ok_or_else(|| no_sprint(id))
    }

    /// A project's sprints: active first, then future, then closed, each by creation.
    pub fn project_sprints(&self, project: &ProjectKey) -> Vec<&Sprint> {
        let mut v: Vec<&Sprint> = self.sprints.values().filter(|s| &s.project == project).collect();
        let order = |s: &SprintState| match s {
            SprintState::Active => 0,
            SprintState::Future => 1,
            SprintState::Closed => 2,
        };
        v.sort_by_key(|s| (order(&s.state), s.id));
        v
    }

    pub fn active_sprint(&self, project: &ProjectKey) -> Option<&Sprint> {
        self.sprints.values().find(|s| &s.project == project && s.state == SprintState::Active)
    }

    fn sprint_name(&self, id: Option<Id>) -> Option<String> {
        id.and_then(|id| self.sprints.get(&id)).map(|s| s.name.clone())
    }

    pub fn create_sprint(
        &mut self,
        project: &str,
        name: Option<&str>,
        goal_text: Option<&str>,
        start: Option<Timestamp>,
        end: Option<Timestamp>,
    ) -> Result<Id> {
        let project = self.project(project)?.key.clone();
        if self.sprints.len() >= MAX_SPRINTS {
            return Err(ModelError::Invalid(format!("a workspace holds at most {MAX_SPRINTS} sprints")));
        }
        check_dates(start, end)?;
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => title(n, "the sprint name")?,
            None => {
                let mut n = self.sprints.values().filter(|s| s.project == project).count().saturating_add(1);
                loop {
                    let candidate = format!("{project} Sprint {n}");
                    if !self.sprints.values().any(|s| s.project == project && s.name == candidate) {
                        break candidate;
                    }
                    n = n.saturating_add(1);
                }
            }
        };
        let goal = goal(goal_text.unwrap_or(""))?;
        let id = self.ids.next_id();
        self.sprints.insert(id, Sprint { id, project, name, goal, state: SprintState::Future, start, end, completed: None });
        Ok(id)
    }

    pub fn edit_sprint(&mut self, id: Id, edit: SprintEdit) -> Result<()> {
        let s = self.sprint(id)?;
        if s.state == SprintState::Closed && (edit.start.is_some() || edit.end.is_some()) {
            return Err(ModelError::Invalid(format!("{} is closed: its dates can't change", s.name)));
        }
        let name = edit.name.as_deref().map(|n| title(n, "the sprint name")).transpose()?;
        let goal = edit.goal.as_deref().map(goal).transpose()?;
        if s.state != SprintState::Future && (edit.start == Some(None) || edit.end == Some(None)) {
            return Err(ModelError::Invalid(format!("{} is {}: only a future sprint's dates can be cleared", s.name, s.state.name())));
        }
        let start = edit.start.unwrap_or(s.start);
        let end = edit.end.unwrap_or(s.end);
        check_dates(start, end)?;
        let s = self.sprints.get_mut(&id).ok_or_else(|| no_sprint(id))?;
        if let Some(n) = name {
            s.name = n;
        }
        if let Some(g) = goal {
            s.goal = g;
        }
        s.start = start;
        s.end = end;
        Ok(())
    }

    /// Deletes a future or active sprint; its issues return to the backlog.
    pub fn delete_sprint(&mut self, id: Id, author: &str, now: Timestamp) -> Result<Vec<IssueKey>> {
        let s = self.sprint(id)?;
        if s.state == SprintState::Closed {
            return Err(ModelError::Invalid(format!("{} is closed and keeps its record; it can't be deleted", s.name)));
        }
        let name = s.name.clone();
        let members: Vec<IssueKey> = self.issues.values().filter(|i| i.sprint == Some(id)).map(|i| i.key.clone()).collect();
        for k in &members {
            if let Some(i) = self.issues.get_mut(k) {
                i.sprint = None;
                i.updated = now;
            }
            self.record(k, author, now, vec![("sprint", Some(name.clone()), None)]);
        }
        self.sprints.remove(&id);
        Ok(members)
    }

    /// Moves issues (and their sub-tasks) into an open sprint, or to the backlog (`None`).
    pub fn set_sprint(&mut self, keys: &[String], sprint: Option<Id>, author: &str, now: Timestamp) -> Result<Vec<IssueKey>> {
        let (project, keys) = self.resolve_keys(keys)?;
        if let Some(id) = sprint {
            let s = self.sprint(id)?;
            if s.project != project {
                return Err(ModelError::Invalid(format!("{} belongs to project {}", s.name, s.project)));
            }
            if s.state == SprintState::Closed {
                return Err(ModelError::Invalid(format!("{} is closed: issues can't be added to it", s.name)));
            }
        }
        let mut all = keys.clone();
        for k in &keys {
            let i = self.issues.get(k).ok_or_else(|| ModelError::NoSuchIssue(k.to_string()))?;
            if self.is_subtask(i) {
                return Err(ModelError::Invalid(format!("{k} is a sub-task: it follows its parent's sprint")));
            }
            for sub in self.issues.values().filter(|s| s.parent.as_ref() == Some(k)) {
                if !all.contains(&sub.key) {
                    all.push(sub.key.clone());
                }
            }
        }
        let to = self.sprint_name(sprint);
        for k in &all {
            let Some(i) = self.issues.get_mut(k) else { continue };
            if i.sprint == sprint {
                continue;
            }
            let old = std::mem::replace(&mut i.sprint, sprint);
            i.updated = now;
            let from = self.sprint_name(old);
            self.record(k, author, now, vec![("sprint", from, to.clone())]);
        }
        Ok(keys)
    }

    /// Whether the issue's type is a sub-task type of its project.
    pub fn is_subtask(&self, i: &crate::Issue) -> bool {
        self.projects
            .get(&i.key.project)
            .and_then(|p| p.issue_types.iter().find(|t| t.name.eq_ignore_ascii_case(&i.issue_type)))
            .is_some_and(|t| t.subtask)
    }

    /// Starts a future sprint, optionally renaming it and setting its goal at the same time.
    pub fn start_sprint(&mut self, id: Id, name: Option<&str>, start: Timestamp, end: Timestamp, goal_text: Option<&str>) -> Result<()> {
        let s = self.sprint(id)?;
        if s.state != SprintState::Future {
            return Err(ModelError::Invalid(format!("{} is {}: only a future sprint can start", s.name, s.state.name())));
        }
        if let Some(a) = self.active_sprint(&s.project) {
            return Err(ModelError::Invalid(format!("{} is already active: complete it before starting another sprint", a.name)));
        }
        check_dates(Some(start), Some(end))?;
        let open = self.issues.values().filter(|i| i.sprint == Some(id)).any(|i| self.status_category(i) != StatusCategory::Done);
        if !open {
            return Err(ModelError::Invalid(format!("{} has no open issues: add an issue that isn't done before starting it", s.name)));
        }
        let goal = goal_text.map(goal).transpose()?;
        let name = name.map(|n| title(n, "the sprint name")).transpose()?;
        let s = self.sprints.get_mut(&id).ok_or_else(|| no_sprint(id))?;
        if let Some(n) = name {
            s.name = n;
        }
        s.state = SprintState::Active;
        s.start = Some(start);
        s.end = Some(end);
        if let Some(g) = goal {
            s.goal = g;
        }
        Ok(())
    }

    /// Completes the active sprint `id`. Every issue in it keeps the sprint in `past_sprints`;
    /// Done-category issues stay there, open ones move to `to`.
    ///
    /// Refused while a Done-category issue in the sprint has sub-tasks that aren't done: the
    /// parent would stay with the closed sprint while its open sub-tasks moved on (Atlassian's
    /// public docs for completing a sprint require those sub-tasks to be done first).
    pub fn complete_sprint(&mut self, id: Id, to: MoveOpenTo, author: &str, now: Timestamp) -> Result<Completion> {
        let s = self.sprint(id)?;
        if s.state != SprintState::Active {
            return Err(ModelError::Invalid(format!("{} is {}: only the active sprint can be completed", s.name, s.state.name())));
        }
        let mut blocked: Vec<String> = Vec::new();
        for parent in self.issues.values().filter(|i| i.sprint == Some(id) && self.status_category(i) == StatusCategory::Done) {
            let open: Vec<String> = self
                .issues
                .values()
                .filter(|c| c.parent.as_ref() == Some(&parent.key) && self.is_subtask(c) && self.status_category(c) != StatusCategory::Done)
                .map(|c| c.key.to_string())
                .collect();
            if !open.is_empty() {
                blocked.push(format!("{} ({})", parent.key, open.join(", ")));
            }
        }
        if !blocked.is_empty() {
            let shown = blocked.iter().take(10).cloned().collect::<Vec<_>>().join("; ");
            let more = if blocked.len() > 10 { format!(" and {} more", blocked.len().saturating_sub(10)) } else { String::new() };
            return Err(ModelError::Invalid(format!(
                "{} can't be completed: done issues have open sub-tasks: {shown}{more}. Finish the sub-tasks or reopen the parent first",
                s.name
            )));
        }
        let (project, name) = (s.project.clone(), s.name.clone());
        let target = match to {
            MoveOpenTo::Backlog => None,
            MoveOpenTo::Sprint(t) => {
                let ts = self.sprint(t)?;
                if t == id || ts.project != project || ts.state == SprintState::Closed {
                    return Err(ModelError::Invalid(format!("open issues can move only to another open sprint of {project}")));
                }
                Some(t)
            }
            MoveOpenTo::New => Some(self.create_sprint(project.as_str(), None, None, None, None)?),
        };
        let to_name = self.sprint_name(target);
        let members: Vec<(IssueKey, bool)> =
            self.issues.values().filter(|i| i.sprint == Some(id)).map(|i| (i.key.clone(), self.status_category(i) == StatusCategory::Done)).collect();
        let mut out = Completion { to: target, ..Completion::default() };
        for (k, done) in members {
            let Some(i) = self.issues.get_mut(&k) else { continue };
            if !i.past_sprints.contains(&id) {
                i.past_sprints.push(id);
            }
            i.sprint = if done { None } else { target };
            if done {
                out.done.push(k);
            } else {
                i.updated = now;
                self.record(&k, author, now, vec![("sprint", Some(name.clone()), to_name.clone())]);
                out.moved.push(k);
            }
        }
        let s = self.sprints.get_mut(&id).ok_or_else(|| no_sprint(id))?;
        s.state = SprintState::Closed;
        s.completed = Some(now);
        Ok(out)
    }
}
