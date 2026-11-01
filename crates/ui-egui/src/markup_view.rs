//! Renders a rich-text [`Doc`] read-only.

use egui::text::LayoutJob;
use egui::{Color32, FontId, Stroke, TextFormat, Ui};
use trackcraft_markup::{Block, Doc, Inline, Mark};

use crate::theme::Tokens;

pub fn show(ui: &mut Ui, t: &Tokens, doc: &Doc) {
    if doc.is_empty() {
        ui.label(egui::RichText::new("Nothing here yet.").italics().color(t.muted));
        return;
    }
    for b in &doc.blocks {
        block(ui, t, b, 0);
    }
}

fn block(ui: &mut Ui, t: &Tokens, b: &Block, depth: usize) {
    if depth > 32 {
        return;
    }
    match b {
        Block::Paragraph(inl) => {
            ui.label(job(t, inl, 14.0));
            ui.add_space(4.0);
        }
        Block::Heading { level, content } => {
            let size = match level {
                1 => 24.0,
                2 => 20.0,
                3 => 17.0,
                _ => 15.0,
            };
            ui.add_space(6.0);
            let mut j = job(t, content, size);
            for s in &mut j.sections {
                s.format.color = t.text;
            }
            ui.label(j);
            ui.add_space(2.0);
        }
        Block::BulletList(items) | Block::OrderedList(items) => {
            let ordered = matches!(b, Block::OrderedList(_));
            for (n, item) in items.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.add_space(6.0 + 14.0 * depth as f32);
                    let marker = if ordered { format!("{}.", n.saturating_add(1)) } else { "•".to_string() };
                    ui.label(egui::RichText::new(marker).color(t.muted));
                    ui.vertical(|ui| {
                        for b in item {
                            block(ui, t, b, depth + 1);
                        }
                    });
                });
            }
            ui.add_space(2.0);
        }
        Block::CodeBlock { text, .. } => {
            egui::Frame::new().fill(t.code_bg).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(text).monospace().size(13.0));
            });
            ui.add_space(4.0);
        }
        Block::Quote(bs) => {
            egui::Frame::new().stroke(Stroke::NONE).inner_margin(egui::Margin { left: 12, right: 0, top: 2, bottom: 2 }).show(ui, |ui| {
                let top = ui.cursor().top();
                for b in bs {
                    block(ui, t, b, depth + 1);
                }
                let bottom = ui.min_rect().bottom();
                let x = ui.min_rect().left() - 8.0;
                ui.painter().line_segment([egui::pos2(x, top), egui::pos2(x, bottom)], Stroke::new(3.0, t.border));
            });
        }
        Block::Rule => {
            ui.separator();
        }
        Block::Unknown(v) => {
            let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("content");
            ui.label(egui::RichText::new(format!("[{ty} — not shown yet]")).italics().color(t.muted));
        }
    }
}

fn job(t: &Tokens, inl: &[Inline], size: f32) -> LayoutJob {
    let mut j = LayoutJob::default();
    j.wrap.max_width = f32::INFINITY;
    for i in inl {
        match i {
            Inline::Text { text, marks } => {
                let mut f = TextFormat { font_id: FontId::proportional(size), color: t.text, ..Default::default() };
                for m in marks {
                    match m {
                        Mark::Strong => f.color = if t.dark { Color32::WHITE } else { Color32::BLACK },
                        Mark::Em => f.italics = true,
                        Mark::Code => {
                            f.font_id = FontId::monospace(size - 1.0);
                            f.background = t.code_bg;
                        }
                        Mark::Strike => f.strikethrough = Stroke::new(1.0, t.muted),
                        Mark::Underline => f.underline = Stroke::new(1.0, t.text),
                        Mark::Link(_) => {
                            f.color = t.accent;
                            f.underline = Stroke::new(1.0, t.accent);
                        }
                    }
                }
                j.append(text, 0.0, f);
            }
            Inline::HardBreak => j.append("\n", 0.0, TextFormat { font_id: FontId::proportional(size), ..Default::default() }),
            Inline::Mention { text, .. } => j.append(
                text,
                0.0,
                TextFormat { font_id: FontId::proportional(size), color: t.accent, background: t.accent_soft, ..Default::default() },
            ),
            Inline::Unknown(_) => {}
        }
    }
    j
}
