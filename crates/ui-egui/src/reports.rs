//! Reports (UI-24): a project's Burndown, Velocity, Cumulative flow and Created vs resolved
//! charts (`report.*` queries), each painted by [`crate::charts`] with a legend, hover tooltips
//! and a "Show data" table; plus the Home dashboard's report gadgets.
//!
//! Reports are queries, so under enforcement they only count what the acting user can browse.
//! Results are cached per workspace revision and user; the clock moving on doesn't refresh them
//! until something changes (or the view is reopened).

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use serde_json::{Value, json};
use trackcraft_core::{Id, ProjectKey};
use trackcraft_model::SprintState;

use crate::charts::{self, Chart, Mark, Series};
use crate::theme::Tokens;
use crate::widgets::{card, empty_state, field_row};
use crate::{Action, ComboBox, TrackApp, View};

/// A report tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Burndown,
    Velocity,
    Cfd,
    CreatedResolved,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Burndown, Tab::Velocity, Tab::Cfd, Tab::CreatedResolved];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Burndown => "Burndown",
            Tab::Velocity => "Velocity",
            Tab::Cfd => "Cumulative flow",
            Tab::CreatedResolved => "Created vs resolved",
        }
    }

    /// `ui.reports`' `"tab"`: `burndown`, `velocity`, `cfd`, `createdResolved` (any case).
    pub fn parse(s: &str) -> Option<Tab> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "burndown" => Tab::Burndown,
            "velocity" => Tab::Velocity,
            "cfd" | "cumulativeflow" | "cumulative flow" => Tab::Cfd,
            "createdresolved" | "created vs resolved" | "created" => Tab::CreatedResolved,
            _ => return None,
        })
    }
}

/// The ranges offered for the day-based reports.
const RANGES: [usize; 3] = [14, 30, 90];

/// Reports view state: the tab, choices, the data table toggle and the query cache.
#[derive(Debug, Default)]
pub(crate) struct ReportsUi {
    pub tab: Tab,
    /// The burndown's sprint (`None` = the active one, else the latest started).
    pub sprint: Option<Id>,
    /// Count issues instead of story points.
    pub issues: bool,
    /// Days shown by Cumulative flow and Created vs resolved (`0` = 30).
    pub days: usize,
    /// "Show data": the values as a table under each chart.
    pub show_data: bool,
    cache: BTreeMap<String, Result<Value, String>>,
    cache_for: Option<(u64, String)>,
}

impl ReportsUi {
    pub fn days(&self) -> usize {
        if self.days == 0 { 30 } else { self.days }
    }
}

