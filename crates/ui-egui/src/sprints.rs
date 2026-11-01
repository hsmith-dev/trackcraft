//! Sprint dialogs (UI-12): start, complete, create, edit and delete.
//!
//! Each dialog runs its command(s) through `Session::run` when confirmed and shows the
//! engine's error inline instead of closing. Dates are typed as `YYYY-MM-DD` and checked here
//! first, so a typo gets a plain message before anything runs.

use egui::RichText;
use serde_json::{Value, json};
use trackcraft_core::{Id, ProjectKey};
use trackcraft_engine::Session;
use trackcraft_model::{SprintState, StatusCategory, Timestamp};

use crate::theme::Tokens;
use crate::widgets::field_row;
use crate::{Action, ComboBox, TrackApp, View};

/// Which sprint dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Start,
    Complete,
    Create,
    Edit,
    Delete,
}

/// Where completing a sprint sends its open issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveTo {
    Sprint(Id),
    New,
    Backlog,
}

#[derive(Debug, Clone)]
pub(crate) struct SprintDialog {
    pub kind: Kind,
    /// The sprint (every kind but Create).
    pub id: Option<Id>,
    pub project: ProjectKey,
    pub name: String,
    loaded_name: String,
    pub goal: String,
    loaded_goal: String,
    /// `YYYY-MM-DD` text.
    pub start: String,
    pub end: String,
    /// The Start dialog's duration preset in weeks; `None` is a custom end date.
    pub weeks: Option<i64>,
    pub move_to: MoveTo,
    pub error: Option<String>,
}

/// Duration presets in weeks.
const PRESETS: [i64; 4] = [1, 2, 3, 4];

fn ymd(t: Timestamp) -> String {
    t.format("%Y-%m-%d").to_string()
}

fn add_days(t: Timestamp, days: i64) -> Option<Timestamp> {
    Timestamp::from_timestamp(t.timestamp().checked_add(days.checked_mul(86_400)?)?, 0)
}

/// A typed `YYYY-MM-DD` date, or a message naming the field.
pub(crate) fn parse_date(field: &str, s: &str) -> Result<Timestamp, String> {
    let s = s.trim();
    let shaped = s.len() == 10 && s.char_indices().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() });
    let bad = || format!("{field}: write the date as YYYY-MM-DD, for example 2026-11-03.");
    if !shaped {
        return Err(bad());
    }
    format!("{s}T00:00:00Z").parse::<Timestamp>().map_err(|_| bad())
}

impl SprintDialog {
    /// A dialog of `kind` (`start`, `complete`, `create`, `edit`, `delete`) for sprint `id`, or
    /// for a new sprint in `project`.
    pub(crate) fn open(session: &Session, kind: &str, id: Option<Id>, project: &str) -> Result<SprintDialog, String> {
        let kind = match kind {
            "start" => Kind::Start,
            "complete" => Kind::Complete,
            "create" => Kind::Create,
            "edit" => Kind::Edit,
            "delete" => Kind::Delete,
            other => return Err(format!("unknown sprint dialog {}", other.chars().take(30).collect::<String>())),
        };
        let ws = &session.ws;
        let today = ymd(session.now());
        if kind == Kind::Create {
            let project = ws.project(project).map_err(|e| e.to_string())?.key.clone();
            let n = ws.sprints.values().filter(|s| s.project == project).count().saturating_add(1);
            let name = format!("{project} Sprint {n}");
            return Ok(SprintDialog {
                kind,
                id: None,
                project,
                name: name.clone(),
                loaded_name: name,
                goal: String::new(),
                loaded_goal: String::new(),
                start: String::new(),
                end: String::new(),
                weeks: None,
                move_to: MoveTo::Backlog,
                error: None,
            });
        }
        // Without an id: the project's active sprint (complete) or next planned one (others).
        let id = match id {
            Some(id) => id,
            None => {
                let key = ws.project(project).map_err(|_| "missing \"id\" (the sprint) or \"project\"".to_string())?.key.clone();
                let want = if kind == Kind::Complete { SprintState::Active } else { SprintState::Future };
                ws.project_sprints(&key).into_iter().find(|s| s.state == want).map(|s| s.id).ok_or(format!("{key} has no {} sprint", want.name()))?
            }
        };
        let sp = ws.sprint(id).map_err(|e| e.to_string())?;
        let ok_state = match kind {
            Kind::Start => sp.state == SprintState::Future,
            Kind::Complete => sp.state == SprintState::Active,
            _ => sp.state != SprintState::Closed,
        };
        if !ok_state {
            return Err(format!("{} is {}", sp.name, sp.state.name()));
        }
        let next = ws.project_sprints(&sp.project).into_iter().find(|s| s.state == SprintState::Future && s.id != id).map(|s| s.id);
        let mut d = SprintDialog {
            kind,
            id: Some(id),
            project: sp.project.clone(),
            name: sp.name.clone(),
            loaded_name: sp.name.clone(),
            goal: sp.goal.clone(),
            loaded_goal: sp.goal.clone(),
            start: sp.start.map(ymd).unwrap_or_default(),
            end: sp.end.map(ymd).unwrap_or_default(),
            weeks: None,
            move_to: next.map_or(MoveTo::Backlog, MoveTo::Sprint),
            error: None,
        };
        if kind == Kind::Start {
            // A preset when the saved dates are one, else two weeks from the saved start or today.
            let span = sp.start.zip(sp.end).map(|(s, e)| (e - s).num_days());
            match span {
                Some(days) if days % 7 == 0 && PRESETS.contains(&(days / 7)) => d.weeks = Some(days / 7),
                Some(_) => d.weeks = None,
                None => {
                    if d.start.is_empty() {
                        d.start = today;
                    }
                    d.weeks = Some(2);
                    d.apply_preset();
                }
            }
        }
        Ok(d)
    }

