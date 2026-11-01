//! The issue view: summary, description, sub-tasks, links, activity and the fields column.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::IssueKey;
use trackcraft_model::{Change, Issue, LinkKind, PRIORITIES, StatusCategory, Workspace, priority_rank};

use crate::theme::Tokens;
use crate::widgets::{
    Candidate, card, empty_state, field_row, issue_picker, label_editor, link_truncated, name_of, number_field, pill, remove_button, short,
    user_picker, who,
};

/// Comments drawn at first, and added per "Show older" click (UI-6: 500-comment issues).
const COMMENT_PAGE: usize = 50;
/// Description blocks drawn before "Show the whole description" (UI-6: huge descriptions).
const DESCRIPTION_BLOCKS: usize = 30;
use crate::{Action, ComboBox, Then, TrackApp, View, icons, markup_view};

pub(crate) fn detail(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &IssueKey) {
    // UI-6: long activity and descriptions scroll instead of running off the window.
    egui::ScrollArea::vertical().id_salt("issue-detail").auto_shrink([false, false]).show(ui, |ui| detail_body(app, ui, t, key));
}

fn detail_body(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, key: &IssueKey) {
    let Some(issue) = app.session.ws.issues.get(key).cloned() else {
        ui.label("This issue no longer exists.");
        return;
    };
    let k = key.to_string();
    let ws_now = app.session.now();
    let project_name = app.session.ws.projects.get(&key.project).map(|p| p.name.clone()).unwrap_or_default();
    let cat = app.session.ws.status_category(&issue);
    let transitions: Vec<String> = app
        .session
        .ws
        .projects
        .get(&key.project)
        .map(|p| p.workflow.available(&issue.status).iter().map(|t| t.to.clone()).collect())
        .unwrap_or_default();

    let parent_exists = issue.parent.as_ref().is_some_and(|p| app.session.ws.issues.contains_key(p));
    let can_edit = crate::widgets::can(app, "issue.edit", &json!({"key": k})); // UI-37
    ui.horizontal(|ui| {
        // At most half the row for a 255-character project name.
        let w = ui.available_width() * 0.5;
        ui.allocate_ui(egui::vec2(w, ui.spacing().interact_size.y), |ui| {
            if link_truncated(ui, RichText::new(&project_name).color(t.muted)).clicked() {
                app.push(Action::Nav(View::Board(key.project.clone())));
            }
        });
        ui.label(RichText::new("/").color(t.muted));
        if let Some(p) = &issue.parent {
            if parent_exists {
                if ui.link(RichText::new(p.to_string()).color(t.muted)).clicked() {
                    app.push(Action::Nav(View::Issue(p.clone())));
                }
            } else {
                // UI-6: a deleted parent is plain struck text, not a link.
                ui.label(RichText::new(p.to_string()).color(t.muted).strikethrough()).on_hover_text("This parent issue no longer exists");
            }
            ui.label(RichText::new("/").color(t.muted));
        }
        icons::issue_type(ui, &issue.issue_type, 16.0);
        ui.label(RichText::new(&k).monospace().color(t.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::inbox::watch_toggle(app, ui, trackcraft_model::Target::Issue(key.clone()))
        }); // M4.5
    });
    ui.add_space(4.0);

    // Summary: click to edit.
    match app.issue_ui.editing_summary.as_mut() {
        Some(s) => {
            let r = ui.add(egui::TextEdit::singleline(s).font(egui::FontId::proportional(22.0)).desired_width(f32::INFINITY));
            r.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let v = s.clone();
                app.issue_ui.editing_summary = None;
                app.push(Action::Run { id: "issue.edit", params: json!({"key": k, "summary": v}), then: Then::Nothing });
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.issue_ui.editing_summary = None;
            }
        }
        None => {
            let r = ui
                .add(egui::Label::new(RichText::new(&issue.summary).size(22.0).strong()).sense(egui::Sense::click()))
                .on_hover_text(if can_edit { "Click to edit".to_string() } else { crate::widgets::denied("edit this issue") }); // UI-37
            if r.clicked() && can_edit {
                app.issue_ui.editing_summary = Some(issue.summary.clone());
            }
        }
    }
    ui.add_space(10.0);

    let avail = ui.available_width();
    let right_w = 300.0f32.min(avail * 0.4);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(avail - right_w - 20.0);
            main_column(app, ui, t, &issue, &k, ws_now);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(right_w);
            // UI-6: long names (an unbreakable display name, a raw user id) truncate in the
            // fields column instead of pushing it past the window edge.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            fields(app, ui, t, &issue, &k, cat, &transitions);
        });
    });
}

