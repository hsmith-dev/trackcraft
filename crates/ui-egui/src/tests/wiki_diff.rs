//! Version diff (UI-28) and templates (UI-29).

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::Id;

use super::*;
use crate::View;

const PAGE: u64 = 24;

#[test]
fn diff_shows_an_insert_after_editing() {
    let mut h = harness(app());
    let body = "The app is a thin client over a local note store that syncs in the background.\n\nA brand new paragraph.";
    h.state_mut().session.run("page.edit", &json!({"id": PAGE, "body": body})).unwrap();
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "history", "id": PAGE}})).unwrap();
    h.run_steps(3);
    h.get_by_label("Compare with previous").click();
    h.run_steps(4);
    assert_eq!(h.state().view, View::PageDiff(Id(PAGE), 1, 2));
    // Marked by text, not colour alone.
    assert!(h.query_by_label("ADDED").is_some());
    assert!(h.query_by_label("A brand new paragraph.").is_some());
    assert!(h.query_by_label("1 added").is_some());
    assert!(h.query_by_label("REMOVED").is_none());
}

#[test]
fn diff_from_ticked_versions_and_navigate() {
    let mut h = harness(app());
    for body in ["Two words.", "Two more words."] {
        h.state_mut().session.run("page.edit", &json!({"id": PAGE, "body": body})).unwrap();
    }
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "history", "id": PAGE}})).unwrap();
    h.run_steps(3);
    // Rows are newest first: tick v3 and v1.
    let ticks: Vec<_> = h.get_all_by_label("Compare").collect();
    assert_eq!(ticks.len(), 3);
    ticks[0].click();
    ticks[2].click();
    h.run_steps(3);
    h.get_by_label("Compare selected").click();
    h.run_steps(4);
    assert_eq!(h.state().view, View::PageDiff(Id(PAGE), 1, 3));
    // ui.navigate with defaults: the current version against the one before.
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "diff", "id": PAGE}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().view, View::PageDiff(Id(PAGE), 2, 3));
    assert!(h.query_by_label("CHANGED").is_some());
    assert!(h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "diff", "id": PAGE, "from": 9}})).is_err());
}

#[test]
fn new_page_from_builtin_template_fills_body_and_date() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.newPage", "params": {"space": "ENG"}})).unwrap();
    // The modal settles (centres on its final height) over a few frames.
    h.run_steps(8);
    h.get_by_label("Decision record").click();
    h.run_steps(2);
    assert_eq!(h.state().wiki.template, Some(json!("decision-record")));
    h.get_by_label("Create").click();
    h.run_steps(4);
    let today = fixed_now().format("%Y-%m-%d").to_string();
    let ws = &h.state().session.ws;
    let View::Page(id) = h.state().view else { panic!("not on a page: {:?} {:?}", h.state().view, h.state().new_page) };
    let p = ws.page(id).unwrap();
    let md = p.body.to_markdown();
    assert!(md.contains("## Options weighed"), "{md}");
    assert!(md.contains(&today), "{md}");
    assert!(p.title.starts_with("Decision"));
}

#[test]
fn save_page_as_template_and_delete_it() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "page", "id": PAGE}})).unwrap();
    h.run_steps(3);
    h.get_by_label("More page actions").click();
    h.run_steps(2);
    h.get_by_label("Save as template\u{2026}").click();
    h.run_steps(3);
    h.get_by_label("Save template").click();
    h.run_steps(3);
    let saved: Vec<_> = h.state().session.ws.templates.values().cloned().collect();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Architecture overview");
    assert!(saved[0].body.to_markdown().contains("thin client"));
    // The space's template list offers Delete for it (not for built-ins).
    h.state_mut().control(&json!({"method": "ui.pageDialog", "params": {"kind": "templates", "id": PAGE}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.get_all_by_label_contains("Delete template").count(), 1);
    h.get_by_label("Delete template Architecture overview").click();
    h.run_steps(3);
    assert!(h.state().session.ws.templates.is_empty());
}
