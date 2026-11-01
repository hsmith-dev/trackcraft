//! Version diff (UI-28): two versions of a page compared block by block (`page.diff`), with
//! word-level marks inside changed blocks. Marks are never colour only: inserted text is
//! underlined and labelled "Added", deleted text struck through and labelled "Removed".

use egui::text::LayoutJob;
use egui::{Color32, FontId, Rect, RichText, Stroke, TextFormat, Ui, vec2};
use serde_json::{Value, json};
use trackcraft_core::Id;

use crate::theme::Tokens;
use crate::{Action, ComboBox, Then, TrackApp, View};

/// `over` laid over `base` at `a` (0..1): the diff tints, computed from the theme tokens.
pub(crate) fn mix(base: Color32, over: Color32, a: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * a).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(base.r(), over.r()), l(base.g(), over.g()), l(base.b(), over.b()))
}

/// The diff's own colours, derived from the theme.
struct Tints {
    ins_bg: Color32,
    ins_word: Color32,
    del_bg: Color32,
    del_word: Color32,
}

impl Tints {
    fn of(t: &Tokens) -> Tints {
        let (soft, strong) = if t.dark { (0.16, 0.32) } else { (0.10, 0.24) };
        Tints {
            ins_bg: mix(t.surface, t.ok, soft),
            ins_word: mix(t.surface, t.ok, strong),
            del_bg: mix(t.surface, t.danger, soft),
            del_word: mix(t.surface, t.danger, strong),
        }
    }
}

/// The diff for `(page, from, to)`, cached until the workspace changes.
#[derive(Debug, Default)]
pub(crate) struct DiffCache {
    key: Option<(Id, u32, u32, u64)>,
    value: Option<Result<Value, String>>,
}

fn load(app: &mut TrackApp, id: Id, from: u32, to: u32) -> Result<Value, String> {
    let key = (id, from, to, app.session.revision());
    if app.wiki.diff.key != Some(key) {
        let v = app.session.run("page.diff", &json!({"id": id.0, "from": from, "to": to}));
        app.wiki.diff = DiffCache { key: Some(key), value: Some(v) };
    }
    app.wiki.diff.value.clone().unwrap_or_else(|| Err("no diff".into()))
}

