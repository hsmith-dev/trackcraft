//! Hostile data (UI-6): the stress and empty fixtures (`crate::fixtures`) in every view.
//!
//! The stress fixture keeps to scripts the bundled font covers (Latin, Greek, Cyrillic); emoji,
//! CJK, Arabic and Devanagari draw as tofu, so their test builds its harness with
//! `allow_missing_glyphs()` (the app's own policy) instead of kittest's default panic.

use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::{IssueKey, ProjectKey};

use super::*;
use crate::View;
use crate::fixtures::{self, StressSpec};

fn pk(k: &str) -> ProjectKey {
    ProjectKey::parse(k).unwrap()
}

fn on(ws: trackcraft_model::Workspace) -> TrackApp {
    TrackApp::new(Session::new(ws, fixtures::ME).with_clock(fixed_now))
}

fn page_titled(app: &TrackApp, title: &str) -> trackcraft_core::Id {
    app.session.ws.pages.values().find(|p| p.title == title).map(|p| p.id).unwrap()
}

fn space_home(app: &TrackApp, key: &str) -> trackcraft_core::Id {
    app.session.ws.spaces.get(&pk(key)).unwrap().homepage
}

/// The deepest page of the DEEP space.
fn deepest(app: &TrackApp) -> trackcraft_core::Id {
    let ws = &app.session.ws;
    ws.pages.values().filter(|p| p.space == pk("DEEP")).max_by_key(|p| ws.ancestors(p.id).len()).map(|p| p.id).unwrap()
}

fn stress_views(app: &TrackApp) -> Vec<View> {
    let mut v = vec![
        View::Home,
        View::Issues,
        View::Board(pk("BIG")),
        View::Board(pk("LONG")),
        View::Board(pk("INTL")),
        View::Board(pk("NONE")),
        View::Backlog(pk("LONG")),
        View::Issue(key(fixtures::HOSTILE_ISSUE)),
        View::Issue(key(fixtures::ORPHAN_ISSUE)),
        View::Issue(key(fixtures::ORPHAN_CHILD)),
        View::Issue(key(fixtures::LONG_WORD_ISSUE)),
        View::Issue(key("INTL-1")),
        View::Issue(key("BIG-1")),
        View::Page(space_home(app, "DEEP")),
        View::Page(deepest(app)),
        View::Page(page_titled(app, "Five thousand lines")),
        View::PageHistory(page_titled(app, "Five thousand lines")),
        View::Page(space_home(app, "BARE")),
    ];
    if let Some(sp) = app.session.ws.sprints.values().next() {
        v.push(View::SprintBoard(sp.project.clone(), sp.id));
    }
    v
}

/// Renders every view for a few frames, failing if any frame-set exceeds `budget`.
fn render_all(h: &mut Harness<'static, TrackApp>, budget: Duration) {
    let views = stress_views(h.state());
    for v in views {
        h.state_mut().navigate(v.clone());
        let t0 = Instant::now();
        h.run_steps(3);
        let took = t0.elapsed();
        eprintln!("{v:?}: {took:?}");
        assert!(took < budget, "{v:?} took {took:?}");
        // Expand the deep tree in the sidebar too, then scroll the view.
        let all: Vec<_> = h.state().session.ws.pages.keys().copied().collect();
        h.state_mut().expanded.extend(all);
        h.run_steps(1);
    }
}

#[test]
fn every_view_survives_the_stress_fixture_in_both_themes() {
    let ws = fixtures::stress(fixed_now);
    for dark in [false, true] {
        let mut h = harness(on(ws.clone()));
        h.state_mut().dark = Some(dark);
        render_all(&mut h, Duration::from_secs(20));
    }
}

#[test]
fn uncovered_scripts_draw_tofu_without_panicking() {
    let ws = fixtures::stress_with(fixed_now, StressSpec { issues: 50, uncovered_scripts: true });
    let app = on(ws);
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).allow_missing_glyphs().build_ui_state(
        |ui, app: &mut TrackApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    let intl: Vec<IssueKey> = h.state().session.ws.issues.keys().filter(|k| k.project == pk("INTL")).cloned().collect();
    assert!(intl.len() >= 7);
    for k in intl {
        h.state_mut().navigate(View::Issue(k));
        h.run_steps(2);
    }
    render_all(&mut h, Duration::from_secs(20));
}

#[test]
fn stress_fixture_has_every_hostile_shape() {
    let ws = fixtures::stress(fixed_now);
    assert_eq!(ws.projects.len(), 5);
    assert_eq!(ws.issues.keys().filter(|k| k.project == pk("BIG")).count(), 10_000);
    let hostile = &ws.issues[&key(fixtures::HOSTILE_ISSUE)];
    assert_eq!(hostile.summary.chars().count(), 255);
    assert_eq!(hostile.comments.len(), 500);
    assert_eq!(hostile.labels.len(), 100);
    assert!(!ws.users.contains_key(fixtures::GONE_USER));
    assert_eq!(hostile.assignee.as_deref(), Some(fixtures::GONE_USER));
    assert_eq!(hostile.reporter, fixtures::GONE_USER);
    assert_eq!(ws.issues[&key(fixtures::ORPHAN_ISSUE)].status, fixtures::ORPHAN_STATUS);
    let child = &ws.issues[&key(fixtures::ORPHAN_CHILD)];
    assert!(child.parent.as_ref().is_some_and(|p| !ws.issues.contains_key(p)));
    assert!(ws.issues.contains_key(&key(fixtures::LONG_WORD_ISSUE)));
    let depth = ws.pages.keys().map(|p| ws.ancestors(*p).len()).max().unwrap();
    assert!(depth >= 60, "deepest page has {depth} ancestors");
    let huge = ws.pages.values().find(|p| p.title == "Five thousand lines").unwrap();
    assert!(huge.body.blocks.len() > 5_000);
}

