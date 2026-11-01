//! The window frame: top bar, sidebar (projects, filters, spaces with page trees) and home.

use egui::{RichText, Ui};
use serde_json::json;
use trackcraft_core::Id;

use crate::theme::Tokens;
use crate::widgets::{card, empty_state, link_truncated};
use crate::{Action, ComboBox, FileRequest, Then, TrackApp, View, icons};

pub(crate) fn top_bar(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::top("top")
        .exact_size(52.0)
        .frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(14, 8)).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if icons::app_mark(ui, t, 24.0).interact(egui::Sense::click()).on_hover_text("Home").clicked() {
                    app.push(Action::Nav(View::Home));
                }
                ui.label(RichText::new(&app.session.ws.name).strong().size(15.0));
                ui.add_space(8.0);
                ui.menu_button("File", |ui| {
                    for (label, sc, req) in [
                        ("New workspace", "", FileRequest::New),
                        ("Open…", "Mod+O", FileRequest::Open),
                        ("Save", "Mod+S", FileRequest::Save),
                        ("Save As…", "Mod+Shift+S", FileRequest::SaveAs),
                    ] {
                        if ui.add(egui::Button::new(label).shortcut_text(crate::key(sc))).clicked() {
                            app.push(Action::File(req));
                            ui.close();
                        }
                    }
                });
                ui.add_space(8.0);
                let w = (ui.available_width() - 420.0).clamp(160.0, 520.0);
                let mut q = app.jql.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut q).desired_width(w).hint_text("Search issues with JQL  ( / )").font(egui::TextStyle::Monospace),
                );
                if app.take_focus_search() {
                    resp.request_focus();
                }
                if resp.changed() {
                    app.jql = q;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    app.push(Action::Search(app.jql.clone()));
                }
                let create = egui::Button::new(RichText::new("+ Create").color(t.on_accent).strong()).fill(t.accent);
                if ui.add(create).on_hover_text("Create an issue (C)").clicked() {
                    app.push(Action::OpenCreate(current_project(app)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let me = app.session.user.clone();
                    let display = app.session.ws.users.get(&me).map(|u| u.display_name.clone()).unwrap_or_else(|| me.clone());
                    // Right-to-left: the combo is drawn before its label, and linked to it after.
                    let acting = ComboBox::from_id_salt("acting-user")
                        .selected_text(display.clone())
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            let users: Vec<(String, String)> =
                                app.session.ws.users.values().map(|u| (u.id.clone(), u.display_name.clone())).collect();
                            for (id, name) in users {
                                if ui.selectable_label(id == me, &name).clicked() {
                                    app.push(Action::Run {
                                        id: "user.switch",
                                        params: json!({"id": id}),
                                        then: Then::Toast(format!("Acting as {name}")),
                                    });
                                }
                            }
                        })
                        .response;
                    icons::avatar(ui, t, &me, &display, 26.0);
                    let label = ui.label(RichText::new("Acting as").color(t.muted).size(12.0));
                    let _ = acting.labelled_by(label.id);
                    let theme = if t.dark { "Light" } else { "Dark" };
                    if ui.small_button(theme).on_hover_text("Switch the interface theme").clicked() {
                        app.push(Action::ToggleTheme);
                    }
                    let redo = ui
                        .add_enabled(app.session.can_redo(), egui::Button::new("Redo").small())
                        .on_hover_text(format!("Redo ({})", crate::key("Mod+Shift+Z")));
                    if redo.clicked() {
                        app.push(Action::Run { id: "edit.redo", params: json!({}), then: Then::Nothing });
                    }
                    let undo = ui
                        .add_enabled(app.session.can_undo(), egui::Button::new("Undo").small())
                        .on_hover_text(format!("Undo ({})", crate::key("Mod+Z")));
                    if undo.clicked() {
                        app.push(Action::Run { id: "edit.undo", params: json!({}), then: Then::Nothing });
                    }
                    if ui.small_button(crate::key("Mod+K")).on_hover_text("Command palette").clicked() {
                        app.push(Action::TogglePalette);
                    }
                });
            });
        });
}

fn current_project(app: &TrackApp) -> Option<String> {
    match &app.view {
        View::Board(p) | View::Backlog(p) | View::SprintBoard(p, _) => Some(p.to_string()),
        View::Issue(k) => Some(k.project.to_string()),
        _ => None,
    }
}