fn word_job(t: &Tokens, tints: &Tints, hunks: &[Value], size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for h in hunks.iter().take(20_000) {
        let text = h["text"].as_str().unwrap_or_default();
        let mut f = TextFormat { font_id: FontId::proportional(size), color: t.text, ..Default::default() };
        match h["kind"].as_str().unwrap_or_default() {
            "insert" => {
                f.background = tints.ins_word;
                f.underline = Stroke::new(1.0, t.ok);
            }
            "delete" => {
                f.background = tints.del_word;
                f.strikethrough = Stroke::new(1.0, t.danger);
                f.color = t.muted;
            }
            _ => {}
        }
        job.append(text, 0.0, f);
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

/// A block of the diff: a tinted frame with a left bar and a text tag (not colour only).
fn hunk_frame(ui: &mut Ui, fill: Color32, bar: Color32, tag: &str, body: impl FnOnce(&mut Ui)) {
    let r = egui::Frame::new().fill(fill).corner_radius(4).inner_margin(egui::Margin { left: 14, right: 10, top: 6, bottom: 6 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if !tag.is_empty() {
            ui.label(RichText::new(tag).size(10.5).strong().color(bar));
        }
        body(ui);
    });
    let rr = r.response.rect;
    ui.painter().rect_filled(Rect::from_min_size(rr.min, vec2(3.0, rr.height())), 1.5, bar);
    ui.add_space(4.0);
}

fn plain(ui: &mut Ui, text: &str, color: Color32, strike: bool) {
    let mut rt = RichText::new(text).size(14.0).color(color);
    if strike {
        rt = rt.strikethrough();
    }
    ui.add(egui::Label::new(rt).wrap());
}

/// Versions to pick from, newest first: (number, "v3 · message").
fn versions(app: &TrackApp, id: Id) -> Vec<(u32, String)> {
    let Ok(p) = app.session.ws.page(id) else { return vec![] };
    let mut out: Vec<(u32, String)> = p.history.iter().map(|v| (v.version, v.message.clone())).collect();
    out.push((p.version, p.message.clone()));
    out.reverse();
    out.into_iter()
        .map(|(n, m)| {
            let m: String = m.chars().take(40).collect();
            (n, if m.is_empty() { format!("v{n}") } else { format!("v{n} · {m}") })
        })
        .collect()
}

fn picker(ui: &mut Ui, salt: &str, label: &str, cur: u32, list: &[(u32, String)]) -> Option<u32> {
    let mut pick = None;
    ui.label(RichText::new(label).size(12.5));
    let shown = list.iter().find(|v| v.0 == cur).map(|v| v.1.clone()).unwrap_or_else(|| format!("v{cur}"));
    ComboBox::from_id_salt(salt).selected_text(shown).width(170.0).show_ui(ui, |ui| {
        for (n, text) in list {
            if ui.selectable_label(*n == cur, text).clicked() {
                pick = Some(*n);
            }
        }
    });
    pick
}

/// The diff view: header with version pickers, title change, then the block hunks.
pub(crate) fn view(app: &mut TrackApp, ui: &mut Ui, t: &Tokens, id: Id, from: u32, to: u32) {
    let Ok(title) = app.session.ws.page(id).map(|p| p.title.clone()) else {
        ui.label("This page no longer exists.");
        return;
    };
    super::breadcrumbs(app, ui, t, id);
    let list = versions(app, id);
    let mut nav = None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Back to history").clicked() {
                nav = Some(View::PageHistory(id));
            }
            if ui.button("Back to page").clicked() {
                nav = Some(View::Page(id));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(format!("Compare versions of “{title}”")).size(22.0).strong()).truncate());
            });
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if let Some(f) = picker(ui, "diff-from", "From", from, &list) {
            nav = Some(View::PageDiff(id, f, to));
        }
        ui.add_space(8.0);
        if let Some(n) = picker(ui, "diff-to", "To", to, &list) {
            nav = Some(View::PageDiff(id, from, n));
        }
        if from != to && ui.small_button("Swap").on_hover_text("Compare the other way round").clicked() {
            nav = Some(View::PageDiff(id, to, from));
        }
        let current = app.session.ws.page(id).map(|p| p.version).unwrap_or(0);
        if from != current && ui.small_button(format!("Restore v{from}")).on_hover_text("Make this older version the newest one").clicked() {
            app.push(Action::Run {
                id: "page.restore",
                params: json!({"id": id.0, "version": from}),
                then: Then::Toast(format!("Restored version {from} as a new version")),
            });
        }
    });
    if let Some(v) = nav {
        app.push(Action::Nav(v));
    }
    ui.add_space(8.0);
    let d = match load(app, id, from, to) {
        Ok(d) => d,
        Err(e) => {
            ui.label(RichText::new(e).color(t.danger));
            return;
        }
    };
    let tints = Tints::of(t);
    let stats = &d["stats"];
    let n = |k: &str| stats[k].as_u64().unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        let chip = |ui: &mut Ui, text: String, fill: Color32, color: Color32| {
            egui::Frame::new().fill(fill).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                ui.label(RichText::new(text).size(12.0).color(color));
            });
        };
        chip(ui, format!("{} added", n("inserted")), tints.ins_bg, t.ok);
        chip(ui, format!("{} removed", n("deleted")), tints.del_bg, t.danger);
        chip(ui, format!("{} changed", n("changed")), t.surface_alt, t.text);
        ui.label(RichText::new(format!("v{from} to v{to}")).color(t.muted).size(12.0));
    });
    if d["coarse"].as_bool().unwrap_or(false) {
        ui.add_space(4.0);
        egui::Frame::new().fill(t.surface_alt).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.label(
                RichText::new("Coarse diff: these versions are too large to compare block by block, so the whole body is shown as replaced.")
                    .color(t.muted)
                    .size(12.5),
            );
        });
    }
    ui.add_space(8.0);
    egui::ScrollArea::vertical().id_salt("page-diff").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(860.0);
        let title_hunks = d["title"]["inline"].as_array().cloned().unwrap_or_default();
        if d["title"]["changed"].as_bool().unwrap_or(false) {
            hunk_frame(ui, t.surface_alt, t.accent, "TITLE CHANGED", |ui| {
                ui.label(word_job(t, &tints, &title_hunks, 18.0));
            });
        } else {
            ui.label(RichText::new(d["to"]["title"].as_str().unwrap_or_default()).size(18.0).strong());
            ui.add_space(4.0);
        }
        let blocks = d["blocks"].as_array().cloned().unwrap_or_default();
        if blocks.iter().all(|b| b["kind"] == "equal") {
            ui.label(RichText::new("The bodies of these versions are the same.").color(t.muted));
        }
        for b in blocks.iter().take(5_000) {
            let text = b["text"].as_str().unwrap_or_default();
            match b["kind"].as_str().unwrap_or_default() {
                "insert" => hunk_frame(ui, tints.ins_bg, t.ok, "ADDED", |ui| plain(ui, text, t.text, false)),
                "delete" => hunk_frame(ui, tints.del_bg, t.danger, "REMOVED", |ui| plain(ui, text, t.muted, true)),
                "change" => {
                    let hunks = b["inline"].as_array().cloned().unwrap_or_default();
                    hunk_frame(ui, t.surface, t.accent, "CHANGED", |ui| {
                        ui.label(word_job(t, &tints, &hunks, 14.0));
                    });
                }
                _ => {
                    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 10, top: 4, bottom: 4 }).show(ui, |ui| {
                        plain(ui, text, t.muted, false);
                    });
                    ui.add_space(4.0);
                }
            }
        }
    });
}