fn main_column(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str, now: trackcraft_model::Timestamp) {
    let mut acts: Vec<Action> = Vec::new();
    let can_edit = crate::widgets::can(app, "issue.edit", &json!({"key": k})); // UI-37
    ui.horizontal(|ui| {
        ui.label(RichText::new("Description").strong());
        if app.issue_ui.editing_description.is_none()
            && ui
                .add_enabled(can_edit, egui::Button::new("Edit").small())
                .on_hover_text("Edit the description (E)")
                .on_disabled_hover_text(crate::widgets::denied("edit this issue"))
                .clicked()
        {
            app.issue_ui.editing_description = Some(issue.description.to_markdown());
        }
    });
    let mut end_edit = false;
    match app.issue_ui.editing_description.as_mut() {
        Some(text) => {
            let r = ui.add(
                egui::TextEdit::multiline(text)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Markdown: **bold**, *italic*, - lists, ## headings"),
            );
            if std::mem::take(&mut app.issue_ui.focus_description) {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    end_edit = true;
                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "description": text.clone()}), then: Then::Nothing });
                }
                if ui.button("Cancel").clicked() {
                    end_edit = true;
                }
            });
        }
        None => {
            let full_id = egui::Id::unique(("issue-full-description", k));
            let full = ui.data(|d| d.get_temp::<bool>(full_id)).unwrap_or(false);
            let n = issue.description.blocks.len();
            let mut expand = false;
            card(ui, t, "", |ui| {
                if full || n <= DESCRIPTION_BLOCKS {
                    markup_view::show(ui, t, &issue.description);
                } else {
                    let head = trackcraft_markup::Doc { blocks: issue.description.blocks.iter().take(DESCRIPTION_BLOCKS).cloned().collect() };
                    markup_view::show(ui, t, &head);
                    expand = ui.button(format!("Show the whole description ({} more blocks)", n - DESCRIPTION_BLOCKS)).clicked();
                }
            });
            if expand {
                ui.data_mut(|d| d.insert_temp(full_id, true));
            }
        }
    }
    if end_edit {
        app.issue_ui.editing_description = None;
    }
    ui.add_space(14.0);

    let ws = &app.session.ws;
    let children: Vec<&Issue> = ws.issues.values().filter(|i| i.parent.as_ref() == Some(&issue.key)).collect();
    if !children.is_empty() || issue.issue_type != "Sub-task" {
        ui.horizontal(|ui| {
            ui.label(RichText::new(if issue.issue_type == "Epic" { "Child issues" } else { "Sub-tasks" }).strong());
            // The create dialog takes this issue as the parent (see `new_form`).
            if issue.issue_type != "Sub-task" && ui.small_button("+ Add").clicked() {
                acts.push(Action::OpenCreate(Some(issue.key.project.to_string())));
            }
        });
        for c in &children {
            let cat = ws.status_category(c);
            ui.horizontal(|ui| {
                icons::issue_type(ui, &c.issue_type, 15.0);
                if ui.link(c.key.to_string()).clicked() {
                    acts.push(Action::Nav(View::Issue(c.key.clone())));
                }
                pill(ui, t, cat, &c.status);
                ui.add(egui::Label::new(&c.summary).truncate());
            });
        }
        ui.add_space(10.0);
    }

    links(app, ui, t, issue, k, &mut acts);
    ui.add_space(10.0);
    activity(app, ui, t, issue, k, now, &mut acts);
    for a in acts {
        app.push(a);
    }
}

fn inverse(outward: &str) -> String {
    match outward {
        "blocks" => "is blocked by",
        "duplicates" => "is duplicated by",
        "clones" => "is cloned by",
        other => other,
    }
    .to_string()
}

// ---- Links (UI-9) ----

/// One row of "Linked issues": the relation as read from this issue, and the other issue.
struct LinkRow {
    rel: String,
    kind: LinkKind,
    other: IssueKey,
    /// The link is stored on this issue (else on `other`, pointing here).
    outward: bool,
}

/// Link relations offered in the form (the first is the default): label, kind, and whether this
/// issue is the source.
const RELATIONS: [(&str, LinkKind, bool); 7] = [
    ("relates to", LinkKind::Relates, true),
    ("blocks", LinkKind::Blocks, true),
    ("is blocked by", LinkKind::Blocks, false),
    ("duplicates", LinkKind::Duplicates, true),
    ("is duplicated by", LinkKind::Duplicates, false),
    ("clones", LinkKind::Clones, true),
    ("is cloned by", LinkKind::Clones, false),
];

