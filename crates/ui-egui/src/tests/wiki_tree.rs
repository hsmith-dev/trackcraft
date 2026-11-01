//! Page tree moves (UI-30): drag in the sidebar tree, Move to… and Copy.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use trackcraft_core::Id;

use super::*;

/// ENG: home 22 › [Architecture overview 24 › [25, 26], Release checklist 27].
const HOME: u64 = 22;

fn order(h: &Harness<'static, TrackApp>, parent: u64) -> Vec<u64> {
    h.state().session.ws.children(Id(parent)).iter().map(|p| p.id.0).collect()
}

/// The sidebar row (not a page-body link) with this title.
fn row(h: &Harness<'static, TrackApp>, title: &str) -> egui::Rect {
    h.get_all_by_label(title).map(|n| n.rect()).find(|r| r.left() < 360.0).unwrap()
}

fn drag(h: &mut Harness<'static, TrackApp>, from: egui::Pos2, to: egui::Pos2) {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(button(from, true));
    h.run_steps(1);
    for n in 1..=6 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (n as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(button(to, false));
    h.run_steps(3);
}

#[test]
fn tree_drag_reorders_siblings_and_makes_child() {
    let mut h = harness(app());
    h.run_steps(3);
    assert_eq!(order(&h, HOME), vec![24, 27]);
    // Release checklist onto the top edge of Architecture overview: before it.
    let from = row(&h, "Release checklist");
    let onto = row(&h, "Architecture overview");
    drag(&mut h, from.center(), egui::pos2(onto.center().x, onto.top() + 1.0));
    assert_eq!(order(&h, HOME), vec![27, 24]);
    // And back below it: after.
    let from = row(&h, "Release checklist");
    let onto = row(&h, "Architecture overview");
    drag(&mut h, from.center(), egui::pos2(onto.center().x, onto.bottom() - 1.0));
    assert_eq!(order(&h, HOME), vec![24, 27]);
    // Onto the middle: it becomes a child (the target opens).
    let from = row(&h, "Release checklist");
    let onto = row(&h, "Architecture overview");
    drag(&mut h, from.center(), onto.center());
    assert_eq!(h.state().session.ws.page(Id(27)).unwrap().parent, Some(Id(24)));
    assert!(h.state().expanded.contains(&Id(24)));
}

#[test]
fn tree_drop_into_own_subtree_is_refused() {
    let mut h = harness(app());
    h.state_mut().expanded.insert(Id(24));
    h.run_steps(3);
    let from = row(&h, "Architecture overview");
    let onto = row(&h, "Sync protocol");
    drag(&mut h, from.center(), onto.center());
    assert_eq!(h.state().session.ws.page(Id(24)).unwrap().parent, Some(Id(HOME)));
    assert_eq!(h.state().session.ws.page(Id(25)).unwrap().parent, Some(Id(24)));
}

#[test]
fn copy_with_children_creates_the_subtree() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "page", "id": 24}})).unwrap();
    h.run_steps(3);
    h.get_by_label("More page actions").click();
    h.run_steps(2);
    h.get_by_label("Copy\u{2026}").click();
    h.run_steps(3);
    h.get_by_label("Copy").click();
    h.run_steps(4);
    let ws = &h.state().session.ws;
    let copy = ws.pages.values().find(|p| p.title == "Architecture overview (copy)").unwrap();
    assert_eq!(copy.parent, Some(Id(HOME)));
    let kids: Vec<String> = ws.children(copy.id).iter().map(|p| p.title.clone()).collect();
    assert_eq!(kids.len(), 2);
    assert!(kids[0].starts_with("Sync protocol") && kids[1].starts_with("Offline cache"), "{kids:?}");
    assert_eq!(h.state().view, crate::View::Page(copy.id));
}

#[test]
fn move_picker_places_a_page() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.pageDialog", "params": {"kind": "move", "id": 27}})).unwrap();
    h.run_steps(4);
    // Every space is offered (UI-fixes: pages move between spaces with their subtree).
    assert!(h.query_all_by_label("Team Home (space home)").next().is_some());
    // The dialog's row (the sidebar and the home view list it too) is the last one drawn.
    h.get_all_by_label("Sync protocol").last().unwrap().click();
    h.run_steps(2);
    h.get_by_label("Before").click();
    h.run_steps(2);
    match &h.state().wiki.dialog {
        Some(crate::wiki::Dialog::Move(f)) => assert!(f.target.is_some() && f.place == crate::wiki::tree::Place::Before, "{f:?}"),
        other => panic!("{other:?}"),
    }
    h.get_by_label("Move").click();
    h.run_steps(3);

    let ws = &h.state().session.ws;
    let onboarding = ws.page(Id(25)).unwrap();
    let moved = ws.page(Id(27)).unwrap();
    assert_eq!(moved.parent, onboarding.parent);
    let sibs: Vec<Id> = ws.children(onboarding.parent.unwrap()).iter().map(|p| p.id).collect();
    let (a, b) = (sibs.iter().position(|i| *i == Id(27)).unwrap(), sibs.iter().position(|i| *i == onboarding.id).unwrap());
    assert_eq!(a + 1, b);
    assert!(h.state().wiki.dialog.is_none());
}
