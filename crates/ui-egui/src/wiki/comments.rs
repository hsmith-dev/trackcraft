//! Page comments (UI-27): footer threads with one reply level under the page, and block
//! comments in a margin beside the body, aligned to the block they quote.
//!
//! Selecting text inside egui labels is not reliable, so an inline comment anchors a whole
//! block: the hover button beside a block opens a small form, and the anchor quotes the
//! block's first [`QUOTE_CHARS`] characters (`start` 0, `end` the quote's length).

use egui::{Color32, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use trackcraft_core::Id;
use trackcraft_markup::{Block, Doc};
use trackcraft_model::{Page, PageComment, locate_anchor};

use crate::theme::Tokens;
use crate::{TrackApp, markup_view};

/// Width of the inline-comment margin beside the body.
pub(crate) const MARGIN_W: f32 = 270.0;
/// Gap between the body and the margin; the block comment button sits in it.
pub(crate) const GUTTER: f32 = 36.0;
/// How much of a block an inline comment quotes.
pub(crate) const QUOTE_CHARS: usize = 80;

/// Typed text and toggles of the comment areas on the open page.
#[derive(Debug, Default)]
pub(crate) struct CommentsUi {
    /// The new footer comment.
    pub draft: String,
    /// The open reply box: the thread it answers and its text.
    pub reply: Option<(Id, String)>,
    /// The comment being edited and its text.
    pub edit: Option<(Id, String)>,
    /// The open block comment form: block index and text.
    pub block: Option<(usize, String)>,
    /// Show resolved inline threads in the margin.
    pub show_resolved: bool,
}

/// A comment command and what to clear when it succeeds.
enum Op {
    Add { params: serde_json::Value, clear: Clear },
    Edit { comment: Id, body: String },
    Delete(Id),
    Resolve(Id, bool),
}

#[derive(Clone, Copy)]
enum Clear {
    Draft,
    Reply,
    Block,
}

/// The quote an inline comment on `b` carries: its start, up to [`QUOTE_CHARS`] characters,
/// ending at a word boundary (UI-fixes).
pub(crate) fn quote_of(b: &Block) -> String {
    crate::widgets::word_prefix(&b.plain_text(), QUOTE_CHARS).to_string()
}

/// Where an inline thread sits now: its current block, or `None` when the quoted text is gone.
fn located(p: &Page, c: &PageComment) -> Option<usize> {
    let a = c.anchor.as_ref()?;
    locate_anchor(&p.body, a).and_then(|(b, _, _)| usize::try_from(b).ok())
}

/// Open (unresolved, still anchored) inline threads by block: shown as a bar beside the block.
pub(crate) fn open_blocks(p: &Page) -> std::collections::BTreeSet<usize> {
    p.comments.iter().filter(|c| c.parent.is_none() && c.anchor.is_some() && !c.resolved).filter_map(|c| located(p, c)).collect()
}

/// Every comment on the page, replies included (the header badge).
pub(crate) fn count(p: &Page) -> usize {
    p.comments.len()
}

/// Runs `op`; returns whether it worked (errors become a toast).
fn exec(app: &mut TrackApp, page: Id, op: Op) {
    let (id, params, toast, clear) = match op {
        Op::Add { params, clear } => ("page.comment", params, None, Some(clear)),
        Op::Edit { comment, body } => ("page.editComment", json!({"id": page.0, "comment": comment.0, "body": body}), None, None),
        Op::Delete(c) => ("page.deleteComment", json!({"id": page.0, "comment": c.0}), Some("Comment deleted"), None),
        Op::Resolve(c, on) => (
            "page.resolveComment",
            json!({"id": page.0, "comment": c.0, "resolved": on}),
            Some(if on { "Comment resolved" } else { "Comment reopened" }),
            None,
        ),
    };
    let editing = id == "page.editComment";
    match app.session.run(id, &params) {
        Ok(_) => {
            app.jql_ran = None;
            if let Some(m) = toast {
                app.notify_done(m, true);
            }
            let c = &mut app.wiki.comments;
            match clear {
                Some(Clear::Draft) => c.draft.clear(),
                Some(Clear::Reply) => c.reply = None,
                Some(Clear::Block) => c.block = None,
                None => {}
            }
            if editing {
                c.edit = None;
            }
        }
        Err(e) => app.notify_error(e),
    }
}

/// A multi-line text box with an accessible name.
fn text_box(ui: &mut Ui, text: &mut String, label: &str, hint: &str, rows: usize) -> egui::Response {
    let l = ui.label(RichText::new(label).size(12.0));
    ui.add(egui::TextEdit::multiline(text).hint_text(hint).desired_rows(rows).desired_width(f32::INFINITY)).labelled_by(l.id)
}

/// A small button whose accessible name says what it acts on.
fn small(ui: &mut Ui, text: impl Into<egui::WidgetText>, name: &str) -> bool {
    let r = ui.small_button(text);
    let name = name.to_string();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
    r.clicked()
}

fn primary(ui: &mut Ui, t: &Tokens, text: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text).color(t.on_accent).strong()).fill(t.accent)).clicked()
}

