//! Primitives for importers (`trackcraft-import`): bring in data that already has an identity
//! and a past — issue keys, creation dates, statuses outside the default workflow, closed
//! sprints, pages with their own timestamps. Each one validates like the ordinary primitives;
//! none of them is reachable from a user command except through an import.

use trackcraft_core::{Id, IssueKey, Timestamp};
use trackcraft_markup::Doc;

use crate::{IssueType, MAX_SPRINTS, ModelError, NewIssue, Result, Sprint, SprintState, Status, StatusCategory, Transition, Workspace, clip, title};

/// Most statuses / issue types an import may add to one project.
pub const MAX_IMPORTED_KINDS: usize = 200;

impl Workspace {
    /// Adds a status to a project's workflow, reachable from any status (a global transition).
    /// Returns `false` when a status of that name (case-insensitive) already exists.
    pub fn add_status(&mut self, project: &str, name: &str, category: StatusCategory) -> Result<bool> {
        let name = title(name, "the status name")?;
        let key = self.project(project)?.key.clone();
        let p = self.projects.get_mut(&key).ok_or_else(|| ModelError::NoSuchProject(key.to_string()))?;
        if p.workflow.status(&name).is_some() {
            return Ok(false);
        }
        if p.workflow.statuses.len() >= MAX_IMPORTED_KINDS {
            return Err(ModelError::Invalid(format!("project {key} has too many statuses (at most {MAX_IMPORTED_KINDS})")));
        }
        p.workflow.statuses.push(Status { name: name.clone(), category });
        p.workflow.transitions.push(Transition { name: name.clone(), from: vec![], to: name });
        Ok(true)
    }

