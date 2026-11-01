//! Issues as JQL records. Field names follow Jira's documented JQL fields.

use trackcraft_jql::{FieldValue, Record};

use crate::{Issue, Workspace, priority_rank};

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
            "issuelinktype" => FieldValue::Many(i.links.iter().map(|l| l.kind.outward().to_string()).collect()),
            _ => FieldValue::None,
        }
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