/// The issue view's per-issue edit state (UI-9). Reset when the view changes.
#[derive(Debug, Default)]
pub(crate) struct Polish {
    /// The open "Link issue" form.
    pub link_form: Option<LinkForm>,
    /// The comment being edited, and its text.
    pub editing_comment: Option<(trackcraft_core::Id, String)>,
    /// Links from other issues to this one, built once per workspace revision (gap G7).
    inward: Option<(u64, IssueKey, Vec<(LinkKind, IssueKey)>)>,
    /// Labels used anywhere in the workspace, built once per revision.
    labels: Option<(u64, Vec<String>)>,
}

/// The inline "Link issue" form: the relation (index into [`RELATIONS`]) and the other issue.
#[derive(Debug, Default)]
pub(crate) struct LinkForm {
    pub rel: usize,
    pub target: Option<IssueKey>,
}

/// Links that point at `key` from other issues: rebuilt only when the workspace changed.
fn inward_links(p: &mut Polish, rev: u64, ws: &Workspace, key: &IssueKey) -> Vec<(LinkKind, IssueKey)> {
    if let Some((r, k, v)) = &p.inward
        && *r == rev
        && k == key
    {
        return v.clone();
    }
    let v: Vec<(LinkKind, IssueKey)> =
        ws.issues.values().flat_map(|o| o.links.iter().filter(|l| &l.target == key).map(move |l| (l.kind, o.key.clone()))).collect();
    p.inward = Some((rev, key.clone(), v.clone()));
    v
}

/// Every label used in the workspace, sorted (at most 5000): rebuilt only when the workspace
/// changed.
pub(crate) fn workspace_labels(cache: &mut Option<(u64, Vec<String>)>, rev: u64, ws: &Workspace) -> Vec<String> {
    if let Some((r, v)) = cache
        && *r == rev
    {
        return v.clone();
    }
    let set: std::collections::BTreeSet<&String> = ws.issues.values().flat_map(|i| i.labels.iter()).collect();
    let v: Vec<String> = set.into_iter().take(5000).cloned().collect();
    *cache = Some((rev, v.clone()));
    v
}

fn links(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str, acts: &mut Vec<Action>) {
    let rev = app.session.revision();
    let inward = inward_links(&mut app.issue_ui.polish, rev, &app.session.ws, &issue.key);
    let rows: Vec<LinkRow> = issue
        .links
        .iter()
        .map(|l| LinkRow { rel: l.kind.outward().to_string(), kind: l.kind, other: l.target.clone(), outward: true })
        .chain(inward.iter().map(|(kind, o)| LinkRow { rel: inverse(kind.outward()), kind: *kind, other: o.clone(), outward: false }))
        .collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Linked issues").strong());
        if app.issue_ui.polish.link_form.is_none() && ui.small_button("Link issue").on_hover_text("Link this issue to another").clicked() {
            app.issue_ui.polish.link_form = Some(LinkForm::default());
        }
    });
    if app.issue_ui.polish.link_form.is_some() {
        link_form(app, ui, t, issue, k, acts);
    }
    let ws = &app.session.ws;
    if rows.is_empty() && app.issue_ui.polish.link_form.is_none() {
        ui.label(RichText::new("No linked issues").color(t.muted));
    }
    for row in rows {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&row.rel).color(t.muted));
            if ui.link(row.other.to_string()).clicked() {
                acts.push(Action::Nav(View::Issue(row.other.clone())));
            }
            // The remove cross and the status stay at the right; the summary takes what's left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let name = format!("Remove link: {} {}", row.rel, row.other);
                if remove_button(ui, t, &name, 18.0).clicked() {
                    let (from, to) = if row.outward { (k.to_string(), row.other.to_string()) } else { (row.other.to_string(), k.to_string()) };
                    acts.push(Action::Run {
                        id: "issue.unlink",
                        params: json!({"key": from, "type": row.kind.outward(), "target": to}),
                        then: Then::Toast("Link removed".into()),
                    });
                }
                if let Some(o) = ws.issues.get(&row.other) {
                    pill(ui, t, ws.status_category(o), &o.status);
                    // The summary sits next to the key, cut to the room left.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(&o.summary).truncate());
                    });
                }
            });
        });
    }
}