    /// Adds an issue type to a project. Returns `false` when one of that name already exists.
    pub fn add_issue_type(&mut self, project: &str, name: &str, subtask: bool) -> Result<bool> {
        let name = title(name, "the issue type name")?;
        let key = self.project(project)?.key.clone();
        let p = self.projects.get_mut(&key).ok_or_else(|| ModelError::NoSuchProject(key.to_string()))?;
        if p.issue_types.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
            return Ok(false);
        }
        if p.issue_types.len() >= MAX_IMPORTED_KINDS {
            return Err(ModelError::Invalid(format!("project {key} has too many issue types (at most {MAX_IMPORTED_KINDS})")));
        }
        p.issue_types.push(IssueType { name, subtask });
        Ok(true)
    }

    /// Creates an issue with a given number (an imported key such as `ABC-17`), created at
    /// `created`. Refuses a number already taken; the project's `next_number` moves past it so
    /// later issues never reuse it. Everything else validates as in [`Workspace::create_issue`].
    pub fn create_issue_numbered(&mut self, new: NewIssue, number: u64, reporter: &str, created: Timestamp) -> Result<IssueKey> {
        if number == 0 {
            return Err(ModelError::Invalid("issue numbers start at 1".into()));
        }
        let pkey = self.project(&new.project)?.key.clone();
        let key = IssueKey::new(pkey.clone(), number);
        if self.issues.contains_key(&key) {
            return Err(ModelError::Duplicate(format!("issue {key}")));
        }
        let saved = self.projects.get(&pkey).map(|p| p.next_number).unwrap_or(1);
        if let Some(p) = self.projects.get_mut(&pkey) {
            p.next_number = number;
        }
        let out = self.create_issue(new, reporter, created);
        if let Some(p) = self.projects.get_mut(&pkey) {
            p.next_number = if out.is_ok() { saved.max(number.saturating_add(1)) } else { saved };
        }
        out
    }

    /// Sets an imported issue's status (which must be in its project's workflow; no transition
    /// check — the issue got there in the system it came from) and its dates.
    pub fn set_imported_state(
        &mut self,
        key: &str,
        status: Option<&str>,
        updated: Option<Timestamp>,
        resolved: Option<Option<Timestamp>>,
    ) -> Result<()> {
        let issue = self.issue(key)?;
        let k = issue.key.clone();
        let status = match status {
            Some(s) => {
                let wf = &self.projects.get(&k.project).ok_or_else(|| ModelError::NoSuchProject(k.project.to_string()))?.workflow;
                Some(wf.status(s).ok_or_else(|| ModelError::Invalid(format!("project {} has no status {}", k.project, clip(s, 40))))?.clone())
            }
            None => None,
        };
        let i = self.issues.get_mut(&k).ok_or_else(|| ModelError::NoSuchIssue(k.to_string()))?;
        if let Some(st) = status {
            i.status = st.name;
            i.resolved = match (st.category, resolved) {
                (StatusCategory::Done, Some(r)) => r.or(Some(i.updated)),
                (StatusCategory::Done, None) => i.resolved.or(Some(i.updated)),
                _ => None,
            };
        } else if let Some(r) = resolved {
            i.resolved = r;
        }
        if let Some(u) = updated {
            i.updated = u.max(i.created);
        }
        Ok(())
    }

    /// Creates a sprint in any state, as it was in the system it came from. At most one active
    /// sprint per project; an active sprint needs a start and an end date.
    pub fn import_sprint(
        &mut self,
        project: &str,
        name: &str,
        state: SprintState,
        start: Option<Timestamp>,
        end: Option<Timestamp>,
        completed: Option<Timestamp>,
    ) -> Result<Id> {
        let project = self.project(project)?.key.clone();
        let name = title(name, "the sprint name")?;
        if self.sprints.len() >= MAX_SPRINTS {
            return Err(ModelError::Invalid(format!("a workspace holds at most {MAX_SPRINTS} sprints")));
        }
        if let (Some(s), Some(e)) = (start, end)
            && e <= s
        {
            return Err(ModelError::Invalid("a sprint's end date must be after its start date".into()));
        }
        if state == SprintState::Active {
            if start.is_none() || end.is_none() {
                return Err(ModelError::Invalid("an active sprint needs a start and an end date".into()));
            }
            if let Some(a) = self.active_sprint(&project) {
                return Err(ModelError::Invalid(format!("{} is already active in {project}", a.name)));
            }
        }
        let completed = if state == SprintState::Closed { completed.or(end) } else { None };
        let id = self.ids.next_id();
        self.sprints.insert(id, Sprint { id, project, name, goal: String::new(), state, start, end, completed });
        Ok(id)
    }

    /// Places an imported issue in sprints: `current` must be an open sprint of its project (or
    /// `None` = backlog), `past` closed ones. No history is recorded (the import is the history).
    pub fn set_imported_sprints(&mut self, key: &str, current: Option<Id>, past: &[Id]) -> Result<()> {
        let k = self.issue(key)?.key.clone();
        if let Some(id) = current {
            let s = self.sprint(id)?;
            if s.project != k.project || s.state == SprintState::Closed {
                return Err(ModelError::Invalid(format!("{} can't hold {k}", s.name)));
            }
        }
        let mut kept: Vec<Id> = Vec::new();
        for id in past {
            let s = self.sprint(*id)?;
            if s.project != k.project || s.state != SprintState::Closed {
                return Err(ModelError::Invalid(format!("{} is not a closed sprint of {}", s.name, k.project)));
            }
            if !kept.contains(id) {
                kept.push(*id);
            }
        }
        let i = self.issues.get_mut(&k).ok_or_else(|| ModelError::NoSuchIssue(k.to_string()))?;
        i.sprint = current;
        i.past_sprints = kept;
        Ok(())
    }

    /// Fills a page that has never been edited (version 1, no history) with imported content and
    /// dates, without making a new version: a space's homepage, or a page just created by the
    /// importer. The title stays unique in the space.
    pub fn set_imported_page(
        &mut self,
        id: Id,
        new_title: Option<&str>,
        body: Doc,
        author: Option<&str>,
        created: Timestamp,
        updated: Timestamp,
    ) -> Result<()> {
        let author = author.map(|a| self.user(a).map(|u| u.id.clone())).transpose()?;
        let page = self.page(id)?;
        if page.version != 1 || !page.history.is_empty() {
            return Err(ModelError::Invalid(format!("page {id} has been edited: an import can't overwrite it")));
        }
        let t = match new_title {
            Some(t) => {
                let t = title(t, "the page title")?;
                if self.pages.values().any(|p| p.space == page.space && p.id != id && p.title.eq_ignore_ascii_case(&t)) {
                    return Err(ModelError::Duplicate(format!("a page titled '{t}' in space {}", page.space)));
                }
                t
            }
            None => page.title.clone(),
        };
        let p = self.pages.get_mut(&id).ok_or(ModelError::NoSuchPage(id))?;
        p.title = t;
        p.body = body;
        if let Some(a) = author {
            p.author = a;
        }
        p.created = created;
        p.updated = updated.max(created);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewPage, ProjectKind};

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn ws() -> Workspace {
        let mut w = Workspace::new("t");
        w.add_user("ann", "Ann").unwrap();
        w.create_project("ABC", "Abc", ProjectKind::Software, None).unwrap();
        w
    }

    #[test]
    fn numbered_issues_keep_their_keys() {
        let mut w = ws();
        let new = |s: &str| NewIssue { project: "ABC".into(), summary: s.into(), ..Default::default() };
        let k = w.create_issue_numbered(new("a"), 17, "ann", at("2024-01-02T03:04:05Z")).unwrap();
        assert_eq!(k.to_string(), "ABC-17");
        assert_eq!(w.issue("ABC-17").unwrap().created, at("2024-01-02T03:04:05Z"));
        assert!(w.create_issue_numbered(new("b"), 17, "ann", at("2024-01-02T03:04:05Z")).is_err());
        assert!(w.create_issue_numbered(new("b"), 0, "ann", at("2024-01-02T03:04:05Z")).is_err());
        w.create_issue_numbered(new("c"), 3, "ann", at("2024-01-02T03:04:05Z")).unwrap();
        assert_eq!(w.project("ABC").unwrap().next_number, 18);
        // A failed create leaves next_number alone.
        let bad = NewIssue { assignee: Some("nobody".into()), ..new("d") };
        assert!(w.create_issue_numbered(bad, 99, "ann", at("2024-01-02T00:00:00Z")).is_err());
        assert_eq!(w.project("ABC").unwrap().next_number, 18);
    }

    #[test]
    fn statuses_types_and_state() {
        let mut w = ws();
        assert!(w.add_status("ABC", "Shipped", StatusCategory::Done).unwrap());
        assert!(!w.add_status("ABC", "shipped", StatusCategory::Done).unwrap());
        assert!(w.add_status("ABC", "", StatusCategory::Done).is_err());
        assert!(w.add_issue_type("ABC", "Spike", false).unwrap());
        assert!(!w.add_issue_type("ABC", "spike", false).unwrap());
        let new = NewIssue { project: "ABC".into(), summary: "s".into(), issue_type: Some("Spike".into()), ..Default::default() };
        let k = w.create_issue(new, "ann", at("2024-01-01T00:00:00Z")).unwrap().to_string();
        w.set_imported_state(&k, Some("Shipped"), Some(at("2024-02-01T00:00:00Z")), Some(Some(at("2024-01-15T00:00:00Z")))).unwrap();
        let i = w.issue(&k).unwrap();
        assert_eq!((i.status.as_str(), i.resolved), ("Shipped", Some(at("2024-01-15T00:00:00Z"))));
        assert!(w.set_imported_state(&k, Some("Nope"), None, None).is_err());
        w.set_imported_state(&k, Some("To Do"), None, None).unwrap();
        assert_eq!(w.issue(&k).unwrap().resolved, None);
    }

    #[test]
    fn sprints_and_pages() {
        let mut w = ws();
        let t = at("2024-01-01T00:00:00Z");
        let e = at("2024-01-15T00:00:00Z");
        let closed = w.import_sprint("ABC", "S1", SprintState::Closed, Some(t), Some(e), None).unwrap();
        let active = w.import_sprint("ABC", "S2", SprintState::Active, Some(t), Some(e), None).unwrap();
        assert!(w.import_sprint("ABC", "S3", SprintState::Active, Some(t), Some(e), None).is_err());
        assert!(w.import_sprint("ABC", "S3", SprintState::Active, None, None, None).is_err());
        let new = NewIssue { project: "ABC".into(), summary: "s".into(), ..Default::default() };
        let k = w.create_issue(new, "ann", t).unwrap().to_string();
        w.set_imported_sprints(&k, Some(active), &[closed, closed]).unwrap();
        assert_eq!(w.issue(&k).unwrap().past_sprints, vec![closed]);
        assert!(w.set_imported_sprints(&k, Some(closed), &[]).is_err());
        assert!(w.set_imported_sprints(&k, None, &[active]).is_err());

        w.create_space("DOC", "Docs", "ann", t).unwrap();
        let home = w.space("DOC").unwrap().homepage;
        let page = NewPage { space: "DOC".into(), parent: None, title: "Other".into(), body: Doc::default() };
        let p = w.create_page(page, "ann", t).unwrap();
        w.set_imported_page(home, Some("Home"), Doc::plain("hi"), None, t, e).unwrap();
        assert_eq!(w.page(home).unwrap().version, 1);
        assert!(w.set_imported_page(home, Some("other"), Doc::default(), None, t, e).is_err());
        w.edit_page(p, crate::PageEdit { body: Some(Doc::plain("x")), ..Default::default() }, "ann", e).unwrap();
        assert!(w.set_imported_page(p, None, Doc::default(), None, t, e).is_err());
    }
}
