//! Keyboard: the scoped keymap, the help overlay, typing vs shortcuts, chords, injected keys.

use egui_kittest::kittest::Queryable as _;
use serde_json::json;

use super::*;
use crate::View;
use crate::keys::{Act, Chord, Scope, table};

#[test]
fn question_mark_opens_help_and_escape_closes_it() {
    let mut h = harness(app());
    h.run_steps(2);
    h.key_press(egui::Key::Questionmark);
    h.run_steps(2);
    assert!(h.state().help);
    // Single keys are off behind the overlay.
    h.key_press(egui::Key::C);
    h.run_steps(2);
    assert!(h.state().create.is_none());
    // Every entry of the keymap is listed, keys and label.
    for b in table() {
        assert!(h.query_all_by_label(&b.label).next().is_some(), "{} missing from the overlay", b.label);
        assert!(h.query_all_by_label(&crate::key(&b.keys)).next().is_some(), "{} missing from the overlay", b.keys);
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.state().help);
    // Shift+/ from backends that report the physical key.
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Slash);
    h.run_steps(2);
    assert!(h.state().help);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // The palette entry opens it too.
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text("keyboard shortcuts".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().palette.is_none());
    assert!(h.state().help);
}

#[test]
fn typing_in_the_search_box_does_not_trigger_shortcuts() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME"}})).unwrap();
    h.run_steps(3);
    h.key_press(egui::Key::Slash);
    h.run_steps(3);
    for (k, text) in [(egui::Key::J, "j"), (egui::Key::C, "c"), (egui::Key::Questionmark, "?"), (egui::Key::K, "k")] {
        h.key_press(k);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(2);
    }
    assert!(h.state().list_sel.is_none());
    assert!(h.state().create.is_none());
    assert!(!h.state().help);
    assert_eq!(h.state().jql, "project = LUMEjc?k");
}

#[test]
fn control_key_injects_presses() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.search", "params": {"jql": "project = LUME ORDER BY key"}})).unwrap();
    h.run_steps(3);
    h.state_mut().control(&json!({"method": "ui.key", "params": {"key": "ArrowDown"}})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().list_sel, Some(key("LUME-1")));
    h.state_mut().control(&json!({"method": "ui.key", "params": {"key": "?"}})).unwrap();
    h.run_steps(2);
    assert!(h.state().help);
    assert!(h.state_mut().control(&json!({"method": "ui.key", "params": {"key": "NotAKey"}})).is_err());
}

#[test]
fn every_registry_shortcut_is_in_the_keymap() {
    for c in trackcraft_engine::commands().iter().filter(|c| !c.shortcut.is_empty()) {
        let b = table().iter().find(|b| b.act == Act::Cmd(c.id));
        assert!(b.is_some_and(|b| !b.seqs.is_empty()), "{} ({}) is not bound", c.id, c.shortcut);
    }
    // The UI-only rows parse too (documentation rows aside).
    for b in table().iter().filter(|b| b.act != Act::Doc) {
        assert!(!b.seqs.is_empty(), "{} does not parse", b.keys);
    }
}

#[test]
fn no_duplicate_chord_within_a_scope() {
    for scope in Scope::ALL {
        let mut seen: Vec<(Vec<Chord>, &str)> = Vec::new();
        for b in table().iter().filter(|b| b.scope == scope) {
            for seq in &b.seqs {
                if let Some((_, other)) = seen.iter().find(|(s, _)| s == seq) {
                    panic!("{scope:?}: {} is bound to both {other} and {}", b.keys, b.label);
                }
                seen.push((seq.clone(), &b.label));
            }
        }
    }
    // A sequence's first key is never also a single key where both are active.
    for scope in Scope::ALL {
        let active: Vec<_> = table().iter().filter(|b| b.scope == scope || b.scope == Scope::Global).collect();
        let firsts: Vec<Chord> = active.iter().flat_map(|b| b.seqs.iter()).filter(|s| s.len() > 1).filter_map(|s| s.first().copied()).collect();
        for b in &active {
            for seq in b.seqs.iter().filter(|s| s.len() == 1) {
                assert!(!seq.first().is_some_and(|c| firsts.contains(c)), "{} shadows a chord in {scope:?}", b.keys);
            }
        }
    }
}