fn link_form(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str, acts: &mut Vec<Action>) {
    let ws = &app.session.ws;
    let Some(form) = app.issue_ui.polish.link_form.as_mut() else { return };
    let mut close = false;
    egui::Frame::new().fill(t.surface_alt).corner_radius(6).inner_margin(8).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let l_rel = ui.label(RichText::new("This issue").color(t.muted)).id;
            let rel = RELATIONS.get(form.rel).map_or("relates to", |r| r.0);
            ComboBox::from_id_salt("link-type")
                .selected_text(rel)
                .width(130.0)
                .show_ui(ui, |ui| {
                    for (i, (name, _, _)) in RELATIONS.iter().enumerate() {
                        if ui.selectable_label(form.rel == i, *name).clicked() {
                            form.rel = i;
                        }
                    }
                })
                .response
                .labelled_by(l_rel);
            let l_target = ui.label(RichText::new("Issue").color(t.muted)).id;
            let selected = match &form.target {
                Some(tk) => ws.issues.get(tk).map_or_else(|| tk.to_string(), |o| short(tk, &o.summary, 24)),
                None => "Choose an issue".to_string(),
            };
            let me = issue.key.clone();
            let picked = issue_picker(
                ui,
                "link-target",
                selected,
                || {
                    ws.issues
                        .values()
                        .filter(|o| o.key != me)
                        .map(|o| Candidate { key: o.key.clone(), summary: o.summary.clone(), issue_type: o.issue_type.clone() })
                        .collect()
                },
                form.target.as_ref(),
                false,
                l_target,
                220.0,
            );
            if let Some(Some(tk)) = picked.choice {
                form.target = Some(tk);
            }
        });
        ui.horizontal(|ui| {
            let ready = form.target.is_some();
            let add = ui.add_enabled(ready, egui::Button::new(RichText::new("Link").strong())).on_hover_text("Add the link");
            if add.clicked()
                && let Some(target) = &form.target
                && let Some((_, kind, outward)) = RELATIONS.get(form.rel)
            {
                let (from, to) = if *outward { (k.to_string(), target.to_string()) } else { (target.to_string(), k.to_string()) };
                acts.push(Action::Run {
                    id: "issue.link",
                    params: json!({"key": from, "type": kind.outward(), "target": to}),
                    then: Then::Toast("Link added".into()),
                });
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if close {
        app.issue_ui.polish.link_form = None;
    }
}

// ---- Activity (UI-13) ----

/// The activity tabs (UI-13).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ActivityTab {
    #[default]
    Comments,
    History,
    All,
}

impl ActivityTab {
    pub(crate) fn parse(s: &str) -> Option<ActivityTab> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "comments" => ActivityTab::Comments,
            "history" => ActivityTab::History,
            "all" => ActivityTab::All,
            _ => return None,
        })
    }
}

/// One entry of the "All" tab: a comment or one command's changes.
enum Entry<'a> {
    Comment(&'a trackcraft_model::Comment),
    Changes(&'a [Change]),
}

impl Entry<'_> {
    fn at(&self) -> trackcraft_model::Timestamp {
        match self {
            Entry::Comment(c) => c.created,
            Entry::Changes(g) => g.first().map(|c| c.at).unwrap_or_default(),
        }
    }
}

fn activity(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str, now: trackcraft_model::Timestamp, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Activity").strong());
        ui.add_space(12.0);
        for (tab, name) in [(ActivityTab::Comments, "Comments"), (ActivityTab::History, "History"), (ActivityTab::All, "All")] {
            if ui.selectable_label(app.activity_tab == tab, name).clicked() {
                app.activity_tab = tab;
            }
        }
    });
    ui.add_space(4.0);
    let tab = app.activity_tab;
    if tab != ActivityTab::History {
        comment_box(app, ui, t, k, acts);
    }
    ui.add_space(6.0);
    // UI-6: newest first, COMMENT_PAGE at a time.
    let shown_id = egui::Id::unique(("issue-activity-shown", k, tab as u8));
    let shown = ui.data(|d| d.get_temp::<usize>(shown_id)).unwrap_or(COMMENT_PAGE);
    let groups: Vec<&[Change]> = issue.history.chunk_by(|a, b| a.id == b.id).collect();
    let total = match tab {
        ActivityTab::Comments => {
            if issue.comments.is_empty() {
                empty_state(ui, t, "", "No comments yet. Start the conversation above.", None);
            }
            for c in issue.comments.iter().rev().take(shown) {
                comment_row(app, ui, t, k, c, now, acts);
            }
            issue.comments.len()
        }
        ActivityTab::History => {
            if groups.is_empty() {
                empty_state(ui, t, "", "No changes recorded yet. History starts here.", None);
            }
            for g in groups.iter().rev().take(shown) {
                change_row(app, ui, t, &issue.key, g, now);
            }
            groups.len()
        }
        ActivityTab::All => {
            // Comments show in full, so the history's "comment" lines would repeat them.
            let mut all: Vec<Entry<'_>> = issue.comments.iter().map(Entry::Comment).collect();
            all.extend(groups.iter().filter(|g| g.iter().any(|c| c.field != "comment")).map(|g| Entry::Changes(g)));
            all.sort_by_key(|e| std::cmp::Reverse(e.at()));
            if all.is_empty() {
                empty_state(ui, t, "", "Nothing here yet.", None);
            }
            for e in all.iter().take(shown) {
                match e {
                    Entry::Comment(c) => comment_row(app, ui, t, k, c, now, acts),
                    Entry::Changes(g) => change_row(app, ui, t, &issue.key, g, now),
                }
            }
            all.len()
        }
    };
    let older = total.saturating_sub(shown);
    let what = if tab == ActivityTab::Comments { "comments" } else { "activity" };
    if older > 0 && ui.button(format!("Show older {what} ({older} more)")).clicked() {
        ui.data_mut(|d| d.insert_temp(shown_id, shown.saturating_add(COMMENT_PAGE)));
    }
}

