//! Painted charts (UI-24): lines, grouped bars and stacked areas over a shared category axis
//! (days or sprints), with a y axis, a legend, a hover tooltip with exact values, and an
//! accessible data table.
//!
//! No plotting crate: everything is drawn with egui's painter. The look follows one method:
//! - colour by job: three categorical slots per theme, in fixed order (blue, orange, aqua),
//!   checked for colour-blind separation and contrast against our light and dark surfaces;
//!   a reference line (the burndown's ideal, a commitment) is a neutral grey, not a series hue;
//! - thin marks: 2 px lines, an end dot with a 2 px surface ring, bars at most 24 px wide with
//!   4 px rounded data ends and a 2 px gap, stacked bands parted by a 2 px surface line;
//! - recessive hairline grid and baseline, muted axis text; values and legends in text colours;
//! - identity is never colour alone: a legend for two or more series, the tooltip names each
//!   series, and the data table lists every value.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::theme::{Tokens, hex};

/// Categorical slot `n` (0-based) for the theme.
pub(crate) fn series_color(t: &Tokens, n: usize) -> Color32 {
    const LIGHT: [u32; 3] = [0x2A78D6, 0xEB6834, 0x1BAF7A];
    const DARK: [u32; 3] = [0x3987E5, 0xD95926, 0x199E70];
    let p = if t.dark { &DARK } else { &LIGHT };
    hex(p.get(n).or(p.first()).copied().unwrap_or(0x2A78D6))
}

/// The neutral colour of a reference line (ideal, commitment).
pub(crate) fn reference_color(t: &Tokens) -> Color32 {
    if t.dark { hex(0x8B948F) } else { hex(0x9CA39E) }
}

