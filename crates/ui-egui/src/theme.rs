//! Theme tokens: every colour the UI paints comes from here. The palette is TrackCraft's own
//! (a teal accent, amber for work in progress) — never another product's design tokens.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use trackcraft_model::StatusCategory;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub accent_soft: Color32,
    pub danger: Color32,
    pub danger_soft: Color32,
    pub ok: Color32,
    pub code_bg: Color32,
    pub drop_hint: Color32,
}

pub(crate) const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb(((rgb >> 16) & 0xff) as u8, ((rgb >> 8) & 0xff) as u8, (rgb & 0xff) as u8)
}

impl Tokens {
    pub const LIGHT: Tokens = Tokens {
        dark: false,
        bg: hex(0xFAFAF7),
        sidebar: hex(0xF0F1EC),
        surface: hex(0xFFFFFF),
        surface_alt: hex(0xF4F5F1),
        border: hex(0xDADDD3),
        text: hex(0x1F2421),
        muted: hex(0x66706A),
        accent: hex(0x0E7A6B),
        on_accent: hex(0xFFFFFF),
        accent_soft: hex(0xD9F0EB),
        danger: hex(0xB4352B),
        danger_soft: hex(0xF8E1DE),
        ok: hex(0x2F7D32),
        code_bg: hex(0xEEF0EA),
        drop_hint: hex(0xC6E8E1),
    };

    pub const DARK: Tokens = Tokens {
        dark: true,
        bg: hex(0x161A18),
        sidebar: hex(0x1C211F),
        surface: hex(0x222826),
        surface_alt: hex(0x29302D),
        border: hex(0x3A433F),
        text: hex(0xE6EAE7),
        muted: hex(0x9BA6A0),
        accent: hex(0x3CC2A8),
        on_accent: hex(0x0B1714),
        accent_soft: hex(0x1F3D37),
        danger: hex(0xF0806F),
        danger_soft: hex(0x4A2622),
        ok: hex(0x7DCB82),
        code_bg: hex(0x2C3330),
        drop_hint: hex(0x24453F),
    };

    pub fn for_dark(dark: bool) -> Tokens {
        if dark { Tokens::DARK } else { Tokens::LIGHT }
    }

    /// Pill colours (fill, text) for a status category.
    pub fn status(&self, c: StatusCategory) -> (Color32, Color32) {
        match (c, self.dark) {
            (StatusCategory::ToDo, false) => (hex(0xE6E8E2), hex(0x4A524D)),
            (StatusCategory::ToDo, true) => (hex(0x343C38), hex(0xC9D1CC)),
            (StatusCategory::InProgress, false) => (hex(0xFBE7C2), hex(0x7A4A00)),
            (StatusCategory::InProgress, true) => (hex(0x4D3A12), hex(0xF5CF83)),
            (StatusCategory::Done, false) => (hex(0xD6EDD6), hex(0x1F5E23)),
            (StatusCategory::Done, true) => (hex(0x1F3F22), hex(0x9FDCA3)),
        }
    }

    /// A status category's colour in progress bars (sprint headers): grey, amber, green.
    pub fn progress(&self, c: StatusCategory) -> Color32 {
        match (c, self.dark) {
            (StatusCategory::ToDo, false) => hex(0xBCC2B9),
            (StatusCategory::ToDo, true) => hex(0x56605B),
            (StatusCategory::InProgress, false) => hex(0xE2A33A),
            (StatusCategory::InProgress, true) => hex(0xD99A35),
            (StatusCategory::Done, false) => hex(0x4E9A55),
            (StatusCategory::Done, true) => hex(0x6CBF72),
        }
    }

    /// Colour for a priority rank (1 = highest … 5 = lowest).
    pub fn priority(&self, rank: i64) -> Color32 {
        match rank {
            1 => hex(0xC62E2E),
            2 => hex(0xE0752D),
            3 => hex(0xC9A227),
            4 => hex(0x3E8E7E),
            _ => hex(0x7B8A84),
        }
    }

    /// A stable avatar colour for a user id (our own palette).
    pub fn avatar(&self, id: &str) -> Color32 {
        const P: [u32; 8] = [0x0E7A6B, 0x7A5AA8, 0xB0573A, 0x3A6EA5, 0x8A7A1E, 0xA8466F, 0x4E7D3A, 0x5A6470];
        let h = id.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
        hex(P.get((h % 8) as usize).copied().unwrap_or(0x5A6470))
    }

    pub fn visuals(&self) -> Visuals {
        let mut v = if self.dark { Visuals::dark() } else { Visuals::light() };
        v.panel_fill = self.bg;
        v.window_fill = self.surface;
        // Text fields get their own fill so they stand out on cards and dialogs.
        v.extreme_bg_color = if self.dark { self.bg } else { self.surface_alt };
        v.text_edit_bg_color = Some(v.extreme_bg_color);
        v.faint_bg_color = self.surface_alt;
        v.code_bg_color = self.code_bg;
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent_soft;
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.window_stroke = Stroke::new(1.0, self.border);
        v.override_text_color = Some(self.text);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        let r = CornerRadius::same(6);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
            w.corner_radius = r;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, self.border);
        v.widgets.inactive.weak_bg_fill = self.surface_alt;
        v.widgets.inactive.bg_fill = self.surface_alt;
        v.widgets.hovered.weak_bg_fill = self.accent_soft;
        v.widgets.hovered.bg_fill = self.accent_soft;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, self.accent);
        v.widgets.active.weak_bg_fill = self.accent_soft;
        v
    }
}