/// Runs a report query once per workspace revision and user.
pub(crate) fn query(app: &mut TrackApp, id: &str, params: Value) -> Result<Value, String> {
    let stamp = (app.session.revision(), app.session.user.clone());
    if app.reports.cache_for.as_ref() != Some(&stamp) {
        app.reports.cache.clear();
        app.reports.cache_for = Some(stamp);
    }
    let key = format!("{id} {params}");
    if let Some(r) = app.reports.cache.get(&key) {
        return r.clone();
    }
    let r = app.session.run(id, &params);
    if app.reports.cache.len() > 64 {
        app.reports.cache.clear();
    }
    app.reports.cache.insert(key, r.clone());
    r
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `2026-10-03` → `Oct 3` (anything else unchanged).
pub(crate) fn short_date(d: &str) -> String {
    let mut it = d.splitn(3, '-');
    let (_, m, day) = (it.next(), it.next(), it.next());
    match (
        m.and_then(|m| m.parse::<usize>().ok()).and_then(|m| m.checked_sub(1)).and_then(|m| MONTHS.get(m)),
        day.and_then(|d| d.parse::<u32>().ok()),
    ) {
        (Some(m), Some(day)) => format!("{m} {day}"),
        _ => d.to_string(),
    }
}

fn arr(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn unit_word(issues: bool, n: f64) -> &'static str {
    match (issues, (n - 1.0).abs() < 1e-9) {
        (true, true) => "issue",
        (true, false) => "issues",
        (false, true) => "point",
        (false, false) => "points",
    }
}

// ---- chart builders (shared with the tests) -----------------------------------------------

/// The burndown chart of a `report.burndown` result: Remaining and Scope per day, the Ideal line
/// over the whole sprint.
pub(crate) fn burndown_chart(t: &Tokens, r: &Value, compact: bool) -> Chart {
    let days = arr(r, "days");
    let ideal = arr(r, "ideal");
    // On a time axis: the start, the end of each day so far, then the rest of the sprint
    // (ideal only). The ideal is linear in time, so it draws straight.
    struct Pt {
        label: String,
        short: String,
        at: f64,
        remaining: Option<f64>,
        scope: Option<f64>,
        ideal: Option<f64>,
    }
    let mut pts: Vec<Pt> = Vec::new();
    if let Some(first) = ideal.first() {
        let v = f(r, "committed");
        pts.push(Pt {
            label: format!("Sprint start, {}", short_date(s(first, "date"))),
            short: short_date(s(first, "date")),
            at: 0.0,
            remaining: v,
            scope: v,
            ideal: f(first, "remaining"),
        });
    }
    for d in &days {
        pts.push(Pt {
            label: short_date(s(d, "date")),
            short: short_date(s(d, "date")),
            at: f(d, "offset").unwrap_or(0.0),
            remaining: f(d, "remaining"),
            scope: f(d, "scope"),
            ideal: f(d, "ideal"),
        });
    }
    let last = pts.last().map_or(0.0, |p| p.at);
    for d in ideal.iter().skip(1) {
        let at = f(d, "offset").unwrap_or(0.0);
        if at > last + 1e-6 {
            pts.push(Pt {
                label: short_date(s(d, "date")),
                short: short_date(s(d, "date")),
                at,
                remaining: None,
                scope: None,
                ideal: f(d, "remaining"),
            });
        }
    }
    let unit = if s(r, "unit") == "issues" { "issues" } else { "points" };
    let series = vec![
        Series { name: "Remaining".into(), color: charts::series_color(t, 0), mark: Mark::Line, values: pts.iter().map(|p| p.remaining).collect() },
        Series { name: "Scope".into(), color: charts::series_color(t, 1), mark: Mark::Line, values: pts.iter().map(|p| p.scope).collect() },
        Series { name: "Ideal".into(), color: charts::reference_color(t), mark: Mark::Reference, values: pts.iter().map(|p| p.ideal).collect() },
    ];
    Chart {
        label: format!("Burndown chart of {}: remaining {unit} per day, scope and the ideal line", s(&r["sprint"], "name")),
        x_title: "Date".into(),
        x: pts.iter().map(|p| p.label.clone()).collect(),
        x_short: pts.iter().map(|p| p.short.clone()).collect(),
        x_at: pts.iter().map(|p| p.at).collect(),
        series,
        height: if compact { 130.0 } else { 300.0 },
        compact,
    }
}

pub(crate) fn velocity_chart(t: &Tokens, r: &Value) -> Chart {
    let rows = arr(r, "sprints");
    let names: Vec<String> = rows.iter().map(|x| s(x, "name").to_string()).collect();
    Chart {
        label: format!("Velocity chart: {} committed and completed per sprint", if s(r, "unit") == "issues" { "issues" } else { "points" }),
        x_title: "Sprint".into(),
        x_short: names
            .iter()
            .map(|n| if n.chars().count() > 14 { format!("{}...", n.chars().take(12).collect::<String>()) } else { n.clone() })
            .collect(),
        x: names,
        x_at: vec![],
        series: vec![
            Series {
                name: "Committed".into(),
                color: charts::reference_color(t),
                mark: Mark::Bar,
                values: rows.iter().map(|x| f(x, "committed")).collect(),
            },
            Series {
                name: "Completed".into(),
                color: charts::series_color(t, 0),
                mark: Mark::Bar,
                values: rows.iter().map(|x| f(x, "completedValue")).collect(),
            },
        ],
        height: 300.0,
        compact: false,
    }
}

pub(crate) fn cfd_chart(t: &Tokens, r: &Value) -> Chart {
    let rows = arr(r, "days");
    let dates: Vec<String> = rows.iter().map(|x| s(x, "date").to_string()).collect();
    let band = |name: &str, k: &str, slot| Series {
        name: name.into(),
        color: charts::series_color(t, slot),
        mark: Mark::Area,
        values: rows.iter().map(|x| f(x, k)).collect(),
    };
    Chart {
        label: "Cumulative flow diagram: issues per status category at the end of each day".into(),
        x_title: "Date".into(),
        x_short: dates.iter().map(|d| short_date(d)).collect(),
        x: dates,
        x_at: vec![],
        // Done at the bottom: finished work accumulates, the open bands ride on top.
        series: vec![band("Done", "done", 2), band("In Progress", "inProgress", 1), band("To Do", "toDo", 0)],
        height: 300.0,
        compact: false,
    }
}

pub(crate) fn created_resolved_chart(t: &Tokens, r: &Value, compact: bool) -> Chart {
    let rows = arr(r, "days");
    let dates: Vec<String> = rows.iter().map(|x| s(x, "date").to_string()).collect();
    // Daily counts are discrete: columns, side by side.
    let line = |name: &str, k: &str, slot| Series {
        name: name.into(),
        color: charts::series_color(t, slot),
        mark: Mark::Bar,
        values: rows.iter().map(|x| f(x, k)).collect(),
    };
    Chart {
        label: format!("Created vs resolved chart: issues created and resolved per day, last {} days", rows.len()),
        x_title: "Date".into(),
        x_short: dates.iter().map(|d| short_date(d)).collect(),
        x: dates,
        x_at: vec![],
        series: vec![line("Created", "created", 0), line("Resolved", "resolved", 1)],
        height: if compact { 130.0 } else { 300.0 },
        compact,
    }
}

// ---- the view -------------------------------------------------------------------------------

fn show_chart(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, id: &str, c: &Chart) {
    card(ui, t, "", |ui| {
        charts::legend(ui, t, c);
        ui.add_space(4.0);
        charts::chart(ui, t, c);
    });
    ui.add_space(8.0);
    ui.checkbox(&mut app.reports.show_data, "Show data");
    if app.reports.show_data {
        ui.add_space(4.0);
        card(ui, t, "", |ui| charts::data_table(ui, t, id, c));
    }
}

fn unit_picker(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    field_row(ui, t, "Unit", |ui, label| {
        let text = if app.reports.issues { "Issue count" } else { "Story points" };
        ComboBox::from_id_salt("report-unit")
            .selected_text(text)
            .width(140.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.reports.issues, false, "Story points");
                ui.selectable_value(&mut app.reports.issues, true, "Issue count");
            })
            .response
            .labelled_by(label);
    });
}