fn grid_color(t: &Tokens) -> Color32 {
    if t.dark { hex(0x323936) } else { hex(0xE9EBE5) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    /// A 2 px line with an end dot.
    Line,
    /// A thin neutral line (ideal, target).
    Reference,
    /// A column, grouped with the other bar series at each category.
    Bar,
    /// A band stacked on the area series before it.
    Area,
}

#[derive(Debug, Clone)]
pub(crate) struct Series {
    pub name: String,
    pub color: Color32,
    pub mark: Mark,
    /// One value per category; `None` = no value there (a line breaks).
    pub values: Vec<Option<f64>>,
}

#[derive(Debug, Clone)]
pub(crate) struct Chart {
    /// The accessible name: what the chart shows.
    pub label: String,
    /// The category axis' title in the data table ("Date", "Sprint").
    pub x_title: String,
    /// Category labels: the tooltip and table text.
    pub x: Vec<String>,
    /// Shorter axis labels (same length as `x`; empty = use `x`).
    pub x_short: Vec<String>,
    /// Positions on a time axis (same length as `x`, from 0; empty = evenly spaced categories).
    pub x_at: Vec<f64>,
    pub series: Vec<Series>,
    pub height: f32,
    /// A dashboard gadget: smaller text, fewer ticks.
    pub compact: bool,
}

/// A value as text: whole numbers without decimals, others to at most two places.
pub(crate) fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if (v - v.round()).abs() < 1e-9 {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// "Name: value" for every series with a value at category `i` — the tooltip's lines (stacked
/// areas show their own value, not the running total).
pub(crate) fn tooltip_lines(c: &Chart, i: usize) -> Vec<(String, String)> {
    c.series.iter().filter_map(|s| s.values.get(i).copied().flatten().map(|v| (s.name.clone(), num(v)))).collect()
}

/// A "nice" axis top and tick step for values up to `max`.
fn ticks(max: f64, integral: bool, n: usize) -> (f64, f64) {
    let max = if max.is_finite() && max > 0.0 { max } else { 1.0 };
    let raw = max / n.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let mut step = mag
        * if norm <= 1.0 {
            1.0
        } else if norm <= 2.0 {
            2.0
        } else if norm <= 5.0 {
            5.0
        } else {
            10.0
        };
    if integral {
        step = step.max(1.0).round();
    }
    let top = (max / step).ceil() * step;
    (top.max(step), step)
}

/// Paints the chart and returns its response (hovered = the tooltip is showing).
pub(crate) fn chart(ui: &mut Ui, t: &Tokens, c: &Chart) -> egui::Response {
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, c.height), Sense::hover());
    let label = c.label.clone();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, label.clone()));
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let n = c.x.len();
    let font = FontId::proportional(if c.compact { 10.5 } else { 11.5 });
    // The y range: the largest value, or stacked total.
    let mut max: f64 = 0.0;
    let mut integral = true;
    for i in 0..n {
        let mut stack = 0.0;
        for s in &c.series {
            let v = s.values.get(i).copied().flatten().unwrap_or(0.0);
            integral &= (v - v.round()).abs() < 1e-9;
            if s.mark == Mark::Area {
                stack += v;
            } else {
                max = max.max(v);
            }
        }
        max = max.max(stack);
    }
    let (top, step) = ticks(max, integral, if c.compact { 2 } else { 4 });
    let painter = ui.painter_at(rect);
    let tick_text: Vec<(f64, String)> = {
        let mut v = Vec::new();
        let mut y = 0.0;
        while y <= top + step * 0.01 && v.len() < 12 {
            v.push((y, num(y)));
            y += step;
        }
        v
    };
    let label_w = tick_text.iter().map(|(_, s)| ui.fonts_mut(|f| f.layout_no_wrap(s.clone(), font.clone(), t.muted).size().x)).fold(0.0f32, f32::max);
    let plot = Rect::from_min_max(pos2(rect.left() + label_w + 8.0, rect.top() + 6.0), pos2(rect.right() - 6.0, rect.bottom() - 20.0));
    if plot.width() < 10.0 || plot.height() < 10.0 || n == 0 {
        return resp;
    }
    let y_of = |v: f64| plot.bottom() - (v / top).clamp(0.0, 1.05) as f32 * plot.height();
    let band = plot.width() / n as f32;
    let max_at = c.x_at.iter().copied().filter(|v| v.is_finite()).fold(0.0f64, f64::max);
    let timed = c.x_at.len() == n && max_at > 0.0;
    let x_of = |i: usize| {
        if timed {
            let f = (c.x_at.get(i).copied().unwrap_or(0.0) / max_at).clamp(0.0, 1.0) as f32;
            plot.left() + 8.0 + f * (plot.width() - 16.0)
        } else {
            plot.left() + band * (i as f32 + 0.5)
        }
    };
    // Grid and y labels.
    for (v, s) in &tick_text {
        let y = y_of(*v);
        let col = if *v == 0.0 { t.border } else { grid_color(t) };
        painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, col));
        painter.text(pos2(plot.left() - 6.0, y), Align2::RIGHT_CENTER, s, font.clone(), t.muted);
    }
    // X labels, thinned so they don't collide.
    let short = |i: usize| c.x_short.get(i).or(c.x.get(i)).cloned().unwrap_or_default();
    let widest = (0..n).map(|i| ui.fonts_mut(|f| f.layout_no_wrap(short(i), font.clone(), t.muted).size().x)).fold(0.0f32, f32::max);
    let mut drawn: Option<f32> = None;
    for i in 0..n {
        let x = x_of(i);
        if drawn.is_some_and(|d| x - d < widest + 10.0) || x + widest / 2.0 > rect.right() + 4.0 {
            continue;
        }
        painter.text(pos2(x, plot.bottom() + 4.0), Align2::CENTER_TOP, short(i), font.clone(), t.muted);
        drawn = Some(x);
    }
    let hover = resp.hover_pos().filter(|p| plot.expand2(vec2(8.0, 8.0)).contains(*p));
    let hovered = hover.and_then(|p| (0..n).min_by(|a, b| (x_of(*a) - p.x).abs().total_cmp(&(x_of(*b) - p.x).abs())));
    if let Some(i) = hovered {
        let x = x_of(i);
        if timed {
            painter.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, t.border));
        } else {
            painter.rect_filled(
                Rect::from_center_size(pos2(x, plot.center().y), vec2(band.min(40.0), plot.height())),
                0.0,
                grid_color(t).gamma_multiply(0.6),
            );
        }
    }
    let geo = Geo { plot, band, x_of: &x_of, y_of: &y_of };
    paint_areas(&painter, t, c, n, &geo);
    paint_bars(&painter, c, n, &geo);
    paint_lines(&painter, t, c, n, hovered, &geo);
    match hovered {
        Some(i) => {
            let title = c.x.get(i).cloned().unwrap_or_default();
            let lines = tooltip_lines(c, i);
            let colors: Vec<(String, Color32)> = c.series.iter().map(|s| (s.name.clone(), s.color)).collect();
            resp.on_hover_ui_at_pointer(|ui| {
                ui.label(RichText::new(title).strong());
                for (name, v) in &lines {
                    ui.horizontal(|ui| {
                        let col = colors.iter().find(|(n, _)| n == name).map_or(t.muted, |(_, c)| *c);
                        swatch(ui, col);
                        ui.label(RichText::new(format!("{name}: {v}")).color(t.text));
                    });
                }
            })
        }
        None => resp,
    }
}

