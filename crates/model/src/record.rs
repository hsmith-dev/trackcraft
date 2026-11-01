//! Issues as JQL records. Field names follow Jira's documented JQL fields.

use trackcraft_core::Id;
use trackcraft_jql::{FieldChange, FieldHistory, FieldValue, Record};

use crate::{FieldDef, Issue, SprintState, Workspace, field_id_of, priority_rank};

pub struct IssueRecord<'a> {
    pub ws: &'a Workspace,
    pub issue: &'a Issue,
}

impl Record for IssueRecord<'_> {
    fn field(&self, name: &str) -> FieldValue {
        let i = self.issue;
        let text = |s: &str| FieldValue::Text(s.to_string());
        match name.to_ascii_lowercase().as_str() {
            "project" => text(i.key.project.as_str()),
            "key" | "issuekey" | "id" => text(&i.key.to_string()),
            "summary" => text(&i.summary),
            "description" => text(&i.description.plain_text()),
            "comment" => FieldValue::Text(i.comments.iter().map(|c| c.body.plain_text()).collect::<Vec<_>>().join("\n")),
            "status" => text(&i.status),
            "statuscategory" => {
                let c = self.ws.status_category(i);
                FieldValue::Ranked { name: c.name().to_string(), rank: c.rank() }
            }
            "type" | "issuetype" => text(&i.issue_type),
            "priority" => FieldValue::Ranked { name: i.priority.clone(), rank: priority_rank(&i.priority).unwrap_or(i64::MAX) },
            "assignee" => i.assignee.as_deref().map_or(FieldValue::None, text),
            "reporter" => text(&i.reporter),
            "labels" => FieldValue::Many(i.labels.clone()),
            "parent" => i.parent.as_ref().map_or(FieldValue::None, |p| text(&p.to_string())),
            "story points" | "storypoints" => i.story_points.map_or(FieldValue::None, FieldValue::Number),
            "created" | "createddate" => FieldValue::Time(i.created),
            "updated" | "updateddate" => FieldValue::Time(i.updated),
            "resolved" | "resolutiondate" => i.resolved.map_or(FieldValue::None, FieldValue::Time),
            // Rank sorts byte-wise (`ORDER BY rank`); ranks are base-36 keys.
            "rank" => FieldValue::Lexical(i.rank.clone()),
            // The current sprint and closed sprints the issue was in, each as id and name, so
            // `sprint = 7` and `sprint = "LUME Sprint 1"` both match.
            "sprint" => FieldValue::Many(
                i.past_sprints
                    .iter()
                    .chain(i.sprint.iter())
                    .filter_map(|id| self.ws.sprints.get(id))
                    .flat_map(|s| [s.id.0.to_string(), s.name.clone()])
                    .collect(),
            ),
            "issuelinktype" => FieldValue::Many(i.links.iter().map(|l| l.kind.outward().to_string()).collect()),
            // M2.4: components and versions match by name or id (`component = Docs`, `fixVersion = 12`).
            "component" | "components" => self.named(&i.components, |d, id| d.component(id).map(|c| c.name.clone())),
            "fixversion" | "fixversions" => self.named(&i.fix_versions, |d, id| d.version(id).map(|v| v.name.clone())),
            "affectedversion" | "affectsversion" | "affectsversions" | "versions" => {
                self.named(&i.affects_versions, |d, id| d.version(id).map(|v| v.name.clone()))
            }
            _ => self.custom(name),
        }
    }

    /// Value history for `WAS`/`CHANGED`: status, assignee and priority.
    fn history(&self, name: &str) -> Option<FieldHistory> {
        let i = self.issue;
        let (field, current) = match name.to_ascii_lowercase().as_str() {
            "status" => ("status", Some(i.status.clone())),
            "assignee" => ("assignee", i.assignee.clone()),
            "priority" => ("priority", Some(i.priority.clone())),
            _ => return None,
        };
        let changes: Vec<FieldChange> = i
            .history
            .iter()
            .filter(|c| c.field == field)
            .map(|c| FieldChange { at: c.at, by: c.author.clone(), from: c.from.clone(), to: c.to.clone() })
            .collect();
        let initial = match changes.first() {
            Some(c) => c.from.clone(),
            None => current,
        };
        let created_by = i.history.iter().find(|c| c.field == "created").map_or_else(|| i.reporter.clone(), |c| c.author.clone());
        Some(FieldHistory { created: Some(i.created), created_by: Some(created_by), initial, changes })
    }

    /// `openSprints()` (future and active) and `closedSprints()`: sprint ids of the issue's
    /// project. M2.4: `unreleasedVersions()` / `releasedVersions()`: version ids of the named
    /// projects (`releasedVersions(LUME)`), or of the issue's project without arguments.
    fn function(&self, name: &str, args: &[String]) -> Option<Vec<String>> {
        let lower = name.to_ascii_lowercase();
        if lower == "unreleasedversions" || lower == "releasedversions" {
            let released = lower == "releasedversions";
            let mine = self.issue.key.project.as_str();
            let wanted = |k: &str| if args.is_empty() { k == mine } else { args.iter().any(|a| a.trim().eq_ignore_ascii_case(k)) };
            return Some(
                self.ws
                    .fields
                    .iter()
                    .filter(|(k, _)| wanted(k.as_str()))
                    .flat_map(|(_, d)| d.versions.iter().filter(|v| v.released == released).map(|v| v.id.0.to_string()))
                    .collect(),
            );
        }
        let open = match lower.as_str() {
            "opensprints" => true,
            "closedsprints" => false,
            _ => return None,
        };
        let project = &self.issue.key.project;
        Some(
            self.ws
                .sprints
                .values()
                .filter(|s| &s.project == project && (s.state != SprintState::Closed) == open)
                .map(|s| s.id.0.to_string())
                .collect(),
        )
    }

    fn text(&self) -> String {
        let i = self.issue;
        let mut s = format!("{}\n{}", i.summary, i.description.plain_text());
        for c in &i.comments {
            s.push('\n');
            s.push_str(&c.body.plain_text());
        }
        s
    }
}