fn range_picker(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    field_row(ui, t, "Range", |ui, label| {
        let mut d = app.reports.days();
        ComboBox::from_id_salt("report-range")
            .selected_text(format!("Last {d} days"))
            .width(140.0)
            .show_ui(ui, |ui| {
                for r in RANGES {
                    ui.selectable_value(&mut d, r, format!("Last {r} days"));
                }
            })
            .response
            .labelled_by(label);
        app.reports.days = d;
    });
}

fn error_note(ui: &mut Ui, t: &Tokens, e: &str) {
    card(ui, t, "", |ui| {
        empty_state(ui, t, "This report couldn't be built", e, None);
    });
}

pub(crate) fn view(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    let Some(name) = app.session.ws.projects.get(project).map(|p| p.name.clone()) else {
        empty_state(ui, t, "No such project", "It may have been deleted, or you can't see it.", None);
        return;
    };
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Reports").strong());
        ui.label(RichText::new(format!("{name} \u{b7} {project}")).color(t.muted));
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for tab in Tab::ALL {
            if ui.add(egui::Button::selectable(app.reports.tab == tab, tab.label())).clicked() {
                app.reports.tab = tab;
            }
        }
    });
    ui.add_space(10.0);
    egui::ScrollArea::vertical().id_salt("reports-scroll").auto_shrink([false, false]).show(ui, |ui| match app.reports.tab {
        Tab::Burndown => burndown(app, ui, t, project),
        Tab::Velocity => velocity(app, ui, t, project),
        Tab::Cfd => cfd(app, ui, t, project),
        Tab::CreatedResolved => created_resolved(app, ui, t, project),
    });
}