/// One comment: avatar, name, time, body (or its edit box) and its actions.
fn comment(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, c: &PageComment, reply_to: Option<Id>, ops: &mut Vec<Op>) {
    let now = app.session.now();
    let mine = c.author == app.session.user;
    ui.horizontal(|ui| {
        crate::widgets::who(ui, t, &app.session.ws, Some(&c.author), 20.0);
        let name = crate::widgets::name_of(&app.session.ws, Some(&c.author));
        ui.label(RichText::new(name).strong().size(13.0));
        let mut when = crate::ago(now, c.created);
        if c.edited.is_some() {
            when.push_str(" · edited");
        }
        ui.label(RichText::new(when).color(t.muted).size(12.0));
    });
    let editing = app.wiki.comments.edit.as_ref().is_some_and(|e| e.0 == c.id);
    if editing {
        let mut save = None;
        let mut cancel = false;
        if let Some((_, text)) = app.wiki.comments.edit.as_mut() {
            text_box(ui, text, "Edit comment", "", 3);
            ui.horizontal(|ui| {
                if primary(ui, t, "Save", !text.trim().is_empty()) {
                    save = Some(text.clone());
                }
                cancel = ui.button("Cancel").clicked();
            });
        }
        if let Some(body) = save {
            ops.push(Op::Edit { comment: c.id, body });
        }
        if cancel {
            app.wiki.comments.edit = None;
        }
    } else {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            markup_view::show(ui, t, &c.body);
        });
    }
    if editing {
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if let Some(thread) = reply_to
            && small(ui, "Reply", "Reply to comment")
        {
            app.wiki.comments.reply = Some((thread, String::new()));
        }
        if mine {
            if small(ui, "Edit", "Edit comment") {
                app.wiki.comments.edit = Some((c.id, c.body.to_markdown()));
            }
            if small(ui, RichText::new("Delete").color(t.danger), "Delete comment") {
                ops.push(Op::Delete(c.id));
            }
        }
    });
}

/// A thread: the top comment, its replies indented, and the reply box when open.
fn thread(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, p: &Page, top: &PageComment, ops: &mut Vec<Op>) {
    comment(app, ui, t, top, Some(top.id), ops);
    let replies: Vec<&PageComment> = p.comments.iter().filter(|r| r.parent == Some(top.id)).collect();
    if replies.is_empty() && !app.wiki.comments.reply.as_ref().is_some_and(|r| r.0 == top.id) {
        return;
    }
    ui.horizontal_top(|ui| {
        ui.add_space(18.0);
        let r = ui.vertical(|ui| {
            for r in replies {
                comment(app, ui, t, r, None, ops);
                ui.add_space(4.0);
            }
            let mut send = None;
            let mut cancel = false;
            if let Some((thread, text)) = app.wiki.comments.reply.as_mut()
                && *thread == top.id
            {
                let r = text_box(ui, text, "Reply", "Write a reply", 2);
                if text.is_empty() && !ui.memory(|m| m.focused().is_some()) {
                    r.request_focus();
                }
                ui.horizontal(|ui| {
                    if primary(ui, t, "Send reply", !text.trim().is_empty()) {
                        send = Some(text.clone());
                    }
                    cancel = ui.button("Cancel").clicked();
                });
            }
            if let Some(body) = send {
                ops.push(Op::Add { params: json!({"id": p.id.0, "body": body, "parent": top.id.0}), clear: Clear::Reply });
            }
            if cancel {
                app.wiki.comments.reply = None;
            }
        });
        // A thin line ties the replies to their thread.
        let rr = r.response.rect;
        ui.painter().vline(rr.left() - 10.0, rr.y_range(), Stroke::new(2.0, t.border));
    });
}

