//! The unsaved-edit guard: leaving a dirty page editor, issue summary/description edit, or
//! closing a create dialog with typed text holds the action and asks "Discard your changes?".
//! Only "Discard" lets it through; "Keep editing" (the default, also Escape) drops it.
//!
//! The control channel never opens this dialog: its requests fail with "unsaved changes"
//! unless they pass `"force": true` (see [`TrackApp::control`]).

use egui::RichText;

use crate::theme::Tokens;
use crate::{Action, PageEditor, TrackApp, View};

/// At most this many actions wait behind the dialog (one click usually holds one or two).
const MAX_HELD: usize = 8;

#[derive(Debug, Default)]
pub(crate) struct Guard {
    /// Replayed in order on "Discard".
    pub actions: Vec<Action>,
    /// Focus "Keep editing" on the dialog's first frame, so Enter keeps the text too.
    focused: bool,
}

impl TrackApp {
    /// Whether showing `to` would throw away typed text.
    pub(crate) fn would_lose(&self, to: &View) -> bool {
        *to != self.view && (self.page_edit.as_ref().is_some_and(PageEditor::dirty) || self.issue_edit_dirty())
    }

    /// An open summary or description edit differs from the issue.
    fn issue_edit_dirty(&self) -> bool {
        let View::Issue(k) = &self.view else { return false };
        let Some(i) = self.session.ws.issues.get(k) else { return false };
        let ui = &self.issue_ui;
        ui.editing_summary.as_ref().is_some_and(|s| *s != i.summary)
            || ui.editing_description.as_ref().is_some_and(|d| *d != i.description.to_markdown())
    }

    /// Holds `a` until the person answers the dialog (opening it if needed).
    pub(crate) fn hold(&mut self, a: Action) {
        let g = self.guard.get_or_insert_with(Guard::default);
        if g.actions.len() < MAX_HELD {
            g.actions.push(a);
        }
    }

    /// Throws away every edit that leaving the view would lose.
    fn drop_edits(&mut self) {
        self.page_edit = None;
        self.issue_ui.editing_summary = None;
        self.issue_ui.editing_description = None;
    }

    /// For control requests: fail instead of asking, unless `force` discards the edits.
    pub(crate) fn leave_for_control(&mut self, loses: bool, force: bool) -> Result<(), String> {
        if !loses {
            return Ok(());
        }
        if !force {
            return Err("unsaved changes (pass \"force\": true to discard them)".into());
        }
        self.drop_edits();
        Ok(())
    }

    /// "Discard": drop the edits, then do what was held.
    fn discard_and_continue(&mut self) {
        let Some(g) = self.guard.take() else { return };
        for a in g.actions {
            match a {
                Action::CloseCreate => self.create = None,
                other => {
                    self.drop_edits();
                    self.apply(other);
                }
            }
        }
    }
}

/// The "Discard your changes?" dialog.
pub(crate) fn dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(g) = app.guard.as_mut() else { return };
    let focus = !std::mem::replace(&mut g.focused, true);
    let what = if g.actions.iter().all(|a| matches!(a, Action::CloseCreate)) {
        "The new issue has text you typed. Closing the dialog throws it away."
    } else {
        "You have changes that are not saved. Leaving now throws them away."
    };
    let mut keep = false;
    let mut discard = false;
    let modal = egui::Modal::new(egui::Id::unique("discard-guard")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.heading("Discard your changes?");
        ui.add_space(4.0);
        ui.label(RichText::new(what).color(t.muted));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let k = ui
                .add(egui::Button::new(RichText::new("Keep editing").color(t.on_accent).strong()).fill(t.accent))
                .on_hover_text("Keep editing (Escape)");
            if focus {
                k.request_focus();
            }
            keep |= k.clicked();
            discard |= ui.button(RichText::new("Discard").color(t.danger)).clicked();
        });
    });
    if discard {
        app.discard_and_continue();
    } else if keep || modal.should_close() {
        app.guard = None;
    }
}
