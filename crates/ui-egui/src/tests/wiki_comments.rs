//! Page comments (UI-27): footer threads and block (inline) comments in the margin.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::Id;

use super::*;

/// "Architecture overview": a child page with one paragraph.
const PAGE: u64 = 24;

fn page_harness() -> Harness<'static, TrackApp> {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "page", "id": PAGE}})).unwrap();
    h.run_steps(3);
    h
}

fn comments(h: &Harness<'static, TrackApp>) -> Vec<trackcraft_model::PageComment> {
    h.state().session.ws.page(Id(PAGE)).unwrap().comments.clone()
}

fn type_into(h: &mut Harness<'static, TrackApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::MultilineTextInput, label).click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::MultilineTextInput, label).type_text(text);
    h.run_steps(2);
}

#[test]
fn add_reply_edit_delete_footer_comments() {
    let mut h = page_harness();
    // Add.
    type_into(&mut h, "Add a comment", "First thoughts");
    h.get_by_label("Add comment").click();
    h.run_steps(3);
    let c = comments(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].body.to_markdown(), "First thoughts");
    assert!(h.state().wiki.comments.draft.is_empty());
    assert!(h.query_by_label_contains("1 comment").is_some());
    // Reply.
    h.get_by_label("Reply to comment").click();
    h.run_steps(3);
    type_into(&mut h, "Reply", "A reply");
    h.get_by_label("Send reply").click();
    h.run_steps(3);
    let c = comments(&h);
    assert_eq!(c.len(), 2);
    assert_eq!(c[1].parent, Some(c[0].id));
    assert!(h.query_by_label("A reply").is_some());
    // Edit the reply (the second "Edit comment").
    h.get_all_by_label("Edit comment").nth(1).unwrap().click();
    h.run_steps(3);
    h.state_mut().wiki.comments.edit.as_mut().unwrap().1 = "An edited reply".into();
    h.run_steps(1);
    h.get_by_label("Save").click();
    h.run_steps(3);
    let c = comments(&h);
    assert_eq!(c[1].body.to_markdown(), "An edited reply");
    assert!(c[1].edited.is_some());
    // Delete the thread: its reply goes with it.
    h.get_all_by_label("Delete comment").next().unwrap().click();
    h.run_steps(3);
    assert!(comments(&h).is_empty());
    assert!(h.query_by_label("An edited reply").is_none());
}

#[test]
fn others_comments_cannot_be_edited() {
    let mut h = page_harness();
    h.state_mut().session.user = "jo".into();
    h.state_mut().session.run("page.comment", &json!({"id": PAGE, "body": "From Jo"})).unwrap();
    h.state_mut().session.user = "riley".into();
    h.run_steps(3);
    assert!(h.query_by_label("From Jo").is_some());
    assert!(h.query_by_label("Edit comment").is_none());
    assert!(h.query_by_label("Delete comment").is_none());
}

#[test]
fn block_comment_shows_in_margin_and_resolve_hides_it() {
    let mut h = page_harness();
    // Hover the paragraph: the gutter button appears.
    assert!(h.query_by_label("Comment on this block").is_none());
    h.get_all_by_label_contains("The app is a thin client").next().unwrap().hover();
    h.run_steps(2);
    h.get_by_label("Comment on this block").click();
    h.run_steps(3);
    assert_eq!(h.state().wiki.comments.block.as_ref().map(|b| b.0), Some(0));
    type_into(&mut h, "Comment on this block", "Which store?");
    h.get_by_label("Comment").click();
    h.run_steps(3);
    let c = comments(&h);
    assert_eq!(c.len(), 1);
    let a = c[0].anchor.as_ref().unwrap();
    assert_eq!((a.block, a.start), (0, 0));
    assert!(a.quote.starts_with("The app is a thin client"));
    assert_eq!(a.end as usize, a.quote.chars().count());
    // The thread sits in the margin, to the right of the body, level with its block.
    let card = h.get_by_label("Which store?").rect();
    let block = h.get_all_by_label_contains("The app is a thin client").next().unwrap().rect();
    assert!(card.left() > block.right());
    assert!((card.top() - block.top()).abs() < 120.0);
    // Resolve hides it; "Show resolved" brings it back with Reopen.
    h.get_by_label("Resolve thread").click();
    h.run_steps(3);
    assert!(comments(&h)[0].resolved);
    assert!(h.query_by_label("Which store?").is_none());
    h.get_by_label("Show resolved (1)").click();
    h.run_steps(3);
    assert!(h.query_by_label("Which store?").is_some());
    h.get_by_label("Reopen thread").click();
    h.run_steps(3);
    assert!(!comments(&h)[0].resolved);
}

#[test]
fn outdated_inline_comment_is_grouped() {
    let mut h = page_harness();
    let s = &mut h.state_mut().session;
    s.run("page.comment", &json!({"id": PAGE, "body": "Old point", "anchor": {"block": 0, "start": 0, "end": 7, "quote": "The app"}})).unwrap();
    s.run("page.edit", &json!({"id": PAGE, "body": "Rewritten from scratch."})).unwrap();
    h.run_steps(3);
    // Collapsed by default.
    assert!(h.query_by_label("Old point").is_none());
    h.get_by_label("Outdated (1)").click();
    h.run_steps(3);
    assert!(h.query_by_label("Old point").is_some());
}