/// The footer comments under the page body.
pub(crate) fn footer(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, p: &Page) {
    let mut ops = Vec::new();
    let tops: Vec<&PageComment> = p.comments.iter().filter(|c| c.parent.is_none() && c.anchor.is_none()).collect();
    ui.add_space(24.0);
    ui.separator();
    ui.add_space(6.0);
    let n = p.comments.iter().filter(|c| c.anchor.is_none() && c.parent.is_none_or(|t| tops.iter().any(|x| x.id == t))).count();
    ui.label(RichText::new(if n == 0 { "Comments".to_string() } else { format!("Comments ({n})") }).strong().size(16.0));
    ui.add_space(6.0);
    for c in &tops {
        egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            thread(app, ui, t, p, c, &mut ops);
        });
        ui.add_space(6.0);
    }
    if tops.is_empty() {
        ui.label(RichText::new("No comments yet. Start the conversation below.").color(t.muted).size(12.5));
        ui.add_space(4.0);
    }
    let draft = &mut app.wiki.comments.draft;
    text_box(ui, draft, "Add a comment", "Write a comment (Markdown)", 3);
    let ready = !draft.trim().is_empty();
    let body = draft.clone();
    if primary(ui, t, "Add comment", ready) {
        ops.push(Op::Add { params: json!({"id": p.id.0, "body": body}), clear: Clear::Draft });
    }
    for op in ops {
        exec(app, p.id, op);
    }
}