fn comment_box(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, k: &str, acts: &mut Vec<Action>) {
    let ws = &app.session.ws;
    ui.horizontal_top(|ui| {
        let me = app.session.user.clone();
        let name = name_of(ws, Some(&me));
        icons::avatar(ui, t, &me, &name, 26.0);
        ui.vertical(|ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut app.issue_ui.comment).desired_rows(2).desired_width(f32::INFINITY).hint_text("Add a comment… (M)"),
            );
            if std::mem::take(&mut app.issue_ui.focus_comment) {
                r.request_focus();
                r.scroll_to_me(None);
            }
            if !app.issue_ui.comment.trim().is_empty() && ui.button("Save comment").clicked() {
                let body = std::mem::take(&mut app.issue_ui.comment);
                acts.push(Action::Run { id: "issue.comment", params: json!({"key": k, "body": body}), then: Then::Nothing });
            }
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn comment_row(
    app: &mut TrackApp,
    ui: &mut Ui,
    t: &Tokens,
    k: &str,
    c: &trackcraft_model::Comment,
    now: trackcraft_model::Timestamp,
    acts: &mut Vec<Action>,
) {
    let ws = &app.session.ws;
    let mine = c.author == app.session.user;
    let editing = &mut app.issue_ui.polish.editing_comment;
    ui.horizontal_top(|ui| {
        let name = name_of(ws, Some(&c.author));
        who(ui, t, ws, Some(&c.author), 26.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                // The name gets at most 60% of the row, so the time and the buttons stay visible.
                let w = ui.available_width() * 0.6;
                ui.allocate_ui(egui::vec2(w, ui.spacing().interact_size.y), |ui| {
                    ui.add(egui::Label::new(RichText::new(&name).strong()).truncate());
                });
                ui.label(RichText::new(crate::ago(now, c.created)).color(t.muted).size(12.0)).on_hover_text(exact(c.created));
                if c.updated > c.created {
                    ui.label(RichText::new("edited").color(t.muted).size(12.0).italics()).on_hover_text(format!("Edited {}", exact(c.updated)));
                }
                if mine && editing.as_ref().is_none_or(|(id, _)| *id != c.id) {
                    if ui.small_button("Edit").on_hover_text("Edit your comment").clicked() {
                        *editing = Some((c.id, c.body.to_markdown()));
                    }
                    if ui.small_button("Delete").clicked() {
                        acts.push(Action::Run {
                            id: "issue.deleteComment",
                            params: json!({"key": k, "id": c.id.0}),
                            then: Then::Toast("Comment deleted".into()),
                        });
                    }
                }
            });
            match editing {
                Some((id, text)) if *id == c.id && mine => {
                    let r = ui.add(egui::TextEdit::multiline(text).desired_rows(3).desired_width(f32::INFINITY).hint_text("Comment (Markdown)"));
                    let focus_id = r.id.with("focused");
                    let first = ui.data_mut(|d| !std::mem::replace(d.get_temp_mut_or_default::<bool>(focus_id), true));
                    if first {
                        r.request_focus();
                    }
                    let save_key = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
                    let mut save = false;
                    let mut cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    ui.horizontal(|ui| {
                        save = ui.button("Save").on_hover_text(format!("Save ({})", crate::key("Mod+Enter"))).clicked() || save_key;
                        cancel |= ui.button("Cancel").on_hover_text("Cancel (Escape)").clicked();
                    });
                    if save && !text.trim().is_empty() {
                        acts.push(Action::Run {
                            id: "issue.editComment",
                            params: json!({"key": k, "id": c.id.0, "body": text.clone()}),
                            then: Then::Toast("Comment updated".into()),
                        });
                        cancel = true;
                    }
                    if cancel {
                        ui.data_mut(|d| d.remove::<bool>(focus_id));
                        *editing = None;
                    }
                }
                _ => markup_view::show(ui, t, &c.body),
            }
        });
    });
    ui.add_space(8.0);
}