fn burndown(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    // Started sprints: the active one first, then closed ones, newest first.
    let mut sprints: Vec<(Id, String, SprintState)> = app
        .session
        .ws
        .project_sprints(project)
        .into_iter()
        .filter(|s| s.state != SprintState::Future && s.start.is_some())
        .map(|s| (s.id, s.name.clone(), s.state))
        .collect();
    sprints.sort_by_key(|(id, _, st)| (*st != SprintState::Active, std::cmp::Reverse(*id)));
    if sprints.is_empty() {
        let mut open = false;
        card(ui, t, "", |ui| {
            open =
                empty_state(ui, t, "No sprints to chart yet", "Start a sprint from the backlog; its burndown shows up here.", Some("Open backlog"));
        });
        if open {
            app.push(Action::Nav(View::Backlog(project.clone())));
        }
        return;
    }
    let current = app.reports.sprint.filter(|id| sprints.iter().any(|s| s.0 == *id)).or(sprints.first().map(|s| s.0));
    let Some(current) = current else { return };
    egui::Grid::new("burndown-controls").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        field_row(ui, t, "Sprint", |ui, label| {
            let mut pick = current;
            let text = sprints.iter().find(|s| s.0 == current).map(|s| s.1.clone()).unwrap_or_default();
            ComboBox::from_id_salt("report-sprint")
                .selected_text(text)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (id, n, st) in &sprints {
                        let n = if *st == SprintState::Active { format!("{n} (active)") } else { n.clone() };
                        ui.selectable_value(&mut pick, *id, n);
                    }
                })
                .response
                .labelled_by(label);
            if pick != current {
                app.reports.sprint = Some(pick);
            }
        });
        unit_picker(app, ui, t);
    });
    ui.add_space(8.0);
    let unit = if app.reports.issues { "issues" } else { "points" };
    let r = match query(app, "report.burndown", json!({"sprint": current.0, "unit": unit})) {
        Ok(r) => r,
        Err(e) => return error_note(ui, t, &e),
    };
    let (rem, scope, committed) = (f(&r, "remaining").unwrap_or(0.0), f(&r, "scope").unwrap_or(0.0), f(&r, "committed").unwrap_or(0.0));
    let issues = app.reports.issues;
    let state = if r.get("completed").is_some() { "completed".to_string() } else { "in progress".to_string() };
    ui.label(
        RichText::new(format!(
            "{} of {} {} remaining \u{b7} committed {} at the start \u{b7} sprint {state}",
            charts::num(rem),
            charts::num(scope),
            unit_word(issues, scope),
            charts::num(committed)
        ))
        .color(t.text),
    );
    ui.add_space(6.0);
    let c = burndown_chart(t, &r, false);
    show_chart(app, ui, t, "burndown", &c);
    let events = arr(&r, "events");
    ui.add_space(10.0);
    card(ui, t, "Scope changes", |ui| {
        if events.is_empty() {
            ui.label(RichText::new("Nothing was added, removed or re-estimated after the sprint started.").color(t.muted));
            return;
        }
        egui::Grid::new("burndown-events").striped(true).spacing([18.0, 4.0]).show(ui, |ui| {
            for h in ["Date", "Issue", "Change", "Remaining"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for e in events.iter().take(200) {
                ui.label(short_date(s(e, "at").get(..10).unwrap_or_default()));
                let key = s(e, "key").to_string();
                if ui.link(RichText::new(&key).monospace()).clicked()
                    && let Ok(k) = trackcraft_core::IssueKey::parse(&key)
                {
                    app.push(Action::Nav(View::Issue(k)));
                }
                let what = match s(e, "kind") {
                    "added" => "Added to the sprint".to_string(),
                    "removed" => "Removed from the sprint".to_string(),
                    "completed" => "Done".to_string(),
                    "reopened" => "Reopened".to_string(),
                    "estimateChanged" => {
                        format!("Estimate {} to {}", charts::num(f(e, "from").unwrap_or(0.0)), charts::num(f(e, "to").unwrap_or(0.0)))
                    }
                    other => other.to_string(),
                };
                ui.label(what);
                let d = f(e, "remainingDelta").unwrap_or(0.0);
                ui.label(RichText::new(if d > 0.0 { format!("+{}", charts::num(d)) } else { charts::num(d) }).monospace());
                ui.end_row();
            }
        });
    });
}

fn velocity(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    egui::Grid::new("velocity-controls").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| unit_picker(app, ui, t));
    ui.add_space(8.0);
    let unit = if app.reports.issues { "issues" } else { "points" };
    let r = match query(app, "report.velocity", json!({"project": project.as_str(), "unit": unit})) {
        Ok(r) => r,
        Err(e) => return error_note(ui, t, &e),
    };
    let rows = arr(&r, "sprints");
    if rows.is_empty() {
        card(ui, t, "", |ui| {
            empty_state(
                ui,
                t,
                "No completed sprints yet",
                "Velocity compares what each sprint committed to with what it completed. Complete a sprint to see it here.",
                None,
            );
        });
        return;
    }
    let avg = f(&r, "average").unwrap_or(0.0);
    ui.label(
        RichText::new(format!(
            "Average completed: {} {} over the last {}",
            charts::num(avg),
            unit_word(app.reports.issues, avg),
            if rows.len() == 1 { "sprint".to_string() } else { format!("{} sprints", rows.len()) }
        ))
        .color(t.text),
    );
    ui.add_space(6.0);
    let c = velocity_chart(t, &r);
    show_chart(app, ui, t, "velocity", &c);
}