/// The comment-bubble button in the gutter beside block `n` (shown on hover or while its form
/// is open). Returns whether it was clicked.
pub(crate) fn block_button(ui: &mut Ui, t: &Tokens, block: Rect, x: f32, n: usize, active: bool) -> bool {
    let rect = Rect::from_min_size(pos2(x, block.top()), vec2(24.0, 22.0));
    let r = ui.interact(rect, egui::Id::unique(("block-comment", n)), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Comment on this block"));
    let r = r.on_hover_text("Comment on this block");
    let p = ui.painter();
    if r.hovered() || active {
        p.rect_filled(rect, 5.0, t.accent_soft);
    }
    let c = if r.hovered() || active { t.accent } else { t.muted };
    // A speech bubble: a rounded body and a small tail.
    let body = Rect::from_center_size(rect.center() - vec2(0.0, 1.5), vec2(14.0, 10.0));
    p.rect_stroke(body, 3.0, Stroke::new(1.4, c), egui::StrokeKind::Middle);
    let tail = [pos2(body.left() + 3.0, body.bottom()), pos2(body.left() + 3.0, body.bottom() + 4.0), pos2(body.left() + 7.0, body.bottom())];
    p.add(egui::Shape::line(tail.to_vec(), Stroke::new(1.4, c)));
    r.clicked()
}

/// A bar beside a block that has open inline comments.
pub(crate) fn mark_block(ui: &Ui, t: &Tokens, block: Rect) {
    let bar = Rect::from_min_size(pos2(block.left() - 10.0, block.top()), vec2(3.0, block.height().max(12.0)));
    ui.painter().rect_filled(bar, 1.5, t.accent);
}

/// A quote in a card: cut at a word boundary with "…" when it stops before `source` (the
/// quoted block's text, when it is still there) or is long (UI-fixes).
fn quote(ui: &mut Ui, t: &Tokens, text: &str, source: Option<&str>, struck: bool) {
    let text = crate::widgets::quote_excerpt(text, source, QUOTE_CHARS);
    let mut rt = RichText::new(format!("“{text}”")).italics().color(t.muted).size(12.0);
    if struck {
        rt = rt.strikethrough();
    }
    let r = ui.add(egui::Label::new(rt).wrap());
    let rr = r.rect;
    ui.painter().vline(rr.left() - 6.0, rr.y_range(), Stroke::new(2.0, t.accent_soft));
}

/// One inline thread card in the margin.
fn inline_card(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, p: &Page, c: &PageComment, outdated: bool, ops: &mut Vec<Op>) {
    let fill = if c.resolved || outdated { t.surface_alt } else { t.surface };
    let edge = if c.resolved || outdated { t.border } else { t.accent };
    egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, edge)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(a) = &c.anchor {
            let source = if outdated { None } else { located(p, c).and_then(|b| p.body.blocks.get(b)).map(Block::plain_text) };
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.vertical(|ui| quote(ui, t, &a.quote, source.as_deref(), outdated));
            });
        }
        if c.resolved {
            let by = crate::widgets::name_of(&app.session.ws, c.resolved_by.as_deref());
            ui.label(RichText::new(format!("Resolved by {by}")).color(t.ok).size(11.5));
        }
        thread(app, ui, t, p, c, ops);
        if small(ui, if c.resolved { "Reopen" } else { "Resolve" }, if c.resolved { "Reopen thread" } else { "Resolve thread" }) {
            ops.push(Op::Resolve(c.id, !c.resolved));
        }
    });
    ui.add_space(6.0);
}

/// Pushes the margin's cursor down to `y` (no-op when it is already past it).
fn align_to(ui: &mut Ui, y: f32) {
    let gap = y - ui.cursor().top();
    if gap > 0.0 {
        ui.add_space(gap);
    }
}

/// The inline-comment margin: open threads aligned to their blocks (`tops` holds each block's
/// top edge; empty when the body is drawn lazily), the block form, resolved threads behind a
/// toggle and outdated ones in a collapsed group.
pub(crate) fn margin(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, p: &Page, tops: &[f32]) {
    let mut ops = Vec::new();
    let inline: Vec<(&PageComment, Option<usize>)> =
        p.comments.iter().filter(|c| c.parent.is_none() && c.anchor.is_some()).map(|c| (c, located(p, c))).collect();
    let resolved = inline.iter().filter(|(c, at)| c.resolved && at.is_some()).count();
    let outdated: Vec<&PageComment> = inline.iter().filter(|(_, at)| at.is_none()).map(|(c, _)| *c).collect();
    let show_resolved = app.wiki.comments.show_resolved;
    // What is placed beside blocks: threads (and the open form), by block then age.
    enum Item<'a> {
        Thread(&'a PageComment),
        Form(usize),
    }
    let mut items: Vec<(usize, Item)> = inline
        .iter()
        .filter(|(c, at)| at.is_some() && (show_resolved || !c.resolved))
        .filter_map(|(c, at)| at.map(|b| (b, Item::Thread(c))))
        .collect();
    if let Some((b, _)) = &app.wiki.comments.block {
        items.push((*b, Item::Form(*b)));
    }
    items.sort_by_key(|(b, i)| (*b, matches!(i, Item::Form(_))));
    if !inline.is_empty() || app.wiki.comments.block.is_some() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Inline comments").color(t.muted).size(12.0).strong());
            if resolved > 0 {
                let mut on = show_resolved;
                if ui.checkbox(&mut on, RichText::new(format!("Show resolved ({resolved})")).size(12.0)).changed() {
                    app.wiki.comments.show_resolved = on;
                }
            }
        });
        ui.add_space(4.0);
    }
    for (b, item) in items {
        if let Some(y) = tops.get(b) {
            align_to(ui, *y);
        }
        match item {
            Item::Thread(c) => inline_card(app, ui, t, p, c, false, &mut ops),
            Item::Form(b) => block_form(app, ui, t, p, b, &mut ops),
        }
    }
    if !outdated.is_empty() {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new(format!("Outdated ({})", outdated.len())).color(t.muted).size(12.5))
            .id_salt(("outdated-comments", p.id.0))
            .default_open(false)
            .show(ui, |ui| {
                ui.label(RichText::new("The text these comments quoted has changed.").color(t.muted).size(11.5));
                for c in outdated {
                    inline_card(app, ui, t, p, c, true, &mut ops);
                }
            });
    }
    for op in ops {
        exec(app, p.id, op);
    }
}