/// The exact time, for hover text.
fn exact(t: trackcraft_model::Timestamp) -> String {
    t.format("%Y-%m-%d %H:%M UTC").to_string()
}

/// A field's name as people read it.
fn field_name(f: &str) -> String {
    match f {
        "status" => "Status",
        "resolution" => "Resolution",
        "assignee" => "Assignee",
        "priority" => "Priority",
        "summary" => "Summary",
        "description" => "Description",
        "labels" => "Labels",
        "storyPoints" => "Story points",
        "sprint" => "Sprint",
        "link" => "Link",
        "comment" => "Comment",
        "issuetype" => "Type",
        "parent" => "Parent",
        "rank" => "Rank",
        other => return other.chars().take(40).collect(),
    }
    .to_string()
}

/// One command's changes: who, when, and a line per field.
fn change_row(app: &TrackApp, ui: &mut Ui, t: &Tokens, key: &IssueKey, g: &[Change], now: trackcraft_model::Timestamp) {
    let ws = &app.session.ws;
    let Some(first) = g.first() else { return };
    ui.horizontal_top(|ui| {
        who(ui, t, ws, Some(&first.author), 26.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() * 0.6;
                ui.allocate_ui(egui::vec2(w, ui.spacing().interact_size.y), |ui| {
                    ui.add(egui::Label::new(RichText::new(name_of(ws, Some(&first.author))).strong()).truncate());
                });
                ui.label(RichText::new(crate::ago(now, first.at)).color(t.muted).size(12.0)).on_hover_text(exact(first.at));
            });
            for c in g {
                ui.horizontal(|ui| {
                    if c.field == "created" {
                        ui.label(RichText::new("created the issue").color(t.muted));
                        return;
                    }
                    ui.label(RichText::new(format!("{}:", field_name(&c.field))).color(t.muted));
                    // An added link or comment reads as a sentence, not "None to ...".
                    if c.from.is_none() && matches!(c.field.as_str(), "link" | "comment") {
                        value(ui, t, ws, key, &c.field, c.to.as_deref());
                        return;
                    }
                    value(ui, t, ws, key, &c.field, c.from.as_deref());
                    arrow(ui, t.muted);
                    value(ui, t, ws, key, &c.field, c.to.as_deref());
                });
            }
        });
    });
    ui.add_space(8.0);
}

/// A painted right arrow between the old and new value (the bundled font has no arrow glyph).
fn arrow(ui: &mut Ui, color: egui::Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 12.0), egui::Sense::hover());
    let y = r.center().y;
    let (a, b) = (egui::pos2(r.left() + 2.0, y), egui::pos2(r.right() - 3.0, y));
    let s = egui::Stroke::new(1.4, color);
    let p = ui.painter();
    p.line_segment([a, b], s);
    p.line_segment([b, b + egui::vec2(-4.0, -3.5)], s);
    p.line_segment([b, b + egui::vec2(-4.0, 3.5)], s);
}

/// A history value drawn like the field it came from: status as a pill, people with avatars.
fn value(ui: &mut Ui, t: &Tokens, ws: &Workspace, key: &IssueKey, field: &str, v: Option<&str>) {
    let Some(v) = v.filter(|v| !v.is_empty()) else {
        ui.label(RichText::new("None").color(t.muted).italics());
        return;
    };
    match field {
        "status" => {
            let cat = ws
                .projects
                .get(&key.project)
                .and_then(|p| p.workflow.statuses.iter().find(|s| s.name.eq_ignore_ascii_case(v)))
                .map_or(StatusCategory::ToDo, |s| s.category);
            pill(ui, t, cat, v);
        }
        "assignee" | "reporter" => {
            who(ui, t, ws, Some(v), 16.0);
            ui.label(name_of(ws, Some(v)));
        }
        _ => {
            let shown: String = v.chars().take(60).collect();
            let more = if v.chars().count() > 60 { "..." } else { "" };
            let r = ui.label(format!("{shown}{more}"));
            if !more.is_empty() {
                r.on_hover_text(v.chars().take(500).collect::<String>());
            }
        }
    }
}

