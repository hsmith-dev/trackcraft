//! UI-fixes round: peek pane, File › Import…, cross-space page moves, rich diff, quote trimming.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::Id;

use super::*;
use crate::View;

fn on_list(h: &mut Harness<'static, TrackApp>) {
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
}

#[test]
fn space_peeks_at_the_selected_row_and_follows_the_arrows() {
    let mut h = harness(app());
    on_list(&mut h);
    h.key_press(egui::Key::J);
    h.run_steps(2);
    h.key_press(egui::Key::Space);
    h.run_steps(3);
    assert_eq!(h.state().peek, Some(key("LUME-1")));
    let summary = h.state().session.ws.issues.get(&key("LUME-1")).unwrap().summary.clone();
    assert!(h.get_all_by_label(&summary).count() >= 2, "the pane repeats the summary");
    assert!(h.query_by_label("Open").is_some() && h.query_by_label("Close").is_some());
    // Arrow keys move the selection; the pane follows.
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert_eq!(h.state().peek, Some(key("LUME-2")));
    // Escape closes; Space again opens; Space on the same issue closes.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().peek, None);
    assert_eq!(h.state().view, View::Issues);
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert_eq!(h.state().peek, Some(key("LUME-2")));
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert_eq!(h.state().peek, None);
}

#[test]
fn peek_open_button_opens_the_issue_and_the_pane_closes() {
    let mut h = harness(app());
    on_list(&mut h);
    h.state_mut().control(&json!({"method": "ui.peek", "params": {"key": "LUME-2"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().list_sel, Some(key("LUME-2")), "peeking selects the row");
    // The latest comments are listed.
    assert!(h.query_by_label_contains("comments").is_some());
    h.get_by_label("Open").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issue(key("LUME-2")));
    assert_eq!(h.state().peek, None, "the issue view can't peek");
}

#[test]
fn peek_on_board_and_backlog_and_bad_requests() {
    let mut h = harness(app());
    // Not on a view that can peek.
    assert!(h.state_mut().control(&json!({"method": "ui.peek", "params": {"key": "LUME-2"}})).is_err());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "board", "project": "LUME"}})).unwrap();
    h.run_steps(3);
    h.state_mut().control(&json!({"method": "ui.peek", "params": {"key": "LUME-3"}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().board_sel, Some(key("LUME-3")));
    for bad in [json!({"key": "nope"}), json!({"key": "LUME-9999"}), json!({"key": 5})] {
        assert!(h.state_mut().control(&json!({"method": "ui.peek", "params": bad})).is_err(), "{bad}");
    }
    h.state_mut().control(&json!({"method": "ui.peek", "params": {"key": null}})).unwrap();
    assert_eq!(h.state().peek, None);
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "backlog", "project": "LUME"}})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::J);
    h.run_steps(2);
    h.key_press(egui::Key::Space);
    h.run_steps(3);
    assert!(h.state().peek.is_some());
    assert_eq!(h.state().peek, h.state().backlog.sel);
    // Deleting the issue closes the pane.
    let k = h.state().peek.clone().unwrap();
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": "issue.delete", "params": {"key": k.to_string()}}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().peek, None);
}

#[test]
fn file_import_opens_the_chooser_and_asks_the_host() {
    let mut h = harness(app());
    h.run_steps(2);
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Import\u{2026}").click();
    h.run_steps(4);
    assert!(h.state().import.choose);
    h.get_by_label("Confluence space export (folder, .zip or one page)").click();
    h.run_steps(2);
    h.get_by_label("An unpacked folder").click();
    h.run_steps(2);
    h.get_by_label("or a new space key").click();
    h.run_steps(2);
    h.event(egui::Event::Text("docs".into()));
    h.run_steps(2);
    h.get_by_label("Choose folder\u{2026}").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().take_file_request(), Some(crate::FileRequest::Import(crate::ImportKind::Confluence { folder: true })));
    assert!(!h.state().import.choose);
    let p = crate::import_dialog::params(h.state(), crate::ImportKind::Confluence { folder: true }, "/x");
    assert_eq!(p, json!({"kind": "confluence", "path": "/x", "space": "DOCS"}));
    // The space only goes with Confluence.
    let p = crate::import_dialog::params(h.state(), crate::ImportKind::JiraCsv, "/x.csv");
    assert_eq!(p, json!({"kind": "jiraCsv", "path": "/x.csv"}));
}