/// The small form that comments on block `b`.
fn block_form(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, p: &Page, b: usize, ops: &mut Vec<Op>) {
    let q = p.body.blocks.get(b).map(quote_of).unwrap_or_default();
    let source = p.body.blocks.get(b).map(Block::plain_text);
    let mut send = None;
    let mut cancel = false;
    egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.accent)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.vertical(|ui| quote(ui, t, &q, source.as_deref(), false));
            });
            if let Some((_, text)) = app.wiki.comments.block.as_mut() {
                let r = text_box(ui, text, "Comment on this block", "Write a comment", 2);
                if text.is_empty() && !ui.memory(|m| m.focused().is_some()) {
                    r.request_focus();
                }
                ui.horizontal(|ui| {
                    if primary(ui, t, "Comment", !text.trim().is_empty() && !q.is_empty()) {
                        send = Some(text.clone());
                    }
                    cancel = ui.button("Cancel").clicked();
                });
            }
        },
    );
    ui.add_space(6.0);
    if let Some(body) = send {
        let end = q.chars().count();
        let anchor = json!({"block": b, "start": 0, "end": end, "quote": q});
        ops.push(Op::Add { params: json!({"id": p.id.0, "body": body, "anchor": anchor}), clear: Clear::Block });
    }
    if cancel {
        app.wiki.comments.block = None;
    }
}

/// Draws the page body block by block, recording each block's top edge (for the margin) and
/// offering the block comment button on hover. `gutter_x` is where the button goes.
pub(crate) fn body_blocks(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, p: &Page, gutter_x: f32) -> Vec<f32> {
    let marked = open_blocks(p);
    let open_form = app.wiki.comments.block.as_ref().map(|b| b.0);
    let mut tops = Vec::with_capacity(p.body.blocks.len());
    let mut clicked = None;
    for (n, b) in p.body.blocks.iter().enumerate() {
        let part = Doc { blocks: vec![b.clone()] };
        let r = ui.scope(|ui| markup_view::show(ui, t, &part)).response;
        let rect = r.rect;
        tops.push(rect.top());
        if marked.contains(&n) {
            mark_block(ui, t, rect);
        }
        let has_text = !b.plain_text().trim().is_empty();
        let zone = Rect::from_min_max(rect.min, pos2(gutter_x + 26.0, rect.max.y.max(rect.min.y + 22.0)));
        if has_text && (ui.rect_contains_pointer(zone) || open_form == Some(n)) && block_button(ui, t, rect, gutter_x, n, open_form == Some(n)) {
            clicked = Some(n);
        }
    }
    if let Some(n) = clicked {
        app.wiki.comments.block = if open_form == Some(n) { None } else { Some((n, String::new())) };
    }
    tops
}

/// Soft background for a comment-count badge.
pub(crate) fn badge(ui: &mut Ui, t: &Tokens, n: usize) -> egui::Response {
    let text = if n == 1 { "1 comment".to_string() } else { format!("{n} comments") };
    let fill: Color32 = if n == 0 { t.surface_alt } else { t.accent_soft };
    let color = if n == 0 { t.muted } else { t.accent };
    ui.add(egui::Button::new(RichText::new(text).size(11.5).color(color)).fill(fill).corner_radius(10).small())
}