fn small_stress() -> TrackApp {
    on(fixtures::stress_with(fixed_now, StressSpec { issues: 20, ..Default::default() }))
}

#[test]
fn orphan_status_gets_other_column() {
    let mut h = harness(small_stress());
    let cols = crate::issues::board_columns(h.state(), &pk("LONG"));
    let last = cols.last().unwrap();
    assert_eq!(last.0, crate::issues::OTHER_STATUSES);
    assert_eq!(last.2, vec![key(fixtures::ORPHAN_ISSUE)]);
    h.state_mut().navigate(View::Board(pk("LONG")));
    h.run_steps(3);
    h.get_by_label(&crate::issues::OTHER_STATUSES.to_uppercase());
    h.get_by_label(&fixtures::ORPHAN_STATUS.to_uppercase());
    // Projects without orphans get no extra column.
    assert!(crate::issues::board_columns(h.state(), &pk("BIG")).iter().all(|c| c.0 != crate::issues::OTHER_STATUSES));
}

#[test]
fn unknown_assignee_renders_raw_id() {
    let mut h = harness(small_stress());
    h.state_mut().navigate(View::Issue(key(fixtures::HOSTILE_ISSUE)));
    h.run_steps(3);
    // The reporter row and every third comment show the raw id.
    assert!(h.query_all_by_label(fixtures::GONE_USER).count() >= 2);
}

#[test]
fn deleted_parent_is_not_a_link() {
    let mut h = harness(small_stress());
    h.state_mut().navigate(View::Issue(key(fixtures::ORPHAN_CHILD)));
    h.run_steps(3);
    h.get_by_label("LONG-3");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Link, "LONG-3").is_none());
}

#[test]
fn comments_render_fifty_at_a_time() {
    let mut h = harness(small_stress());
    h.state_mut().navigate(View::Issue(key(fixtures::HOSTILE_ISSUE)));
    h.run_steps(3);
    h.get_by_label("Comment 499");
    assert!(h.query_by_label("Comment 449").is_none());
    h.get_by_label("Show older comments (450 more)").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Comment 449");
    h.get_by_label("Show older comments (400 more)");
}

#[test]
fn empty_workspace_renders_every_view_with_empty_states() {
    for dark in [false, true] {
        let mut h = harness(on(fixtures::empty()));
        h.state_mut().dark = Some(dark);
        h.run_steps(3);
        h.get_by_label("This workspace is empty");
        h.get_by_label("Open command palette").click();
        h.run_steps(2);
        assert!(h.state().palette.is_some());
        h.state_mut().palette = None;
        h.state_mut().navigate(View::Issues);
        h.run_steps(3);
        h.get_by_label("No issues match this search");
    }
    // One project, one space, one sprint, nothing in them.
    let mut ws = fixtures::empty();
    ws.create_project("EMP", "Empty project", trackcraft_model::ProjectKind::Software, None).unwrap();
    ws.create_space("SPC", "Empty space", fixtures::ME, fixed_now()).unwrap();
    let sprint = ws.create_sprint("EMP", Some("Sprint zero"), None, None, None).unwrap();
    let mut h = harness(on(ws));
    h.state_mut().navigate(View::Board(pk("EMP")));
    h.run_steps(3);
    h.get_by_label("This project has no issues yet");
    h.state_mut().navigate(View::SprintBoard(pk("EMP"), sprint));
    h.run_steps(3);
    h.get_by_label("This sprint has no issues");
    let home = space_home(h.state(), "SPC");
    h.state_mut().navigate(View::Page(home));
    h.run_steps(3);
    h.get_by_label("This space has no pages yet");
    h.state_mut().navigate(View::Home);
    h.run_steps(3);
    h.get_by_label("Nothing assigned to you. Nice.");
    h.get_by_label("No pages yet. Pages you or your team write show up here.");
}

#[test]
fn board_filters_that_match_nothing_offer_to_clear() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Board(pk("LUME")));
    h.state_mut().board_text = "zzz no such card".into();
    h.run_steps(3);
    h.get_by_label("No cards match these filters");
    h.get_by_label("Clear filters").click();
    h.run_steps(2);
    assert!(h.state().board_text.is_empty());
}

#[test]
fn empty_page_and_no_comments_say_so() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-1")));
    h.run_steps(3);
    h.get_by_label("No comments yet. Start the conversation above.");
    let space = h.state().session.ws.spaces.keys().next().unwrap().to_string();
    let r = h.state_mut().session.run("page.create", &json!({"space": space, "title": "Blank"})).unwrap();
    let id = trackcraft_core::Id(r["id"].as_u64().unwrap());
    h.state_mut().navigate(View::Page(id));
    h.run_steps(3);
    h.get_by_label("This page is empty");
}