// ---- Fields column ----

/// Issues that may be `issue`'s parent: not itself, not a sub-task; for a sub-task, a standard
/// issue of the same project. (Cycles are refused by the engine, shown as a toast.)
fn parent_candidates(ws: &Workspace, issue: &Issue) -> Vec<Candidate> {
    let sub = ws.is_subtask(issue);
    ws.issues
        .values()
        .filter(|o| o.key != issue.key && !ws.is_subtask(o) && o.parent.as_ref() != Some(&issue.key))
        .filter(|o| !sub || (o.key.project == issue.key.project && !o.issue_type.eq_ignore_ascii_case("Epic")))
        .map(|o| Candidate { key: o.key.clone(), summary: o.summary.clone(), issue_type: o.issue_type.clone() })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn fields(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, issue: &Issue, k: &str, cat: StatusCategory, transitions: &[String]) {
    let (fill, fg) = t.status(cat);
    // UI-37: editors, transitions and Delete are disabled without permission.
    let can_edit = crate::widgets::can(app, "issue.edit", &json!({"key": k}));
    let can_move = crate::widgets::can(app, "issue.transition", &json!({"key": k, "to": transitions.first().cloned().unwrap_or_default()}));
    let can_delete = crate::widgets::can(app, "issue.delete", &json!({"key": k}));
    let l_status = ui.label(RichText::new("Status").color(t.muted).size(12.0)).id;
    let status = ui
        .add_enabled_ui(can_move, |ui| {
            ComboBox::from_id_salt("status")
                .selected_text(RichText::new(issue.status.to_uppercase()).strong().color(fg))
                .width(180.0)
                .show_ui(ui, |ui| {
                    if transitions.is_empty() {
                        ui.label("No transitions available");
                    }
                    for to in transitions {
                        if ui.selectable_label(false, to).clicked() {
                            app.push(Action::Run { id: "issue.transition", params: json!({"key": k, "to": to}), then: Then::Nothing });
                        }
                    }
                })
                .response
                .labelled_by(l_status)
                .on_disabled_hover_text(crate::widgets::denied("change this issue's status"))
        })
        .inner;
    // `t` (see `keys.rs`) opens the menu.
    if std::mem::take(&mut app.issue_ui.open_status) {
        egui::Popup::open_id(ui.ctx(), status.id.with("popup"));
    }
    let _ = fill;
    ui.add_space(10.0);
    let rev = app.session.revision();
    let suggestions = workspace_labels(&mut app.issue_ui.polish.labels, rev, &app.session.ws);
    card(ui, t, "Details", |ui| {
        if !can_edit {
            ui.label(RichText::new(crate::widgets::denied("edit this issue")).color(t.muted).size(12.0)); // UI-37
            ui.disable();
        }
        let ws = &app.session.ws;
        let me = app.session.user.clone();
        let mut acts = Vec::new();
        egui::Grid::new("fields").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            field_row(ui, t, "Assignee", |ui, l_assignee| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        who(ui, t, ws, issue.assignee.as_deref(), 20.0);
                        let picked = user_picker(ui, ws, &me, "assignee", issue.assignee.as_deref(), l_assignee, 150.0);
                        if let Some(choice) = picked.choice {
                            acts.push(Action::Run { id: "issue.assign", params: json!({"key": k, "assignee": choice}), then: Then::Nothing });
                        }
                        // `a` (see `keys.rs`) opens the picker.
                        if std::mem::take(&mut app.issue_ui.open_assignee) {
                            egui::Popup::open_id(ui.ctx(), picked.response.id.with("popup"));
                        }
                    });
                    if issue.assignee.as_deref() != Some(me.as_str())
                        && ui.link(RichText::new("Assign to me").size(12.0)).on_hover_text("Assign to me (I)").clicked()
                    {
                        acts.push(Action::Run { id: "issue.assignToMe", params: json!({"key": k}), then: Then::Nothing });
                    }
                });
            });

            field_row(ui, t, "Reporter", |ui, _| {
                ui.horizontal(|ui| {
                    who(ui, t, ws, Some(&issue.reporter), 20.0);
                    ui.add(egui::Label::new(name_of(ws, Some(&issue.reporter))).truncate());
                });
            });

            // UI-9: type and parent. Engine rules (an epic has no parent, a sub-task needs one)
            // come back as error toasts through `TrackApp::run`.
            field_row(ui, t, "Type", |ui, l_type| {
                ui.horizontal(|ui| {
                    icons::issue_type(ui, &issue.issue_type, 14.0);
                    let types: Vec<String> =
                        ws.projects.get(&issue.key.project).map(|p| p.issue_types.iter().map(|it| it.name.clone()).collect()).unwrap_or_default();
                    ComboBox::from_id_salt("issue-type")
                        .selected_text(&issue.issue_type)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            for ty in &types {
                                if ui.selectable_label(*ty == issue.issue_type, ty).clicked() && *ty != issue.issue_type {
                                    acts.push(Action::Run {
                                        id: "issue.edit",
                                        params: json!({"key": k, "type": ty}),
                                        then: Then::Toast(format!("Type changed to {ty}")),
                                    });
                                }
                            }
                        })
                        .response
                        .labelled_by(l_type);
                });
            });

            field_row(ui, t, "Parent", |ui, l_parent| {
                if issue.issue_type.eq_ignore_ascii_case("Epic") {
                    ui.label(RichText::new("None (an epic has no parent)").color(t.muted));
                    return;
                }
                let selected = match &issue.parent {
                    Some(p) => ws.issues.get(p).map_or_else(|| p.to_string(), |o| short(p, &o.summary, 18)),
                    None => "None".to_string(),
                };
                let picked = issue_picker(
                    ui,
                    "issue-parent",
                    selected,
                    || parent_candidates(ws, issue),
                    issue.parent.as_ref(),
                    !ws.is_subtask(issue),
                    l_parent,
                    170.0,
                );
                if let Some(choice) = picked.choice {
                    let v = choice.map(|p| p.to_string());
                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "parent": v}), then: Then::Nothing });
                }
            });

            field_row(ui, t, "Priority", |ui, l_priority| {
                ui.horizontal(|ui| {
                    icons::priority(ui, t, &issue.priority, priority_rank(&issue.priority).unwrap_or(3), 14.0);
                    ComboBox::from_id_salt("priority")
                        .selected_text(&issue.priority)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            for p in PRIORITIES {
                                if ui.selectable_label(issue.priority == p, p).clicked() {
                                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "priority": p}), then: Then::Nothing });
                                }
                            }
                        })
                        .response
                        .labelled_by(l_priority);
                });
            });

            // UI-9: chips with a remove cross; type a word and press Enter (or a comma) to add.
            field_row(ui, t, "Labels", |ui, l_labels| {
                let draft = app.issue_ui.labels.get_or_insert_with(String::new);
                if let Some(next) = label_editor(ui, t, "issue-labels", draft, &issue.labels, &suggestions, l_labels, 180.0) {
                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "labels": next}), then: Then::Nothing });
                }
            });

            // UI-9 (G6): one command per committed edit, not per typed digit.
            field_row(ui, t, "Story points", |ui, l_points| {
                if let Some(v) = number_field(ui, "issue-points", issue.story_points, l_points, 80.0) {
                    acts.push(Action::Run { id: "issue.edit", params: json!({"key": k, "storyPoints": v}), then: Then::Nothing });
                }
            });

            let now = app.session.now();
            field_row(ui, t, "Created", |ui, _| ui.label(crate::ago(now, issue.created)));
            field_row(ui, t, "Updated", |ui, _| ui.label(crate::ago(now, issue.updated)));
            if let Some(r) = issue.resolved {
                field_row(ui, t, "Resolved", |ui, _| ui.label(crate::ago(now, r)));
            }
        });
        for a in acts {
            app.push(a);
        }
    });
    ui.add_space(10.0);
    if app.issue_ui.confirm_delete {
        ui.label(RichText::new(format!("Delete {k} and its sub-tasks?")).color(t.danger));
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Delete").color(t.danger)).clicked() {
                app.issue_ui.confirm_delete = false;
                app.push(Action::Run { id: "issue.delete", params: json!({"key": k}), then: Then::Nav(View::Board(issue.key.project.clone())) });
                app.push(Action::Toast(format!("Deleted {k} ({} to undo)", crate::key("Mod+Z"))));
            }
            if ui.button("Cancel").clicked() {
                app.issue_ui.confirm_delete = false;
            }
        });
    } else if ui
        .add_enabled(can_delete, egui::Button::new("Delete issue…").small())
        .on_disabled_hover_text(crate::widgets::denied("delete this issue"))
        .clicked()
    {
        app.issue_ui.confirm_delete = true;
    }
}
