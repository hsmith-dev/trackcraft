//! The icon set, drawn in code (original work; no icon files, no other product's glyphs).
//!
//! Issue types use TrackCraft's own shapes — Epic: hexagon, Story: page with lines, Task: tick
//! box, Bug: diamond with a dot, Sub-task: nested squares. Priority is a column of bars.

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use crate::theme::{Tokens, hex};

const EPIC: Color32 = hex(0x8E4FB8);
const STORY: Color32 = hex(0x0E8A74);
const TASK: Color32 = hex(0x3A6EA5);
const BUG: Color32 = hex(0xC8442F);
const SUBTASK: Color32 = hex(0x6B7A74);

/// Allocates a square of `size` and returns its rect.
pub fn square(ui: &mut Ui, size: f32) -> (Rect, Response) {
    ui.allocate_exact_size(Vec2::splat(size), Sense::hover())
}

pub fn issue_type(ui: &mut Ui, name: &str, size: f32) -> Response {
    let (r, resp) = square(ui, size);
    paint_issue_type(ui.painter(), r, name);
    resp.on_hover_text(name)
}

pub fn paint_issue_type(p: &Painter, r: Rect, name: &str) {
    let r = r.shrink(r.width() * 0.08);
    let c = r.center();
    let s = r.width() / 2.0;
    let white = Stroke::new((r.width() / 9.0).max(1.2), Color32::WHITE);
    match name.to_ascii_lowercase().as_str() {
        "epic" => {
            let pts: Vec<Pos2> = (0..6)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
                    c + vec2(a.cos(), a.sin()) * s
                })
                .collect();
            p.add(Shape::convex_polygon(pts, EPIC, Stroke::NONE));
            p.circle_filled(c, s * 0.28, Color32::WHITE);
        }
        "story" => {
            p.rect_filled(r, s * 0.3, STORY);
            for k in 0..3 {
                let y = r.top() + r.height() * (0.32 + 0.18 * k as f32);
                let w = if k == 2 { 0.4 } else { 0.6 };
                p.line_segment([pos2(r.left() + r.width() * 0.2, y), pos2(r.left() + r.width() * (0.2 + w), y)], white);
            }
        }
        "bug" => {
            let pts = vec![pos2(c.x, r.top()), pos2(r.right(), c.y), pos2(c.x, r.bottom()), pos2(r.left(), c.y)];
            p.add(Shape::convex_polygon(pts, BUG, Stroke::NONE));
            p.circle_filled(c, s * 0.25, Color32::WHITE);
        }
        "sub-task" | "subtask" => {
            p.rect_filled(r, s * 0.25, SUBTASK);
            let inner = Rect::from_min_size(r.min + r.size() * 0.3, r.size() * 0.45);
            p.rect_stroke(inner, s * 0.12, white, egui::StrokeKind::Inside);
        }
        _ => {
            p.rect_filled(r, s * 0.3, TASK);
            let a = pos2(r.left() + r.width() * 0.25, c.y + s * 0.02);
            let b = pos2(r.left() + r.width() * 0.43, r.top() + r.height() * 0.7);
            let d = pos2(r.left() + r.width() * 0.76, r.top() + r.height() * 0.3);
            p.line_segment([a, b], white);
            p.line_segment([b, d], white);
        }
    }
}

/// Priority as 1–4 filled bars (Highest = 4 … Lowest = none filled, one outline).
pub fn priority(ui: &mut Ui, t: &Tokens, name: &str, rank: i64, size: f32) -> Response {
    let (r, resp) = square(ui, size);
    paint_priority(ui.painter(), t, r, rank);
    resp.on_hover_text(format!("Priority: {name}"))
}

