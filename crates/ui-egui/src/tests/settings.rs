//! Settings (UI-7): appearance preferences persist, workspace rename, people, autosave.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;
use crate::View;
use crate::settings::{self, Tab};
use crate::theme::Density;

fn open(h: &mut Harness<'static, TrackApp>, tab: &str) {
    h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "settings", "tab": tab}})).unwrap();
    h.run_steps(3);
}

#[test]
fn settings_render_every_tab_in_both_themes() {
    for dark in [false, true] {
        let mut h = harness(app());
        h.state_mut().dark = Some(dark);
        for tab in ["appearance", "workspace", "people", "shortcuts"] {
            open(&mut h, tab);
        }
        assert!(h.query_all_by_label("Settings").count() >= 1);
    }
    let mut h = harness(app());
    assert!(h.state_mut().control(&json!({"method": "ui.navigate", "params": {"view": "settings", "tab": "nope"}})).is_err());
    open(&mut h, "");
    assert_eq!(h.state().view, View::Settings(Tab::Appearance));
}

#[test]
fn theme_tri_state_persists_across_frames() {
    let mut h = harness(app());
    open(&mut h, "appearance");
    h.get_by_label("Dark").click();
    h.run_steps(3);
    assert_eq!(h.state().dark, Some(true));
    assert_eq!(settings::saved(&h.ctx).map(|s| s.0), Some(Some(true)));
    h.get_by_label("System").click();
    h.run_steps(3);
    assert_eq!(h.state().dark, None, "System is a state of its own, not light");
    assert_eq!(settings::saved(&h.ctx).map(|s| s.0), Some(None));
    h.get_by_label("Light").click();
    h.run_steps(5);
    assert_eq!(h.state().dark, Some(false));
    // A fresh app on the same egui memory (a restart, or File > Open) loads the choice back.
    h.state_mut().dark = None;
    h.state_mut().settings.prefs = settings::Prefs::default();
    h.run_steps(2);
    assert_eq!(h.state().dark, Some(false));
}

#[test]
fn compact_density_changes_item_spacing() {
    let mut h = harness(app());
    open(&mut h, "appearance");
    let before = h.ctx.global_style().spacing.item_spacing;
    h.get_by_label("Compact").click();
    h.run_steps(3);
    assert_eq!(h.state().density(), Density::Compact);
    let after = h.ctx.global_style().spacing.item_spacing;
    assert!(after.y < before.y && after.x < before.x, "{before:?} -> {after:?}");
    assert_eq!(settings::saved(&h.ctx).map(|s| s.1), Some(true));
    h.get_by_label("Comfortable").click();
    h.run_steps(3);
    assert_eq!(h.ctx.global_style().spacing.item_spacing, before);
}

#[test]
fn rename_runs_workspace_rename_and_updates_the_title() {
    let mut h = harness(app());
    open(&mut h, "workspace");
    h.state_mut().settings.name = Some("Renamed team".into());
    h.run_steps(2);
    h.get_by_label("Rename").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.name, "Renamed team");
    assert!(h.state().title().starts_with("Renamed team"));
    assert!(h.state().session.can_undo(), "the rename is an undoable command");
    assert!(h.state().settings.name_error.is_none());
}

#[test]
fn empty_workspace_name_shows_the_error_inline() {
    let mut h = harness(app());
    let name = h.state().session.ws.name.clone();
    open(&mut h, "workspace");
    h.state_mut().settings.name = Some("   ".into());
    h.run_steps(2);
    h.get_by_label("Rename").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.name, name, "nothing changed");
    let err = h.state().settings.name_error.clone().unwrap();
    h.get_by_label(&err);
}