impl IssueRecord<'_> {
    /// Ids and names of components or versions, so both match.
    fn named(&self, ids: &[Id], name: impl Fn(&crate::ProjectFields, Id) -> Option<String>) -> FieldValue {
        let Some(d) = self.ws.fields.get(&self.issue.key.project) else { return FieldValue::Many(vec![]) };
        FieldValue::Many(ids.iter().filter_map(|id| name(d, *id).map(|n| [id.0.to_string(), n])).flatten().collect())
    }

    /// M2.4: a custom field of the issue's project, by `cf[ID]` or by name.
    fn custom(&self, name: &str) -> FieldValue {
        let Some(d) = self.ws.fields.get(&self.issue.key.project) else { return FieldValue::None };
        let lower = name.trim().to_ascii_lowercase();
        let def: Option<&FieldDef> = match lower.strip_prefix("cf[").and_then(|r| r.strip_suffix(']')) {
            Some(_) => field_id_of(&lower).and_then(|id| d.field(id)),
            None => d.fields.iter().find(|f| f.name.eq_ignore_ascii_case(name.trim())),
        };
        let Some(def) = def else { return FieldValue::None };
        match self.issue.custom.get(&def.id) {
            None => FieldValue::None,
            Some(crate::FieldValue::Text(s) | crate::FieldValue::Url(s) | crate::FieldValue::User(s)) => FieldValue::Text(s.clone()),
            Some(crate::FieldValue::Number(n)) => FieldValue::Number(*n),
            Some(crate::FieldValue::Date(day)) => day.and_hms_opt(0, 0, 0).map_or(FieldValue::None, |t| FieldValue::Time(t.and_utc())),
            // Selects sort in option order (`ORDER BY "Severity"`), compare by name.
            Some(crate::FieldValue::Select(s)) => {
                let rank = def.kind.options().iter().position(|o| o == s).and_then(|p| i64::try_from(p).ok()).unwrap_or(i64::MAX);
                FieldValue::Ranked { name: s.clone(), rank }
            }
            Some(crate::FieldValue::MultiSelect(v)) => FieldValue::Many(v.clone()),
        }
    }
}
