//! The board.

use super::*;
use crate::View;

#[test]
fn board_keys_select_and_shift_right_transitions() {
    let mut h = harness(app());
    h.state_mut().navigate(View::Board(trackcraft_core::ProjectKey::parse("LUME").unwrap()));
    h.run_steps(3);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    let sel = h.state().board_sel.clone().unwrap();
    let before = h.state().session.ws.issues[&sel].status.clone();
    let cols = crate::issues::board_columns(h.state(), &sel.project);
    let at = cols.iter().position(|c| c.0 == before).unwrap();
    let next = cols[at + 1].0.clone();
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&sel].status, next);
    // `[` moves it back; the selection follows the card.
    h.key_press(egui::Key::OpenBracket);
    h.run_steps(2);
    assert_eq!(h.state().session.ws.issues[&sel].status, before);
    assert_eq!(h.state().board_sel, Some(sel.clone()));
    // j moves within the column, Enter opens.
    h.key_press(egui::Key::J);
    h.run_steps(2);
    let below = h.state().board_sel.clone().unwrap();
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().view, View::Issue(below));
}