fn nav_item(ui: &mut Ui, t: &Tokens, selected: bool, indent: f32, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let text = if selected { RichText::new(label).color(t.accent).strong() } else { RichText::new(label) };
        ui.add(egui::Button::selectable(selected, text).frame_when_inactive(false))
    })
    .inner
}

fn section(ui: &mut Ui, t: &Tokens, label: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(label.to_uppercase()).size(11.0).color(t.muted).strong());
    ui.add_space(2.0);
}

pub(crate) fn sidebar(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(250.0)
        .frame(egui::Frame::new().fill(t.sidebar).inner_margin(egui::Margin::symmetric(12, 6)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if nav_item(ui, t, app.view == View::Home, 0.0, "Home").clicked() {
                    app.push(Action::Nav(View::Home));
                }
                section(ui, t, "Filters");
                let me = app.session.user.clone();
                for (label, q) in [
                    ("My open issues", "assignee = currentUser() AND statusCategory != Done ORDER BY priority".to_string()),
                    ("Reported by me", "reporter = currentUser() ORDER BY created DESC".to_string()),
                    ("All issues", "ORDER BY updated DESC".to_string()),
                    ("Recently resolved", "resolved >= -14d ORDER BY resolved DESC".to_string()),
                ] {
                    let sel = app.view == View::Issues && app.jql == q;
                    if nav_item(ui, t, sel, 0.0, label).clicked() {
                        app.push(Action::Search(q));
                    }
                }
                let _ = me;

                section(ui, t, "Projects");
                let projects: Vec<(String, String)> = app.session.ws.projects.values().map(|p| (p.key.to_string(), p.name.clone())).collect();
                for (key, name) in projects {
                    ui.label(RichText::new(format!("{name}  ·  {key}")).strong().size(13.0));
                    let pk = trackcraft_core::ProjectKey::parse(&key).ok();
                    let on_board = pk.as_ref().is_some_and(|k| matches!(&app.view, View::Board(p) | View::SprintBoard(p, _) if p == k));
                    if nav_item(ui, t, on_board, 10.0, "Board").clicked()
                        && let Some(k) = pk.clone()
                    {
                        // UI-12: the active sprint's board when one is running.
                        let to = match app.session.ws.active_sprint(&k) {
                            Some(sp) => View::SprintBoard(k, sp.id),
                            None => View::Board(k),
                        };
                        app.push(Action::Nav(to));
                    }
                    // UI-11
                    let on_backlog = pk.as_ref().is_some_and(|k| app.view == View::Backlog(k.clone()));
                    if nav_item(ui, t, on_backlog, 10.0, "Backlog").clicked()
                        && let Some(k) = pk.clone()
                    {
                        app.push(Action::Nav(View::Backlog(k)));
                    }
                    let q = format!("project = {key} ORDER BY created DESC");
                    if nav_item(ui, t, app.view == View::Issues && app.jql == q, 10.0, "Issues").clicked() {
                        app.push(Action::Search(q));
                    }
                }

                section(ui, t, "Spaces");
                let spaces: Vec<(String, String, Id)> =
                    app.session.ws.spaces.values().map(|s| (s.key.to_string(), s.name.clone(), s.homepage)).collect();
                for (key, name, home) in spaces {
                    ui.horizontal(|ui| {
                        let sel = app.view == View::Page(home);
                        let text = if sel { RichText::new(&name).color(t.accent).strong() } else { RichText::new(&name).strong() };
                        if ui.add(egui::Button::selectable(sel, text).frame_when_inactive(false)).clicked() {
                            app.push(Action::Nav(View::Page(home)));
                        }
                        if ui.small_button("+").on_hover_text("Create a page in this space").clicked() {
                            app.push(Action::OpenNewPage { space: key.clone(), parent: None });
                        }
                    });
                    tree(app, ui, t, home, 0);
                }
            });
        });
}