#[test]
fn add_and_edit_a_person() {
    let mut h = harness(app());
    open(&mut h, "people");
    h.state_mut().settings.add_id = "ann".into();
    h.state_mut().settings.add_name = "Ann Lee".into();
    h.run_steps(2);
    h.get_by_label("Add person").click();
    h.run_steps(3);
    assert_eq!(h.state().session.ws.users.get("ann").map(|u| u.display_name.as_str()), Some("Ann Lee"));
    assert!(h.query_all_by_label("ann").count() >= 1);
    // A duplicate id is refused inline.
    h.state_mut().settings.add_id = "ann".into();
    h.run_steps(2);
    h.get_by_label("Add person").click();
    h.run_steps(3);
    assert!(h.state().settings.add_error.is_some());

    // Edit: the first Edit button belongs to the first user (ann sorts first).
    h.get_all_by_label("Edit").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().settings.edit.as_ref().map(|e| e.id.as_str()), Some("ann"));
    if let Some(e) = h.state_mut().settings.edit.as_mut() {
        e.name = "Ann Lee-Park".into();
        e.email = "ann@example.com".into();
    }
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(3);
    let u = h.state().session.ws.users.get("ann").cloned().unwrap();
    assert_eq!((u.display_name.as_str(), u.email.as_deref()), ("Ann Lee-Park", Some("ann@example.com")));
    assert!(h.state().settings.edit.is_none());

    // An empty name is refused inline and the row stays open.
    h.get_all_by_label("Edit").next().unwrap().click();
    h.run_steps(3);
    if let Some(e) = h.state_mut().settings.edit.as_mut() {
        e.name = String::new();
    }
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert!(h.state().settings.edit_error.is_some());
    assert_eq!(h.state().session.ws.users.get("ann").map(|u| u.display_name.as_str()), Some("Ann Lee-Park"));
}

#[test]
fn act_as_switches_the_acting_user() {
    let mut h = harness(app());
    open(&mut h, "people");
    let other = h.state().session.ws.users.keys().find(|k| *k != "riley").cloned().unwrap();
    let idx = h.state().session.ws.users.keys().position(|k| *k == other).unwrap();
    h.get_all_by_label("Act as").nth(idx).unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().session.user, other);
}

#[test]
fn autosave_interval_preference_is_read_by_autosave_secs() {
    let mut h = harness(app());
    assert_eq!(h.state().autosave_secs(), Some(settings::DEFAULT_AUTOSAVE_SECS));
    open(&mut h, "workspace");
    h.state_mut().settings.prefs.autosave_secs = 120;
    h.run_steps(2);
    assert_eq!(h.state().autosave_secs(), Some(120));
    assert_eq!(settings::saved(&h.ctx).map(|s| s.3), Some(120));
    h.get_by_label("Save automatically").click();
    h.run_steps(2);
    assert_eq!(h.state().autosave_secs(), None);
    // A hostile saved interval is clamped.
    h.state_mut().settings.prefs.autosave = true;
    h.state_mut().settings.prefs.autosave_secs = u64::MAX;
    assert_eq!(h.state().autosave_secs(), Some(3600));
}

#[test]
fn settings_open_from_shortcut_palette_and_avatar_menu() {
    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.key", "params": {"key": ",", "command": true}})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Settings(Tab::Appearance));

    let mut h = harness(app());
    h.state_mut().control(&json!({"method": "ui.palette"})).unwrap();
    h.run_steps(2);
    h.state_mut().palette.as_mut().unwrap().query = "settings".into();
    h.run_steps(2);
    h.get_by_label("Settings").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Settings(Tab::Appearance));

    let mut h = harness(app());
    h.run_steps(2);
    let me = crate::widgets::name_of(&h.state().session.ws, Some("riley"));
    h.get_by_role_and_label(egui::accesskit::Role::Button, &me).click();
    h.run_steps(3);
    // The menu item carries its shortcut text.
    h.get_by_label_contains("Settings").click();
    h.run_steps(3);
    assert_eq!(h.state().view, View::Settings(Tab::Appearance));
}

#[test]
fn shortcuts_tab_lists_the_keymap() {
    let mut h = harness(app());
    open(&mut h, "shortcuts");
    h.get_by_label_contains("later release");
    h.get_by_label(&crate::key("Mod+,"));
    for scope in crate::keys::Scope::ALL {
        if crate::keys::table().iter().any(|b| b.scope == scope) {
            assert!(h.query_all_by_label(scope.title()).count() >= 1, "{}", scope.title());
        }
    }
}