    /// With a preset chosen, the end date follows the start date.
    fn apply_preset(&mut self) {
        if let (Some(w), Ok(s)) = (self.weeks, parse_date("Start date", &self.start))
            && let Some(e) = add_days(s, w.saturating_mul(7))
        {
            self.end = ymd(e);
        }
    }

    /// Checks the typed dates; `required` for starting a sprint.
    fn dates(&self, required: bool) -> Result<(Option<Timestamp>, Option<Timestamp>), String> {
        let one = |field: &str, s: &str| if s.trim().is_empty() && !required { Ok(None) } else { parse_date(field, s).map(Some) };
        let (s, e) = (one("Start date", &self.start)?, one("End date", &self.end)?);
        if let (Some(s), Some(e)) = (s, e)
            && e <= s
        {
            return Err("The end date must be after the start date.".into());
        }
        Ok((s, e))
    }

    /// The command(s) confirming the dialog runs, or a message for the person.
    fn steps(&self) -> Result<Vec<(&'static str, Value)>, String> {
        let id = self.id.map(|i| i.0);
        let name = self.name.trim();
        let renamed = name != self.loaded_name.trim();
        if matches!(self.kind, Kind::Start | Kind::Create | Kind::Edit) && name.is_empty() {
            return Err("Give the sprint a name.".into());
        }
        Ok(match self.kind {
            Kind::Start => {
                let (s, e) = self.dates(true)?;
                let mut steps = vec![];
                if renamed {
                    steps.push(("sprint.edit", json!({"id": id, "name": name})));
                }
                let mut p = json!({"id": id, "startDate": s.map(ymd), "endDate": e.map(ymd)});
                if !self.goal.trim().is_empty() || !self.loaded_goal.is_empty() {
                    p["goal"] = json!(self.goal.trim());
                }
                steps.push(("sprint.start", p));
                steps
            }
            Kind::Complete => {
                let to = match self.move_to {
                    MoveTo::Sprint(s) => json!(s.0),
                    MoveTo::New => json!("new"),
                    MoveTo::Backlog => json!("backlog"),
                };
                vec![("sprint.complete", json!({"id": id, "moveOpenTo": to}))]
            }
            Kind::Create => {
                let (s, e) = self.dates(false)?;
                let mut p = json!({"project": self.project.to_string(), "name": name});
                if !self.goal.trim().is_empty() {
                    p["goal"] = json!(self.goal.trim());
                }
                if let Some(s) = s {
                    p["startDate"] = json!(ymd(s));
                }
                if let Some(e) = e {
                    p["endDate"] = json!(ymd(e));
                }
                vec![("sprint.create", p)]
            }
            Kind::Edit => {
                let (s, e) = self.dates(false)?;
                let mut p = json!({"id": id});
                if renamed {
                    p["name"] = json!(name);
                }
                if self.goal.trim() != self.loaded_goal.trim() {
                    p["goal"] = json!(self.goal.trim());
                }
                if let Some(s) = s {
                    p["startDate"] = json!(ymd(s));
                }
                if let Some(e) = e {
                    p["endDate"] = json!(ymd(e));
                }
                vec![("sprint.edit", p)]
            }
            Kind::Delete => vec![("sprint.delete", json!({"id": id}))],
        })
    }