fn tree(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, parent: Id, depth: usize) {
    if depth > 24 {
        return;
    }
    let children: Vec<(Id, String, bool)> =
        app.session.ws.children(parent).iter().map(|c| (c.id, c.title.clone(), !app.session.ws.children(c.id).is_empty())).collect();
    for (id, title, has_kids) in children {
        let open = app.expanded.contains(&id);
        ui.horizontal(|ui| {
            ui.add_space(6.0 + depth as f32 * 12.0);
            if has_kids {
                if icons::chevron(ui, t, open, 14.0).clicked() {
                    if open {
                        app.expanded.remove(&id);
                    } else {
                        app.expanded.insert(id);
                    }
                }
            } else {
                ui.add_space(14.0 + ui.spacing().item_spacing.x);
            }
            let sel = matches!(app.view, View::Page(p) | View::PageHistory(p) if p == id);
            let text = if sel { RichText::new(&title).color(t.accent) } else { RichText::new(&title) };
            if ui.add(egui::Button::selectable(sel, text).frame_when_inactive(false).truncate()).clicked() {
                app.push(Action::Nav(View::Page(id)));
            }
        });
        if open && has_kids {
            tree(app, ui, t, id, depth + 1);
        }
    }
}

pub(crate) fn home(app: &mut TrackApp, ui: &mut Ui, t: &Tokens) {
    let ws = &app.session.ws;
    let me = app.session.user.clone();
    let name = ws.users.get(&me).map(|u| u.display_name.clone()).unwrap_or(me.clone());
    let first = name.split_whitespace().next().unwrap_or(&name);
    ui.add(egui::Label::new(RichText::new(format!("Welcome back, {first}")).size(24.0).strong()).truncate());
    ui.label(
        RichText::new(format!("{} projects · {} issues · {} spaces · {} pages", ws.projects.len(), ws.issues.len(), ws.spaces.len(), ws.pages.len()))
            .color(t.muted),
    );
    ui.add_space(16.0);
    // UI-6: an empty workspace says what to do first.
    if ws.projects.is_empty() && ws.spaces.is_empty() {
        let mut open = false;
        card(ui, t, "", |ui| {
            let hint = format!("Run Create Project or Create Space from the command palette ({}).", crate::key("Mod+K"));
            open = empty_state(ui, t, "This workspace is empty", &hint, Some("Open command palette"));
        });
        if open {
            app.push(Action::TogglePalette);
        }
        return;
    }
    let now = app.session.now();
    let mut mine: Vec<_> = ws
        .issues
        .values()
        .filter(|i| i.assignee.as_deref() == Some(me.as_str()) && ws.status_category(i) != trackcraft_model::StatusCategory::Done)
        .collect();
    // Only the first ten are shown: a partial sort keeps 10 000 assigned issues cheap.
    let rank = |i: &&trackcraft_model::Issue| (trackcraft_model::priority_rank(&i.priority).unwrap_or(9), std::cmp::Reverse(i.updated));
    if mine.len() > 10 {
        mine.select_nth_unstable_by_key(9, rank);
        mine.truncate(10);
    }
    mine.sort_by_key(rank);
    let mut pages: Vec<_> = ws.pages.values().filter(|p| p.parent.is_some()).collect();
    pages.sort_by_key(|p| std::cmp::Reverse(p.updated));
    let mut actions = Vec::new();
    ui.columns(2, |cols| {
        if let Some(c) = cols.get_mut(0) {
            card(c, t, "Assigned to me", |ui| {
                if mine.is_empty() {
                    empty_state(ui, t, "", "Nothing assigned to you. Nice.", None);
                }
                for i in &mine {
                    ui.horizontal(|ui| {
                        icons::issue_type(ui, &i.issue_type, 16.0);
                        if ui.link(RichText::new(i.key.to_string()).color(t.muted).monospace()).clicked() || link_truncated(ui, &i.summary).clicked()
                        {
                            actions.push(Action::Nav(View::Issue(i.key.clone())));
                        }
                    });
                }
            });
        }
        if let Some(c) = cols.get_mut(1) {
            card(c, t, "Recently updated pages", |ui| {
                if pages.is_empty() {
                    empty_state(ui, t, "", "No pages yet. Pages you or your team write show up here.", None);
                }
                for p in pages.iter().take(10) {
                    ui.horizontal(|ui| {
                        icons::page(ui, t, 16.0);
                        // Right to left: the meta text keeps its place, the title truncates.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("{} · {}", p.space, crate::ago(now, p.updated))).color(t.muted).size(12.0));
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                if link_truncated(ui, &p.title).clicked() {
                                    actions.push(Action::Nav(View::Page(p.id)));
                                }
                            });
                        });
                    });
                }
            });
        }
    });
    for a in actions {
        app.push(a);
    }
}