/// The priority bars into `r` (for rows painted without widgets, like the backlog's).
pub fn paint_priority(p: &Painter, t: &Tokens, r: Rect, rank: i64) {
    let filled = (5 - rank.clamp(1, 5)) as usize;
    let col = t.priority(rank);
    let bw = r.width() / 6.0;
    for i in 0..4usize {
        let h = r.height() * (0.35 + 0.18 * i as f32);
        let x = r.left() + bw * 0.5 + i as f32 * bw * 1.4;
        let bar = Rect::from_min_max(pos2(x, r.bottom() - h), pos2(x + bw, r.bottom()));
        if i < filled {
            p.rect_filled(bar, 1.0, col);
        } else {
            p.rect_stroke(bar, 1.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        }
    }
}

/// A drag handle: two columns of three dots (drawn; the bundled font has no handle glyph).
pub fn paint_drag_handle(p: &Painter, color: Color32, center: Pos2) {
    for dx in [-2.5f32, 2.5] {
        for dy in [-5.0f32, 0.0, 5.0] {
            p.circle_filled(center + vec2(dx, dy), 1.5, color);
        }
    }
}

/// Three horizontal dots: a "more actions" button face.
pub fn paint_more(p: &Painter, color: Color32, center: Pos2) {
    for dx in [-5.0f32, 0.0, 5.0] {
        p.circle_filled(center + vec2(dx, 0.0), 1.7, color);
    }
}

/// A round avatar with initials.
pub fn avatar(ui: &mut Ui, t: &Tokens, id: &str, display: &str, size: f32) -> Response {
    let (r, resp) = square(ui, size);
    paint_avatar(ui.painter(), t, r, id, display);
    resp.on_hover_text(display)
}

pub fn paint_avatar(p: &Painter, t: &Tokens, r: Rect, id: &str, display: &str) {
    let initials: String = display.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
    p.circle_filled(r.center(), r.width() / 2.0, t.avatar(id));
    p.text(r.center(), egui::Align2::CENTER_CENTER, initials, egui::FontId::proportional(r.width() * 0.42), Color32::WHITE);
}

/// An empty dashed circle for "Unassigned".
pub fn unassigned(ui: &mut Ui, t: &Tokens, size: f32) -> Response {
    let (r, resp) = square(ui, size);
    ui.painter().circle_stroke(r.center(), r.width() / 2.0 - 0.5, Stroke::new(1.0, t.muted));
    resp.on_hover_text("Unassigned")
}

/// A disclosure triangle: right when closed, down when open.
pub fn chevron(ui: &mut Ui, t: &Tokens, open: bool, size: f32) -> Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let c = r.center();
    let s = size * 0.22;
    let pts = if open {
        vec![c + vec2(-s, -s * 0.6), c + vec2(s, -s * 0.6), c + vec2(0.0, s * 0.8)]
    } else {
        vec![c + vec2(-s * 0.6, -s), c + vec2(-s * 0.6, s), c + vec2(s * 0.8, 0.0)]
    };
    let col = if resp.hovered() { t.text } else { t.muted };
    ui.painter().add(Shape::convex_polygon(pts, col, Stroke::NONE));
    resp
}

/// A page glyph for the wiki tree.
pub fn page(ui: &mut Ui, t: &Tokens, size: f32) -> Response {
    let (r, resp) = square(ui, size);
    let r = r.shrink2(vec2(size * 0.18, size * 0.08));
    let p = ui.painter();
    p.rect_stroke(r, 2.0, Stroke::new(1.2, t.muted), egui::StrokeKind::Inside);
    for k in 1..3 {
        let y = r.top() + r.height() * 0.3 * k as f32;
        p.line_segment([pos2(r.left() + 2.5, y), pos2(r.right() - 2.5, y)], Stroke::new(1.0, t.muted));
    }
    resp
}

/// The app mark: three stacked rounded bars (a board's columns), drawn in code.
pub fn app_mark(ui: &mut Ui, t: &Tokens, size: f32) -> Response {
    let (r, resp) = square(ui, size);
    let p = ui.painter();
    p.rect_filled(r, size * 0.22, t.accent);
    let w = r.width() * 0.18;
    for (i, h) in [0.55f32, 0.38, 0.22].iter().enumerate() {
        let x = r.left() + r.width() * 0.18 + i as f32 * w * 1.45;
        let bar = Rect::from_min_size(pos2(x, r.top() + r.height() * 0.2), vec2(w, r.height() * h));
        p.rect_filled(bar, w * 0.35, t.on_accent);
    }
    resp
}
