//! The target feature catalog: Jira and Confluence features (by name — names only, never their
//! wording or artwork) with the command id that implements each. `cargo xtask parity` writes
//! `docs/parity-checklist.md` from this list; a test keeps every listed id real and the coverage
//! floor rising.
//!
//! An empty id means "not implemented yet". Add rows as features are observed; never delete a
//! row to raise the percentage.

pub struct Feature {
    pub product: &'static str,
    pub area: &'static str,
    pub name: &'static str,
    /// The implementing command, or `""`.
    pub command: &'static str,
}

const fn f(product: &'static str, area: &'static str, name: &'static str, command: &'static str) -> Feature {
    Feature { product, area, name, command }
}

const J: &str = "Jira";
const C: &str = "Confluence";

pub const FEATURES: &[Feature] = &[
    // Jira — projects
    f(J, "Projects", "Create project", "project.create"),
    f(J, "Projects", "Project list", "project.list"),
    f(J, "Projects", "Project details / settings", "project.get"),
    f(J, "Projects", "Project templates (Scrum, Kanban, business)", ""),
    f(J, "Projects", "Archive project", ""),
    f(J, "Projects", "Components", ""),
    f(J, "Projects", "Versions / releases", ""),
    // Jira — issues
    f(J, "Issues", "Create issue", "issue.create"),
    f(J, "Issues", "View issue", "issue.get"),
    f(J, "Issues", "Edit fields", "issue.edit"),
    f(J, "Issues", "Assign", "issue.assign"),
    f(J, "Issues", "Assign to me", "issue.assignToMe"),
    f(J, "Issues", "Delete issue", "issue.delete"),
    f(J, "Issues", "Sub-tasks", "issue.create"),
    f(J, "Issues", "Epics / parent hierarchy", "issue.create"),
    f(J, "Issues", "Issue links", "issue.link"),
    f(J, "Issues", "Labels", "issue.edit"),
    f(J, "Issues", "Priority", "issue.edit"),
    f(J, "Issues", "Story points / estimates", "issue.edit"),
    f(J, "Issues", "Comments", "issue.comment"),
    f(J, "Issues", "Delete comment", "issue.deleteComment"),
    f(J, "Issues", "Edit comment", "issue.editComment"),
    f(J, "Issues", "Change issue type", "issue.edit"),
    f(J, "Issues", "Change parent", "issue.edit"),
    f(J, "Issues", "Remove issue link", "issue.unlink"),
    f(J, "Issues", "Attachments", ""),
    f(J, "Issues", "Watchers", ""),
    f(J, "Issues", "Votes", ""),
    f(J, "Issues", "Work log / time tracking", ""),
    f(J, "Issues", "Due date", ""),
    f(J, "Issues", "Custom fields", ""),
    f(J, "Issues", "Clone issue", ""),
    f(J, "Issues", "Move issue between projects", ""),
    f(J, "Issues", "Bulk change", ""),
    f(J, "Issues", "Activity / change history", "issue.history"),
    f(J, "Issues", "@mentions", ""),
    // Jira — workflow
    f(J, "Workflow", "Transition issue", "issue.transition"),
    f(J, "Workflow", "List available transitions", "issue.transitions"),
    f(J, "Workflow", "Status categories", "issue.transition"),
    f(J, "Workflow", "Resolution on Done", "issue.transition"),
    f(J, "Workflow", "Workflow editor (statuses, transitions)", ""),
    f(J, "Workflow", "Transition conditions / validators / post functions", ""),
    f(J, "Workflow", "Automation rules", ""),
    // Jira — search
    f(J, "Search", "JQL search", "search.jql"),
    f(J, "Search", "ORDER BY", "search.jql"),
    f(J, "Search", "currentUser() / date functions", "search.jql"),
    f(J, "Search", "Relative dates (-7d)", "search.jql"),
    f(J, "Search", "Text search (text ~)", "search.jql"),
    f(J, "Search", "Saved filters", ""),
    f(J, "Search", "WAS / CHANGED history operators", "search.jql"),
    f(J, "Search", "Basic (non-JQL) search builder", ""),
    f(J, "Search", "Export results (CSV)", ""),
    // Jira — agile
    f(J, "Boards", "Board view by status", "board.view"),
    f(J, "Boards", "Board quick filters", "board.view"),
    f(J, "Boards", "Drag cards between columns", "issue.transition"),
    f(J, "Boards", "Backlog", "board.backlog"),
    f(J, "Boards", "Sprints (create, start, complete)", "sprint.start"),
    f(J, "Boards", "Board column configuration / WIP limits", ""),
    f(J, "Boards", "Swimlanes", ""),
    f(J, "Boards", "Rank (drag order)", "issue.rank"),
    f(J, "Reports", "Burndown chart", ""),
    f(J, "Reports", "Velocity chart", ""),
    f(J, "Reports", "Cumulative flow diagram", ""),
    f(J, "Reports", "Dashboards and gadgets", ""),
    f(J, "Planning", "Timeline / roadmap", ""),
    // Confluence — spaces
    f(C, "Spaces", "Create space", "space.create"),
    f(C, "Spaces", "Space directory", "space.list"),
    f(C, "Spaces", "Page tree sidebar", "space.tree"),
    f(C, "Spaces", "Space home page", "space.create"),
    f(C, "Spaces", "Space permissions", ""),
    f(C, "Spaces", "Archive space", ""),
    // Confluence — pages
    f(C, "Pages", "Create page", "page.create"),
    f(C, "Pages", "Child pages", "page.create"),
    f(C, "Pages", "View page", "page.get"),
    f(C, "Pages", "Edit page", "page.edit"),
    f(C, "Pages", "Move page", "page.move"),
    f(C, "Pages", "Delete page (with children)", "page.delete"),
    f(C, "Pages", "Page labels", "page.labels"),
    f(C, "Pages", "Page history", "page.history"),
    f(C, "Pages", "Restore version", "page.restore"),
    f(C, "Pages", "Conflict detection on save", "page.edit"),
    f(C, "Pages", "Compare versions (diff)", ""),
    f(C, "Pages", "Copy page", ""),
    f(C, "Pages", "Page templates and blueprints", ""),
    f(C, "Pages", "Live collaborative editing", ""),
    f(C, "Pages", "Inline comments", ""),
    f(C, "Pages", "Page comments", ""),
    f(C, "Pages", "Attachments and images", ""),
    f(C, "Pages", "Drafts", ""),
    f(C, "Pages", "Page restrictions", ""),
    f(C, "Pages", "Export to PDF / Word", ""),
    f(C, "Pages", "Blog posts", ""),
    f(C, "Editor", "Headings, lists, code blocks, quotes", "page.edit"),
    f(C, "Editor", "Tables", ""),
    f(C, "Editor", "Panels (info, note, warning)", ""),
    f(C, "Editor", "Macros: table of contents, children, Jira issues", ""),
    f(C, "Editor", "Task lists", ""),
    f(C, "Search", "Full-text page search", "page.search"),
    f(C, "Search", "CQL", ""),
    // Shared
    f("Both", "Users", "Users", "user.list"),
    f("Both", "Users", "Add user", "user.add"),
    f("Both", "Users", "Edit user profile (name, email)", "user.edit"),
    f("Both", "Users", "Groups and roles", ""),
    f("Both", "Users", "Permission schemes", ""),
    f("Both", "Platform", "Undo / redo", "edit.undo"),
    f("Both", "Platform", "Command palette", "app.commands"),
    f("Both", "Platform", "Save / open workspace", "file.save"),
    f("Both", "Platform", "Rename workspace", "workspace.rename"),
    f("Both", "Platform", "Notifications (in-app, email)", ""),
    f("Both", "Platform", "Jira issue ↔ page links", ""),
    f("Both", "Platform", "Jira REST API v3 compatibility", ""),
    f("Both", "Platform", "Confluence REST API v2 compatibility", ""),
    f("Both", "Platform", "Import Jira CSV / JSON export", ""),
    f("Both", "Platform", "Import Confluence space export", ""),
    f("Both", "Platform", "Webhooks", ""),
    f("Both", "Platform", "Multi-user server with real-time sync", ""),
];

/// `(implemented, total)`.
pub fn coverage() -> (usize, usize) {
    (FEATURES.iter().filter(|f| !f.command.is_empty()).count(), FEATURES.len())
}
