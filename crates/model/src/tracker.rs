//! The tracker: projects, issue types, workflows, issues, comments and links.

use serde::{Deserialize, Serialize};
use trackcraft_core::{Id, IssueKey, ProjectKey, Timestamp};
use trackcraft_markup::Doc;

use crate::{ModelError, Result, Workspace, labels, title};

/// Default priorities, highest first (rank 1 = highest).
pub const PRIORITIES: [&str; 5] = ["Highest", "High", "Medium", "Low", "Lowest"];

/// Rank of a priority name, case-insensitive (`None` for unknown names).
pub fn priority_rank(name: &str) -> Option<i64> {
    PRIORITIES.iter().position(|p| p.eq_ignore_ascii_case(name)).and_then(|i| i64::try_from(i).ok()).map(|i| i + 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusCategory {
    ToDo,
    InProgress,
    Done,
}

impl StatusCategory {
    pub fn name(self) -> &'static str {
        match self {
            StatusCategory::ToDo => "To Do",
            StatusCategory::InProgress => "In Progress",
            StatusCategory::Done => "Done",
        }
    }
    pub fn rank(self) -> i64 {
        match self {
            StatusCategory::ToDo => 1,
            StatusCategory::InProgress => 2,
            StatusCategory::Done => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub name: String,
    pub category: StatusCategory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub name: String,
    /// Statuses this transition starts from; empty = from any status (a "global" transition).
    pub from: Vec<String>,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    /// The first status is where new issues start.
    pub statuses: Vec<Status>,
    pub transitions: Vec<Transition>,
}

impl Workflow {
    /// A simple software workflow: To Do → In Progress → In Review → Done, any status can move to
    /// any other (the shape of Jira's default "simplified" workflow).
    pub fn software() -> Self {
        let st = |n: &str, c| Status { name: n.into(), category: c };
        let statuses = vec![
            st("To Do", StatusCategory::ToDo),
            st("In Progress", StatusCategory::InProgress),
            st("In Review", StatusCategory::InProgress),
            st("Done", StatusCategory::Done),
        ];
        let transitions = statuses.iter().map(|s| Transition { name: s.name.clone(), from: vec![], to: s.name.clone() }).collect();
        Workflow { name: "Software workflow".into(), statuses, transitions }
    }

    pub fn status(&self, name: &str) -> Option<&Status> {
        self.statuses.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn initial(&self) -> Option<&Status> {
        self.statuses.first()
    }

    /// Transitions available from `from`.
    pub fn available(&self, from: &str) -> Vec<&Transition> {
        self.transitions
            .iter()
            .filter(|t| !t.to.eq_ignore_ascii_case(from) && (t.from.is_empty() || t.from.iter().any(|f| f.eq_ignore_ascii_case(from))))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueType {
    pub name: String,
    pub subtask: bool,
}

impl IssueType {
    pub fn defaults() -> Vec<IssueType> {
        let t = |n: &str, subtask| IssueType { name: n.into(), subtask };
        vec![t("Epic", false), t("Story", false), t("Task", false), t("Bug", false), t("Sub-task", true)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectKind {
    Software,
    Business,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub key: ProjectKey,
    pub name: String,
    pub kind: ProjectKind,
    pub lead: Option<String>,
    pub description: Doc,
    pub issue_types: Vec<IssueType>,
    pub workflow: Workflow,
    /// The number the next issue gets. Numbers are never reused, even after deletes.
    pub next_number: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: Id,
    pub author: String,
    pub body: Doc,
    pub created: Timestamp,
    pub updated: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    Blocks,
    Relates,
    Duplicates,
    Clones,
}

impl LinkKind {
    pub fn parse(s: &str) -> Option<LinkKind> {
        Some(match s.to_ascii_lowercase().as_str() {
            "blocks" => LinkKind::Blocks,
            "relates" | "relates to" => LinkKind::Relates,
            "duplicates" => LinkKind::Duplicates,
            "clones" => LinkKind::Clones,
            _ => return None,
        })
    }
    pub fn outward(self) -> &'static str {
        match self {
            LinkKind::Blocks => "blocks",
            LinkKind::Relates => "relates to",
            LinkKind::Duplicates => "duplicates",
            LinkKind::Clones => "clones",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueLink {
    pub kind: LinkKind,
    pub target: IssueKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub key: IssueKey,
    pub issue_type: String,
    pub summary: String,
    pub description: Doc,
    pub status: String,
    pub priority: String,
    pub assignee: Option<String>,
    pub reporter: String,
    pub labels: Vec<String>,
    pub parent: Option<IssueKey>,
    pub story_points: Option<f64>,
    pub created: Timestamp,
    pub updated: Timestamp,
    /// When the issue last entered a Done-category status; cleared when it leaves.
    pub resolved: Option<Timestamp>,
    pub comments: Vec<Comment>,
    pub links: Vec<IssueLink>,
}

/// What `issue.create` takes.
#[derive(Debug, Clone, Default)]
pub struct NewIssue {
    pub project: String,
    pub issue_type: Option<String>,
    pub summary: String,
    pub description: Doc,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub parent: Option<String>,
    pub story_points: Option<f64>,
}

impl Workspace {
    pub fn project(&self, key: &str) -> Result<&Project> {
        let k = ProjectKey::parse(&key.to_ascii_uppercase()).map_err(|_| ModelError::NoSuchProject(key.to_string()))?;
        self.projects.get(&k).ok_or_else(|| ModelError::NoSuchProject(key.to_string()))
    }

    pub fn create_project(&mut self, key: &str, name: &str, kind: ProjectKind, lead: Option<&str>) -> Result<ProjectKey> {
        let key = ProjectKey::parse(key).map_err(|e| ModelError::Invalid(e.to_string()))?;
        if self.projects.contains_key(&key) {
            return Err(ModelError::Duplicate(format!("project {key}")));
        }
        let lead = match lead {
            Some(l) => Some(self.user(l)?.id.clone()),
            None => None,
        };
        let name = title(name, "the project name")?;
        let p = Project {
            key: key.clone(),
            name,
            kind,
            lead,
            description: Doc::default(),
            issue_types: IssueType::defaults(),
            workflow: Workflow::software(),
            next_number: 1,
        };
        self.projects.insert(key.clone(), p);
        Ok(key)
    }

    pub fn issue(&self, key: &str) -> Result<&Issue> {
        let k = IssueKey::parse(key).map_err(|_| ModelError::NoSuchIssue(key.to_string()))?;
        self.issues.get(&k).ok_or_else(|| ModelError::NoSuchIssue(key.to_string()))
    }

    fn issue_mut(&mut self, key: &str) -> Result<&mut Issue> {
        let k = IssueKey::parse(key).map_err(|_| ModelError::NoSuchIssue(key.to_string()))?;
        self.issues.get_mut(&k).ok_or_else(|| ModelError::NoSuchIssue(key.to_string()))
    }

    fn check_user(&self, u: Option<&str>) -> Result<Option<String>> {
        match u.map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => Ok(Some(self.user(u)?.id.clone())),
            None => Ok(None),
        }
    }

    pub fn create_issue(&mut self, new: NewIssue, reporter: &str, now: Timestamp) -> Result<IssueKey> {
        let reporter = self.user(reporter)?.id.clone();
        let assignee = self.check_user(new.assignee.as_deref())?;
        let summary = title(&new.summary, "the summary")?;
        let labels = labels(&new.labels)?;
        let priority = canonical_priority(new.priority.as_deref().unwrap_or("Medium"))?;
        let story_points = check_points(new.story_points)?;
        let project = self.project(&new.project)?;
        let issue_type = match &new.issue_type {
            Some(t) => project.issue_types.iter().find(|it| it.name.eq_ignore_ascii_case(t)),
            None => project.issue_types.iter().find(|it| it.name == "Task").or(project.issue_types.first()),
        }
        .ok_or_else(|| ModelError::Invalid(format!("project {} has no issue type {}", project.key, new.issue_type.unwrap_or_default())))?
        .clone();
        let status = project.workflow.initial().ok_or_else(|| ModelError::Invalid("the project's workflow has no statuses".into()))?.name.clone();
        let parent = match new.parent.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => Some(self.issue(p)?.key.clone()),
            None => None,
        };
        if issue_type.subtask && parent.is_none() {
            return Err(ModelError::Invalid(format!("a {} needs a parent issue", issue_type.name)));
        }
        let pkey = project.key.clone();
        let number = project.next_number;
        let key = IssueKey::new(pkey.clone(), number);
        if let Some(p) = self.projects.get_mut(&pkey) {
            p.next_number = number.saturating_add(1);
        }
        let issue = Issue {
            key: key.clone(),
            issue_type: issue_type.name,
            summary,
            description: new.description,
            status,
            priority,
            assignee,
            reporter,
            labels,
            parent,
            story_points,
            created: now,
            updated: now,
            resolved: None,
            comments: vec![],
            links: vec![],
        };
        self.issues.insert(key.clone(), issue);
        Ok(key)
    }

    pub fn set_summary(&mut self, key: &str, summary: &str, now: Timestamp) -> Result<()> {
        let s = title(summary, "the summary")?;
        let i = self.issue_mut(key)?;
        i.summary = s;
        i.updated = now;
        Ok(())
    }

    pub fn set_description(&mut self, key: &str, d: Doc, now: Timestamp) -> Result<()> {
        let i = self.issue_mut(key)?;
        i.description = d;
        i.updated = now;
        Ok(())
    }

    pub fn set_priority(&mut self, key: &str, p: &str, now: Timestamp) -> Result<()> {
        let p = canonical_priority(p)?;
        let i = self.issue_mut(key)?;
        i.priority = p;
        i.updated = now;
        Ok(())
    }

    pub fn set_labels(&mut self, key: &str, l: &[String], now: Timestamp) -> Result<()> {
        let l = labels(l)?;
        let i = self.issue_mut(key)?;
        i.labels = l;
        i.updated = now;
        Ok(())
    }

    pub fn set_story_points(&mut self, key: &str, points: Option<f64>, now: Timestamp) -> Result<()> {
        let points = check_points(points)?;
        let i = self.issue_mut(key)?;
        i.story_points = points;
        i.updated = now;
        Ok(())
    }

    pub fn assign(&mut self, key: &str, user: Option<&str>, now: Timestamp) -> Result<()> {
        let u = self.check_user(user)?;
        let i = self.issue_mut(key)?;
        i.assignee = u;
        i.updated = now;
        Ok(())
    }

    /// Moves an issue to `to` through the project's workflow.
    pub fn transition(&mut self, key: &str, to: &str, now: Timestamp) -> Result<()> {
        let issue = self.issue(key)?;
        let project = self.projects.get(&issue.key.project).ok_or_else(|| ModelError::NoSuchProject(issue.key.project.to_string()))?;
        let wf = &project.workflow;
        let target = wf.status(to).ok_or_else(|| ModelError::Invalid(format!("project {} has no status {to}", project.key)))?.clone();
        if !wf.available(&issue.status).iter().any(|t| t.to.eq_ignore_ascii_case(&target.name)) {
            return Err(ModelError::NoTransition { from: issue.status.clone(), to: target.name });
        }
        let i = self.issue_mut(key)?;
        i.status = target.name;
        i.resolved = if target.category == StatusCategory::Done { Some(now) } else { None };
        i.updated = now;
        Ok(())
    }

    pub fn status_category(&self, issue: &Issue) -> StatusCategory {
        self.projects.get(&issue.key.project).and_then(|p| p.workflow.status(&issue.status)).map_or(StatusCategory::ToDo, |s| s.category)
    }

    pub fn add_comment(&mut self, key: &str, author: &str, body: Doc, now: Timestamp) -> Result<Id> {
        let author = self.user(author)?.id.clone();
        if body.is_empty() {
            return Err(ModelError::Invalid("a comment can't be empty".into()));
        }
        self.issue(key)?;
        let id = self.ids.next_id();
        let i = self.issue_mut(key)?;
        i.comments.push(Comment { id, author, body, created: now, updated: now });
        i.updated = now;
        Ok(id)
    }

    pub fn delete_comment(&mut self, key: &str, id: Id) -> Result<()> {
        let i = self.issue_mut(key)?;
        let before = i.comments.len();
        i.comments.retain(|c| c.id != id);
        if i.comments.len() == before { Err(ModelError::NoSuchComment(id)) } else { Ok(()) }
    }

    pub fn link(&mut self, from: &str, kind: LinkKind, to: &str, now: Timestamp) -> Result<()> {
        let target = self.issue(to)?.key.clone();
        let src = self.issue(from)?.key.clone();
        if src == target {
            return Err(ModelError::Invalid("an issue can't link to itself".into()));
        }
        let i = self.issue_mut(from)?;
        let link = IssueLink { kind, target };
        if i.links.contains(&link) {
            return Err(ModelError::Duplicate(format!("link {src} {} {}", kind.outward(), link.target)));
        }
        i.links.push(link);
        i.updated = now;
        Ok(())
    }

    /// Deletes an issue, its sub-tasks, and links pointing at them.
    pub fn delete_issue(&mut self, key: &str) -> Result<Vec<IssueKey>> {
        let k = self.issue(key)?.key.clone();
        let mut gone: Vec<IssueKey> = self.issues.values().filter(|i| i.parent.as_ref() == Some(&k)).map(|i| i.key.clone()).collect();
        gone.push(k);
        for g in &gone {
            self.issues.remove(g);
        }
        for i in self.issues.values_mut() {
            i.links.retain(|l| !gone.contains(&l.target));
            if i.parent.as_ref().is_some_and(|p| gone.contains(p)) {
                i.parent = None;
            }
        }
        Ok(gone)
    }
}

fn canonical_priority(p: &str) -> Result<String> {
    PRIORITIES.iter().find(|q| q.eq_ignore_ascii_case(p.trim())).map(|q| (*q).to_string()).ok_or_else(|| {
        ModelError::Invalid(format!("unknown priority '{}' (use one of {})", p.chars().take(40).collect::<String>(), PRIORITIES.join(", ")))
    })
}

fn check_points(p: Option<f64>) -> Result<Option<f64>> {
    match p {
        Some(v) if !v.is_finite() || !(0.0..=1_000_000.0).contains(&v) => {
            Err(ModelError::Invalid("story points must be between 0 and 1,000,000".into()))
        }
        other => Ok(other),
    }
}