#[test]
fn redo_while_typing_does_not_change_the_workspace() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": "issue.assignToMe", "params": {"key": "LUME-3"}}})).unwrap();
    h.state_mut().control(&json!({"method": "engine.execute", "params": {"command": "edit.undo"}})).unwrap();
    h.run_steps(2);
    assert!(h.state().session.can_redo());
    h.state_mut().navigate(View::Issues);
    h.run_steps(3);
    h.key_press(egui::Key::Slash);
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
    h.run_steps(2);
    assert!(h.state().session.can_redo(), "redo ran while typing");
    assert_ne!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("riley"));
    // Not typing (Escape leaves the field), it redoes.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.state_mut().navigate(View::Home);
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("riley"));
}

#[test]
fn c_is_a_child_page_on_a_page_and_an_issue_elsewhere() {
    let mut h = harness(app());
    let page = child_page(&h);
    h.state_mut().navigate(View::Page(page));
    h.run_steps(3);
    h.key_press(egui::Key::C);
    h.run_steps(3);
    let form = h.state().new_page.as_ref().map(|f| (f.parent, f.space.clone()));
    let space = h.state().session.ws.pages[&page].space.to_string();
    assert_eq!(form, Some((Some(page), space)));
    assert!(h.state().create.is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().new_page.is_none());
    // E on a page edits it.
    h.key_press(egui::Key::E);
    h.run_steps(3);
    assert!(h.state().page_edit.as_ref().is_some_and(|e| e.id == page));
    // Elsewhere, C creates an issue.
    let lume = trackcraft_core::ProjectKey::parse("LUME").unwrap();
    for v in [View::Home, View::Issues, View::Board(lume), View::Issue(key("LUME-2"))] {
        let mut h = harness(app());
        h.state_mut().navigate(v.clone());
        h.run_steps(3);
        h.key_press(egui::Key::C);
        h.run_steps(3);
        assert!(h.state().create.is_some(), "C on {v:?}");
        assert!(h.state().new_page.is_none());
    }
}

#[test]
fn issue_view_letter_shortcuts() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-3")));
    h.run_steps(3);
    // E opens the description editor.
    h.key_press(egui::Key::E);
    h.run_steps(3);
    assert!(h.state().issue_ui.editing_description.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().issue_ui.editing_description.is_none());
    // I assigns to me.
    h.key_press(egui::Key::I);
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].assignee.as_deref(), Some("riley"));
    // A opens the assignee picker.
    assert!(h.query_by_label("Unassigned").is_none());
    h.key_press(egui::Key::A);
    h.run_steps(4);
    assert!(h.query_by_label("Unassigned").is_some(), "assignee picker did not open");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    // T opens the status menu (LUME-3 is To Do, so "In Progress" is offered).
    let before = h.query_all_by_label("In Progress").count();
    h.key_press(egui::Key::T);
    h.run_steps(4);
    assert!(h.query_all_by_label("In Progress").count() > before, "status menu did not open");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    // M focuses the comment box.
    h.key_press(egui::Key::M);
    h.run_steps(3);
    h.event(egui::Event::Text("hi".into()));
    h.run_steps(2);
    assert_eq!(h.state().issue_ui.comment, "hi");
}

#[test]
fn l_on_a_page_edits_its_labels() {
    let mut h = harness(app());
    let page = child_page(&h);
    h.state_mut().navigate(View::Page(page));
    h.run_steps(3);
    h.key_press(egui::Key::L);
    h.run_steps(3);
    assert!(h.state().page_labels.is_some());
    h.state_mut().page_labels.as_mut().unwrap().1 = "design  howto".into();
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert!(h.state().page_labels.is_none());
    assert_eq!(h.state().session.ws.pages[&page].labels, vec!["design".to_string(), "howto".to_string()]);
}

