//! One issue as a table row (`egui_extras` [`TableRow`]): the list's columns, drawn the same
//! way wherever issues are listed. The row's first cell carries its accessible name
//! ("Open LUME-12"); the key is a link.

use egui::{RichText, Ui};
use egui_extras::{Column, TableRow};
use trackcraft_model::{Issue, Timestamp, Workspace, priority_rank};

use super::{name_of, pill, who};
use crate::icons;
use crate::theme::Tokens;

/// A column of the issue table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Col {
    Type,
    Key,
    Summary,
    Status,
    Priority,
    Assignee,
    Updated,
    Reporter,
    Created,
    Labels,
    Points,
}

impl Col {
    /// Every column, in display order.
    pub(crate) const ALL: [Col; 11] = [
        Col::Type,
        Col::Key,
        Col::Summary,
        Col::Status,
        Col::Priority,
        Col::Assignee,
        Col::Reporter,
        Col::Labels,
        Col::Points,
        Col::Created,
        Col::Updated,
    ];

    /// Header text (empty for the type icon column).
    pub(crate) fn header(self) -> &'static str {
        match self {
            Col::Type => "",
            Col::Key => "Key",
            Col::Summary => "Summary",
            Col::Status => "Status",
            Col::Priority => "Priority",
            Col::Assignee => "Assignee",
            Col::Updated => "Updated",
            Col::Reporter => "Reporter",
            Col::Created => "Created",
            Col::Labels => "Labels",
            Col::Points => "Story points",
        }
    }

    /// The JQL field the header sorts by.
    pub(crate) fn field(self) -> &'static str {
        match self {
            Col::Type => "type",
            Col::Key => "key",
            Col::Summary => "summary",
            Col::Status => "status",
            Col::Priority => "priority",
            Col::Assignee => "assignee",
            Col::Updated => "updated",
            Col::Reporter => "reporter",
            Col::Created => "created",
            Col::Labels => "labels",
            Col::Points => "storypoints",
        }
    }

    /// Always shown (not in the column chooser).
    pub(crate) fn fixed(self) -> bool {
        matches!(self, Col::Type | Col::Key | Col::Summary)
    }

    /// Shown until the chooser says otherwise.
    pub(crate) fn default_on(self) -> bool {
        matches!(self, Col::Type | Col::Key | Col::Summary | Col::Status | Col::Priority | Col::Assignee | Col::Updated)
    }

    pub(crate) fn bit(self) -> u32 {
        1u32 << (self as u32)
    }

    /// The column's sizing in the table (all clip, so nothing spills into a neighbour).
    pub(crate) fn column(self) -> Column {
        let c = match self {
            Col::Type => Column::exact(18.0).resizable(false),
            Col::Key => Column::initial(84.0).at_least(60.0),
            Col::Summary => Column::remainder().at_least(140.0),
            Col::Status => Column::initial(118.0).at_least(60.0),
            Col::Priority => Column::initial(96.0).at_least(60.0),
            Col::Assignee => Column::initial(150.0).at_least(60.0),
            Col::Reporter => Column::initial(140.0).at_least(60.0),
            Col::Labels => Column::initial(130.0).at_least(50.0),
            Col::Points => Column::initial(90.0).at_least(40.0),
            Col::Created | Col::Updated => Column::initial(100.0).at_least(60.0),
        };
        c.clip(true)
    }
}

/// What every row needs besides the issue.
pub(crate) struct RowCx<'a> {
    pub t: &'a Tokens,
    pub ws: &'a Workspace,
    pub now: Timestamp,
}

/// Draws `issue` across `cols` in `row`. The returned response is clicked when any part of the
/// row (or its key link) was clicked.
pub(crate) fn issue_row(row: &mut TableRow<'_, '_>, cx: &RowCx<'_>, issue: &Issue, cols: &[Col]) -> Option<egui::Response> {
    let t = cx.t;
    let mut all: Option<egui::Response> = None;
    let mut add = |r: egui::Response| {
        all = Some(match all.take() {
            Some(a) => a.union(r),
            None => r,
        });
    };
    let mut link: Option<egui::Response> = None;
    for &c in cols {
        let (_, r) = row.col(|ui: &mut Ui| match c {
            Col::Type => {
                icons::issue_type(ui, &issue.issue_type, 16.0);
            }
            Col::Key => {
                link = Some(ui.link(RichText::new(issue.key.to_string()).monospace()));
            }
            Col::Summary => {
                // Truncated to the column with an ellipsis; the full text shows on hover.
                ui.add(egui::Label::new(&issue.summary).truncate());
            }
            Col::Status => {
                // Its own row, so the pill keeps its height instead of filling the cell.
                ui.horizontal(|ui| pill(ui, t, cx.ws.status_category(issue), &issue.status));
            }
            Col::Priority => {
                icons::priority(ui, t, &issue.priority, priority_rank(&issue.priority).unwrap_or(3), 14.0);
                ui.add(egui::Label::new(&issue.priority).truncate());
            }
            Col::Assignee => person(ui, cx, issue.assignee.as_deref()),
            Col::Reporter => person(ui, cx, Some(&issue.reporter)),
            Col::Updated => muted(ui, t, crate::ago(cx.now, issue.updated)),
            Col::Created => muted(ui, t, crate::ago(cx.now, issue.created)),
            Col::Labels => muted(ui, t, issue.labels.join(", ")),
            Col::Points => muted(ui, t, issue.story_points.map(|p| p.to_string()).unwrap_or_default()),
        });
        add(r);
    }
    if let Some(l) = link {
        add(l);
    }
    let key = issue.key.to_string();
    if let Some(r) = &all {
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Open {key}")));
    }
    all
}

fn person(ui: &mut Ui, cx: &RowCx<'_>, id: Option<&str>) {
    who(ui, cx.t, cx.ws, id, 18.0);
    ui.add(egui::Label::new(name_of(cx.ws, id)).truncate());
}

fn muted(ui: &mut Ui, t: &Tokens, text: String) {
    ui.add(egui::Label::new(RichText::new(text).color(t.muted)).truncate());
}