fn cfd(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    egui::Grid::new("cfd-controls").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| range_picker(app, ui, t));
    ui.add_space(8.0);
    let days = app.reports.days();
    let r = match query(app, "report.cfd", json!({"project": project.as_str(), "days": days})) {
        Ok(r) => r,
        Err(e) => return error_note(ui, t, &e),
    };
    let rows = arr(&r, "days");
    let any = rows.iter().any(|x| ["toDo", "inProgress", "done"].iter().any(|k| f(x, k).unwrap_or(0.0) > 0.0));
    if !any {
        card(ui, t, "", |ui| {
            empty_state(ui, t, "No issues in this range", "Cumulative flow shows how many issues sit in each status category, day by day.", None);
        });
        return;
    }
    let c = cfd_chart(t, &r);
    show_chart(app, ui, t, "cfd", &c);
}

fn created_resolved(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, project: &ProjectKey) {
    egui::Grid::new("cvr-controls").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| range_picker(app, ui, t));
    ui.add_space(8.0);
    let days = app.reports.days();
    let r = match query(app, "report.createdResolved", json!({"project": project.as_str(), "days": days})) {
        Ok(r) => r,
        Err(e) => return error_note(ui, t, &e),
    };
    let (c_n, r_n) = (f(&r, "created").unwrap_or(0.0), f(&r, "resolved").unwrap_or(0.0));
    ui.label(RichText::new(format!("{} created \u{b7} {} resolved in the last {days} days", charts::num(c_n), charts::num(r_n))).color(t.text));
    ui.add_space(6.0);
    let c = created_resolved_chart(t, &r, false);
    show_chart(app, ui, t, "created-resolved", &c);
}

// ---- Home gadgets ---------------------------------------------------------------------------

/// "Active sprint": a mini burndown of the first project (by name) with an active sprint.
pub(crate) fn active_sprint_gadget(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let mut active: Vec<(String, ProjectKey, Id)> = app
        .session
        .ws
        .sprints
        .values()
        .filter(|s| s.state == SprintState::Active)
        .filter_map(|s| app.session.ws.projects.get(&s.project).map(|p| (p.name.to_lowercase(), p.key.clone(), s.id)))
        .collect();
    active.sort();
    card(ui, t, "Active sprint", |ui| {
        let Some((_, project, id)) = active.first().cloned() else {
            empty_state(ui, t, "", "No sprint is running. Start one from a project's backlog.", None);
            return;
        };
        let r = match query(app, "report.burndown", json!({"sprint": id.0})) {
            Ok(r) => r,
            Err(e) => {
                ui.label(RichText::new(e).color(t.muted));
                return;
            }
        };
        let (rem, scope) = (f(&r, "remaining").unwrap_or(0.0), f(&r, "scope").unwrap_or(0.0));
        ui.horizontal(|ui| {
            ui.label(RichText::new(s(&r["sprint"], "name")).strong());
            ui.label(RichText::new(format!("{} of {} points remaining", charts::num(rem), charts::num(scope))).color(t.muted).size(12.0));
        });
        let c = burndown_chart(t, &r, true);
        charts::legend(ui, t, &c);
        charts::chart(ui, t, &c);
        if ui.link("Open reports").clicked() {
            app.reports.tab = Tab::Burndown;
            app.reports.sprint = Some(id);
            app.push(Action::Nav(View::Reports(project)));
        }
    });
}

/// "Created vs resolved (14 days)" across every issue the user can see.
pub(crate) fn created_resolved_gadget(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    card(ui, t, "Created vs resolved (14 days)", |ui| {
        let r = match query(app, "report.createdResolved", json!({"jql": "", "days": 14})) {
            Ok(r) => r,
            Err(e) => {
                ui.label(RichText::new(e).color(t.muted));
                return;
            }
        };
        let (c_n, r_n) = (f(&r, "created").unwrap_or(0.0), f(&r, "resolved").unwrap_or(0.0));
        ui.label(RichText::new(format!("{} created \u{b7} {} resolved", charts::num(c_n), charts::num(r_n))).color(t.muted).size(12.0));
        let c = created_resolved_chart(t, &r, true);
        charts::legend(ui, t, &c);
        charts::chart(ui, t, &c);
    });
}