#[test]
fn g_chords_navigate() {
    let mut h = harness(app());
    h.run_steps(2);
    h.key_press(egui::Key::G);
    h.run_steps(2);
    h.key_press(egui::Key::I);
    h.run_steps(3);
    assert_eq!(h.state().view, View::Issues);
    // The second key of the chord does not also fire its single-key binding.
    assert!(h.state().list_sel.is_none());
    h.key_press(egui::Key::G);
    h.run_steps(1);
    h.key_press(egui::Key::B);
    h.run_steps(3);
    assert!(matches!(h.state().view, View::Board(_)));
    h.key_press(egui::Key::G);
    h.run_steps(1);
    h.key_press(egui::Key::P);
    h.run_steps(3);
    let home = h.state().session.ws.spaces.values().next().map(|s| s.homepage);
    assert_eq!(Some(h.state().view.clone()), home.map(View::Page));
    h.key_press(egui::Key::G);
    h.run_steps(1);
    h.key_press(egui::Key::H);
    h.run_steps(3);
    assert_eq!(h.state().view, View::Home);
    // After the timeout the second key is on its own.
    h.key_press(egui::Key::G);
    h.run_steps(1);
    h.state_mut().chord.as_mut().unwrap().1 -= 5.0;
    h.key_press(egui::Key::I);
    h.run_steps(3);
    assert_eq!(h.state().view, View::Home);
}

#[test]
fn letters_in_the_search_box_trigger_nothing() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-3")));
    h.run_steps(3);
    let assignee = h.state().session.ws.issues[&key("LUME-3")].assignee.clone();
    h.key_press(egui::Key::Slash);
    h.run_steps(3);
    for k in [egui::Key::G, egui::Key::I, egui::Key::A, egui::Key::E, egui::Key::M, egui::Key::T, egui::Key::C, egui::Key::L] {
        h.key_press(k);
        h.run_steps(2);
    }
    assert_eq!(h.state().view, View::Issue(key("LUME-3")));
    assert!(h.state().create.is_none());
    assert!(h.state().chord.is_none());
    assert!(h.state().issue_ui.editing_description.is_none());
    assert!(!h.state().issue_ui.open_assignee && !h.state().issue_ui.open_status);
    assert_eq!(h.state().session.ws.issues[&key("LUME-3")].assignee, assignee);
}

#[test]
fn palette_shows_shortcuts_for_the_view() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Issue(key("LUME-3")));
    h.run_steps(3);
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(3);
    h.event(egui::Event::Text("assign to me".into()));
    h.run_steps(3);
    assert!(h.query_by_label("I").is_some(), "no shortcut next to Assign to Me");
}

#[test]
fn backlog_shortcuts_match_the_overlay_table() {
    // `backlog::SHORTCUTS` (the backlog's own list) and the Backlog scope of the keymap table
    // must describe the same keys, or the `?` overlay drifts from what the backlog does.
    let table_rows: Vec<(String, String)> = table().iter().filter(|b| b.scope == Scope::Backlog).map(|b| (b.keys.clone(), b.label.clone())).collect();
    let own: Vec<(String, String)> = crate::backlog::SHORTCUTS.iter().map(|(k, l)| ((*k).to_string(), (*l).to_string())).collect();
    assert_eq!(table_rows, own);
}

#[test]
fn single_keys_are_off_behind_sprint_dialogs() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "backlog", "project": "LUME"}})).unwrap();
    h.run_steps(2);
    h.state_mut().control(&json!({"method": "ui.sprintDialog", "params": {"kind": "create", "project": "LUME"}})).unwrap();
    h.run_steps(2);
    assert!(h.state().sprint_dialog.is_some());
    // Take focus away from the dialog's text field so only the modal gate can stop `C`.
    h.ctx.memory_mut(|m| m.request_focus(egui::Id::NULL));
    h.run_steps(1);
    h.key_press(egui::Key::C);
    h.run_steps(2);
    assert!(h.state().create.is_none(), "C opened create-issue behind the sprint dialog");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().sprint_dialog.is_none(), "Escape closes the sprint dialog");
}