/// Where categories and values sit inside the plot.
struct Geo<'a> {
    plot: Rect,
    band: f32,
    x_of: &'a dyn Fn(usize) -> f32,
    y_of: &'a dyn Fn(f64) -> f32,
}

/// Stacked areas, bottom first: a band between the running totals.
fn paint_areas(painter: &egui::Painter, t: &Tokens, c: &Chart, n: usize, g: &Geo<'_>) {
    let areas: Vec<&Series> = c.series.iter().filter(|s| s.mark == Mark::Area).collect();
    if !areas.is_empty() {
        let mut base = vec![0.0f64; n];
        let mut tops: Vec<Vec<Pos2>> = Vec::new();
        for s in &areas {
            let mut mesh = egui::Mesh::default();
            let fill = s.color.gamma_multiply(0.85);
            let mut upper = Vec::with_capacity(n);
            for i in 0..n {
                let lo = base.get(i).copied().unwrap_or(0.0);
                let hi = lo + s.values.get(i).copied().flatten().unwrap_or(0.0);
                upper.push(pos2((g.x_of)(i), (g.y_of)(hi)));
                mesh.colored_vertex(pos2((g.x_of)(i), (g.y_of)(lo)), fill);
                mesh.colored_vertex(pos2((g.x_of)(i), (g.y_of)(hi)), fill);
                if let Some(b) = base.get_mut(i) {
                    *b = hi;
                }
            }
            for i in 1..n {
                let k = u32::try_from(i).unwrap_or(0).saturating_mul(2);
                if k >= 2 {
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k + 1, k);
                }
            }
            if n == 1 {
                // One day: a column-shaped band.
                if let (Some(lo), Some(hi)) = (mesh.vertices.first().map(|v| v.pos), mesh.vertices.get(1).map(|v| v.pos)) {
                    painter.rect_filled(Rect::from_min_max(pos2(lo.x - g.band * 0.3, hi.y), pos2(lo.x + g.band * 0.3, lo.y)), 0.0, fill);
                }
            } else {
                painter.add(egui::Shape::mesh(mesh));
            }
            tops.push(upper);
        }
        // The 2 px surface gap between bands.
        for line in tops.iter().take(tops.len().saturating_sub(1)) {
            if line.len() > 1 {
                painter.add(egui::Shape::line(line.clone(), Stroke::new(2.0, t.surface)));
            }
        }
    }
}

/// Grouped bars.
fn paint_bars(painter: &egui::Painter, c: &Chart, n: usize, g: &Geo<'_>) {
    let bars: Vec<&Series> = c.series.iter().filter(|s| s.mark == Mark::Bar).collect();
    if !bars.is_empty() {
        let k = bars.len() as f32;
        let gap = 2.0;
        let w = ((g.band * 0.7 - gap * (k - 1.0)) / k).clamp(2.0, 24.0);
        let group = w * k + gap * (k - 1.0);
        for i in 0..n {
            for (j, s) in bars.iter().enumerate() {
                let Some(v) = s.values.get(i).copied().flatten() else { continue };
                let x0 = (g.x_of)(i) - group / 2.0 + j as f32 * (w + gap);
                let y = (g.y_of)(v);
                let h = g.plot.bottom() - y;
                if h <= 0.0 {
                    continue;
                }
                let r = h.min(4.0) as u8;
                let radius = CornerRadius { nw: r, ne: r, sw: 0, se: 0 };
                painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x0 + w, g.plot.bottom())), radius, s.color);
            }
        }
    }
}