    fn title(&self) -> &'static str {
        match self.kind {
            Kind::Start => "Start sprint",
            Kind::Complete => "Complete sprint",
            Kind::Create => "Create sprint",
            Kind::Edit => "Edit sprint",
            Kind::Delete => "Delete sprint",
        }
    }

    fn confirm_label(&self) -> &'static str {
        match self.kind {
            Kind::Start => "Start",
            Kind::Complete => "Complete sprint",
            Kind::Create => "Create",
            Kind::Edit => "Save",
            Kind::Delete => "Delete sprint",
        }
    }
}

/// Runs `steps` in order; on a failure, undoes the ones that ran so nothing half-happens.
fn run_steps(app: &mut TrackApp, steps: &[(&'static str, Value)]) -> Result<(), String> {
    for (n, (id, p)) in steps.iter().enumerate() {
        if let Err(e) = app.session.run(id, p) {
            for _ in 0..n {
                let _ = app.session.run("edit.undo", &json!({}));
            }
            return Err(e);
        }
    }
    Ok(())
}

/// The open sprint dialog, if any.
pub(crate) fn dialog(app: &mut TrackApp, ctx: &egui::Context, t: &Tokens) {
    let Some(mut d) = app.sprint_dialog.take() else { return };
    let ws = &app.session.ws;
    let sprint = d.id.and_then(|i| ws.sprints.get(&i)).cloned();
    if d.kind != Kind::Create && sprint.is_none() {
        return;
    }
    // Sub-tasks follow their parents, so they aren't counted on their own (as in the backlog).
    let in_sprint: Vec<_> = d.id.map(|id| ws.issues.values().filter(|i| i.sprint == Some(id) && !ws.is_subtask(i)).collect()).unwrap_or_default();
    let done = in_sprint.iter().filter(|i| ws.status_category(i) == StatusCategory::Done).count();
    let open = in_sprint.len().saturating_sub(done);
    let futures: Vec<(Id, String)> = ws
        .project_sprints(&d.project)
        .into_iter()
        .filter(|s| s.state == SprintState::Future && Some(s.id) != d.id)
        .map(|s| (s.id, s.name.clone()))
        .collect();
    let mut confirm = false;
    let mut close = false;
    let mut to_delete = false;
    let modal = egui::Modal::new(egui::Id::unique("sprint-dialog")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.heading(d.title());
        ui.add_space(8.0);
        match d.kind {
            Kind::Complete => {
                let name = sprint.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                ui.label(RichText::new(name).strong());
                let issues = |n: usize| if n == 1 { "1 issue".to_string() } else { format!("{n} issues") };
                ui.label(format!("{} done, {open} open.", issues(done)));
                ui.add_space(6.0);
                if open > 0 {
                    egui::Grid::new("sprint-complete-grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                        field_row(ui, t, "Move open issues to", |ui, label| {
                            let text = match d.move_to {
                                MoveTo::Sprint(s) => futures.iter().find(|f| f.0 == s).map_or_else(|| "A sprint".to_string(), |f| f.1.clone()),
                                MoveTo::New => "A new sprint".into(),
                                MoveTo::Backlog => "The backlog".into(),
                            };
                            ComboBox::from_id_salt("sprint-move-to")
                                .selected_text(text)
                                .width(240.0)
                                .show_ui(ui, |ui| {
                                    for (id, n) in &futures {
                                        ui.selectable_value(&mut d.move_to, MoveTo::Sprint(*id), n);
                                    }
                                    ui.selectable_value(&mut d.move_to, MoveTo::New, "A new sprint");
                                    ui.selectable_value(&mut d.move_to, MoveTo::Backlog, "The backlog");
                                })
                                .response
                                .labelled_by(label);
                        });
                    });
                    ui.label(RichText::new("Done issues stay with the completed sprint.").color(t.muted).size(12.0));
                } else {
                    ui.label(RichText::new("Every issue is done. Nice work.").color(t.muted));
                }
            }
            Kind::Delete => {
                let name = sprint.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                let n = in_sprint.len();
                let what = match n {
                    0 => "It has no issues.".to_string(),
                    1 => "Its 1 issue goes back to the backlog.".to_string(),
                    n => format!("Its {n} issues go back to the backlog."),
                };
                ui.label(format!("Delete {name}? {what} Undo brings it back."));
            }
            Kind::Start | Kind::Create | Kind::Edit => {
                egui::Grid::new("sprint-form-grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                    field_row(ui, t, "Name", |ui, label| {
                        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(300.0)).labelled_by(label);
                    });
                    if d.kind == Kind::Start {
                        field_row(ui, t, "Duration", |ui, label| {
                            let text = d.weeks.map_or_else(|| "Custom".to_string(), weeks_label);
                            let before = d.weeks;
                            ComboBox::from_id_salt("sprint-duration")
                                .selected_text(text)
                                .width(160.0)
                                .show_ui(ui, |ui| {
                                    for w in PRESETS {
                                        ui.selectable_value(&mut d.weeks, Some(w), weeks_label(w));
                                    }
                                    ui.selectable_value(&mut d.weeks, None, "Custom");
                                })
                                .response
                                .labelled_by(label);
                            if d.weeks != before {
                                d.apply_preset();
                            }
                        });
                    }
                    field_row(ui, t, "Start date", |ui, label| {
                        let r = ui.add(egui::TextEdit::singleline(&mut d.start).desired_width(120.0).hint_text("YYYY-MM-DD")).labelled_by(label);
                        if r.changed() {
                            d.apply_preset();
                        }
                    });
                    field_row(ui, t, "End date", |ui, label| {
                        let r = ui.add(egui::TextEdit::singleline(&mut d.end).desired_width(120.0).hint_text("YYYY-MM-DD")).labelled_by(label);
                        if r.changed() {
                            // Typing an end date is a custom duration.
                            d.weeks = None;
                        }
                    });
                    field_row(ui, t, "Goal", |ui, label| {
                        ui.add(
                            egui::TextEdit::multiline(&mut d.goal)
                                .desired_rows(2)
                                .desired_width(300.0)
                                .hint_text("What this sprint is for (optional)"),
                        )
                        .labelled_by(label);
                    });
                });
                if d.kind == Kind::Start && in_sprint.iter().all(|i| ws.status_category(i) == StatusCategory::Done) {
                    ui.label(RichText::new("Add an issue that isn't done before starting.").color(t.muted).size(12.0));
                }
            }
        }
        if let Some(e) = &d.error {
            ui.add_space(6.0);
            egui::Frame::new().fill(t.danger_soft).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.label(RichText::new(e).color(t.danger));
            });
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let fill = if d.kind == Kind::Delete { t.danger } else { t.accent };
            let fg = if d.kind == Kind::Delete { t.bg } else { t.on_accent };
            confirm |= ui.add(egui::Button::new(RichText::new(d.confirm_label()).color(fg).strong()).fill(fill)).clicked();
            close |= ui.button("Cancel").clicked();
            if d.kind == Kind::Edit {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    to_delete |= ui.button(RichText::new("Delete sprint").color(t.danger)).clicked();
                });
            }
        });
    });
    if modal.should_close() {
        close = true;
    }
    if to_delete {
        d.kind = Kind::Delete;
        d.error = None;
        app.sprint_dialog = Some(d);
        return;
    }
    if confirm {
        match d.steps().and_then(|steps| run_steps(app, &steps)) {
            Ok(()) => {
                app.jql_ran = None;
                let name = d.name.trim().to_string();
                app.notify(match d.kind {
                    Kind::Start => format!("{name} started"),
                    Kind::Complete => format!("{name} completed"),
                    Kind::Create => format!("Created {name}"),
                    Kind::Edit => format!("Saved {name}"),
                    Kind::Delete => format!("Deleted {name}"),
                });
                if d.kind == Kind::Delete && matches!(app.view, View::SprintBoard(_, s) if Some(s) == d.id) {
                    app.push(Action::Nav(View::Board(d.project.clone())));
                }
                return;
            }
            Err(e) => {
                d.error = Some(e);
                close = false;
            }
        }
    }
    if !close {
        app.sprint_dialog = Some(d);
    }
}

fn weeks_label(w: i64) -> String {
    if w == 1 { "1 week".into() } else { format!("{w} weeks") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_strictly() {
        assert!(parse_date("Start date", "2026-11-03").is_ok());
        for bad in ["2026-13-01", "2026-02-30", "11/03/2026", "2026-1-3", "", "2026-11-03x", "２０２６-11-03"] {
            assert!(parse_date("Start date", bad).is_err(), "{bad}");
        }
    }
}