#[test]
fn import_shows_the_report_and_is_one_undo_step() {
    let dir = std::env::temp_dir().join(format!("tc-ui-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("export.csv");
    std::fs::write(&csv, "Summary,Issue key,Issue Type,Status,Priority\nFirst imported,IMP-1,Task,To Do,High\nSecond,IMP-2,Bug,Done,Low\n").unwrap();
    let mut h = harness(app());
    let before = h.state().session.ws.issues.len();
    h.state_mut().import_file(crate::ImportKind::JiraCsv, &csv);
    h.run_steps(4);
    assert_eq!(h.state().session.ws.issues.len(), before + 2);
    let rep = h.state().import.report.clone().unwrap();
    assert!(rep.counts.contains(&("Issues".to_string(), 2)), "{rep:?}");
    assert!(h.query_by_label("Import finished").is_some());
    h.get_by_label("Undo import").click();
    h.run_steps(4);
    assert_eq!(h.state().session.ws.issues.len(), before);
    assert!(h.state().import.report.is_none());
    // A bad file is an error toast, not a report.
    h.state_mut().import_file(crate::ImportKind::JiraJson, &dir.join("missing.json"));
    h.run_steps(2);
    assert!(h.state().import.report.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn move_dialog_offers_other_spaces_and_moves_the_subtree() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.pageDialog", "params": {"kind": "move", "id": 24}})).unwrap();
    h.run_steps(4);
    h.get_all_by_label("Team Home (space home)").last().unwrap().click();
    h.run_steps(2);
    h.get_by_label("Move").click();
    h.run_steps(3);
    let ws = &h.state().session.ws;
    for id in [24, 25, 26] {
        assert_eq!(ws.page(Id(id)).unwrap().space.as_str(), "TEAM");
    }
    assert!(h.state().wiki.dialog.is_none());
    assert!(h.query_by_label_contains("to Team Home").is_some(), "the toast names the space");
}

#[test]
fn tree_drag_onto_another_space_moves_the_page() {
    let mut h = harness(app());
    h.run_steps(3);
    let rows = |h: &Harness<'static, TrackApp>, t: &str| h.get_all_by_label(t).map(|n| n.rect()).find(|r| r.left() < 360.0).unwrap();
    let from = rows(&h, "Release checklist");
    let onto = rows(&h, "Team Home");
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.event(egui::Event::PointerMoved(from.center()));
    h.run_steps(1);
    h.event(button(from.center(), true));
    h.run_steps(1);
    for n in 1..=6 {
        h.event(egui::Event::PointerMoved(from.center() + (onto.center() - from.center()) * (n as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(button(onto.center(), false));
    h.run_steps(3);
    let ws = &h.state().session.ws;
    let p = ws.page(Id(27)).unwrap();
    assert_eq!(p.space.as_str(), "TEAM");
    assert_eq!(p.parent, Some(ws.space("TEAM").unwrap().homepage));
}

#[test]
fn diff_renders_unchanged_blocks_as_rich_text() {
    let mut h = harness(app());
    let v2 = "Intro.\n\n## Storage\n\nOne file.\n\n- alpha\n- beta";
    let v3 = "Intro.\n\n## Storage\n\nOne file, written atomically.\n\n- alpha\n- gamma";
    for body in [v2, v3] {
        h.state_mut().session.run("page.edit", &json!({"id": 24, "body": body})).unwrap();
    }
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "diff", "id": 24, "from": 2, "to": 3}})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Storage").is_some(), "the heading renders as a heading");
    assert!(h.query_by_label("## Storage").is_none(), "no raw Markdown");
    assert!(h.query_by_label_contains("- alpha").is_none(), "list markers become bullets");
}

#[test]
fn diff_shapes_changed_headings_and_bullets() {
    use crate::wiki::diff::{Shape, shaped_texts};
    assert_eq!(Shape::of("## Storage"), Shape::Heading(2));
    assert_eq!(Shape::of("- a\n- b"), Shape::Bullet);
    assert_eq!(Shape::of("#hashtag"), Shape::Plain);
    assert_eq!(Shape::of("####### seven"), Shape::Plain);
    let hunks = vec![json!({"kind": "equal", "text": "## "}), json!({"kind": "insert", "text": "Storage"})];
    assert_eq!(shaped_texts(&hunks, Shape::Heading(2)), ["", "Storage"]);
    let hunks =
        vec![json!({"kind": "equal", "text": "- alpha\n- "}), json!({"kind": "delete", "text": "beta"}), json!({"kind": "insert", "text": "gamma"})];
    assert_eq!(shaped_texts(&hunks, Shape::Bullet), ["\u{2022} alpha\n\u{2022} ", "beta", "gamma"]);
}

#[test]
fn inline_quotes_end_at_a_word_with_an_ellipsis() {
    let mut h = harness(app());
    let body = "Notes live in a single file per workspace, written atomically so a crash never leaves half a note behind.";
    h.state_mut().session.run("page.edit", &json!({"id": 24, "body": body})).unwrap();
    // An older client cut the quote mid-word ("crash never l").
    let quote: String = body.chars().take(80).collect();
    h.state_mut()
        .session
        .run("page.comment", &json!({"id": 24, "body": "Mid-word", "anchor": {"block": 0, "start": 0, "end": 80, "quote": quote}}))
        .unwrap();
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "page", "id": 24}})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("\u{201c}Notes live in a single file per workspace, written atomically so a crash never\u{2026}\u{201d}").is_some());
    // New quotes are taken at a word boundary.
    let b = &h.state().session.ws.page(Id(24)).unwrap().body.blocks[0];
    let q = crate::wiki::comments::quote_of(b);
    assert!(q.ends_with("crash never") && body.starts_with(&q), "{q}");
}