/// Lines: references first, so data lines sit on top.
fn paint_lines(painter: &egui::Painter, t: &Tokens, c: &Chart, n: usize, hovered: Option<usize>, g: &Geo<'_>) {
    for s in c.series.iter().filter(|s| s.mark == Mark::Reference).chain(c.series.iter().rev().filter(|s| s.mark == Mark::Line)) {
        // (Lines draw last-listed first, so the first series — the story — sits on top.)
        let stroke = if s.mark == Mark::Reference { Stroke::new(1.5, s.color) } else { Stroke::new(2.0, s.color) };
        let mut run: Vec<Pos2> = Vec::new();
        let mut last = None;
        for i in 0..n {
            match s.values.get(i).copied().flatten() {
                Some(v) => {
                    let p = pos2((g.x_of)(i), (g.y_of)(v));
                    run.push(p);
                    last = Some(p);
                }
                None => {
                    flush(painter, &mut run, stroke);
                }
            }
        }
        flush(painter, &mut run, stroke);
        if s.mark == Mark::Line
            && let Some(p) = last
        {
            painter.circle_filled(p, 6.0, t.surface);
            painter.circle_filled(p, 4.0, s.color);
        }
        if let Some(i) = hovered
            && let Some(v) = s.values.get(i).copied().flatten()
            && s.mark != Mark::Bar
        {
            let p = pos2((g.x_of)(i), (g.y_of)(v));
            painter.circle_filled(p, 6.0, t.surface);
            painter.circle_filled(p, 4.0, s.color);
        }
    }
}

fn flush(painter: &egui::Painter, run: &mut Vec<Pos2>, stroke: Stroke) {
    if run.len() > 1 {
        painter.add(egui::Shape::line(std::mem::take(run), stroke));
    } else if let Some(p) = run.pop() {
        painter.circle_filled(p, stroke.width + 1.0, stroke.color);
    }
    run.clear();
}

fn swatch(ui: &mut Ui, c: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().rect_filled(r, 2.0, c);
}

/// The legend: a swatch and the series name, in text colour, for each series.
pub(crate) fn legend(ui: &mut Ui, t: &Tokens, c: &Chart) {
    if c.series.len() < 2 {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for s in &c.series {
            if s.mark == Mark::Reference {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 10.0), Sense::hover());
                ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.5, s.color));
            } else {
                swatch(ui, s.color);
            }
            ui.label(RichText::new(&s.name).color(t.text).size(12.5));
            ui.add_space(10.0);
        }
    });
}

/// Every value of the chart as a table (for screen readers and exact reading).
pub(crate) fn data_table(ui: &mut Ui, t: &Tokens, id: &str, c: &Chart) {
    egui::ScrollArea::vertical().id_salt(("chart-table", id)).max_height(280.0).show(ui, |ui| {
        egui::Grid::new(("chart-table-grid", id)).striped(true).spacing([18.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new(&c.x_title).strong().color(t.text));
            for s in &c.series {
                ui.label(RichText::new(&s.name).strong().color(t.text));
            }
            ui.end_row();
            for (i, x) in c.x.iter().enumerate() {
                ui.label(RichText::new(x).color(t.text));
                for s in &c.series {
                    let v = s.values.get(i).copied().flatten().map_or_else(|| "-".to_string(), num);
                    ui.label(RichText::new(v).color(t.text).monospace());
                }
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_ticks() {
        assert_eq!(num(3.0), "3");
        assert_eq!(num(2.5), "2.5");
        assert_eq!(num(1.0 / 3.0), "0.33");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(ticks(8.0, true, 4), (8.0, 2.0));
        assert_eq!(ticks(0.0, true, 4), (1.0, 1.0));
        assert_eq!(ticks(3.0, true, 4), (3.0, 1.0));
        let (top, step) = ticks(f64::INFINITY, false, 4);
        assert!(top.is_finite() && step > 0.0);
    }
}
