//! Look and feel: a calm, flat style for both system themes, plus shared colours.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Theme, Visuals};
use falcon_core::Rgb;

/// Accent used for selection and primary actions.
pub const ACCENT: Color32 = Color32::from_rgb(0x3D, 0x8B, 0xFD);
pub const OK: Color32 = Color32::from_rgb(0x34, 0xC7, 0x59);
pub const WARN: Color32 = Color32::from_rgb(0xFF, 0x9F, 0x0A);
pub const ERR: Color32 = Color32::from_rgb(0xFF, 0x45, 0x3A);

pub fn color(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Colours of the drawn keypad, per theme.
pub struct PadColors {
    pub case: Color32,
    pub case_edge: Color32,
    pub keycap: Color32,
    pub keycap_edge: Color32,
    pub legend: Color32,
    pub label: Color32,
    pub bar_off: Color32,
}

pub fn pad_colors(dark: bool) -> PadColors {
    if dark {
        PadColors {
            case: Color32::from_rgb(0x14, 0x15, 0x18),
            case_edge: Color32::from_rgb(0x3A, 0x3D, 0x44),
            keycap: Color32::from_rgb(0x2C, 0x2F, 0x35),
            keycap_edge: Color32::from_rgb(0x45, 0x49, 0x51),
            legend: Color32::from_gray(150),
            label: Color32::from_gray(235),
            bar_off: Color32::from_rgb(0x33, 0x36, 0x3C),
        }
    } else {
        PadColors {
            case: Color32::from_rgb(0x2A, 0x2C, 0x31),
            case_edge: Color32::from_rgb(0x15, 0x16, 0x19),
            keycap: Color32::from_rgb(0xEC, 0xEE, 0xF1),
            keycap_edge: Color32::from_rgb(0xC5, 0xC9, 0xD0),
            legend: Color32::from_gray(110),
            label: Color32::from_gray(25),
            bar_off: Color32::from_rgb(0x44, 0x47, 0x4E),
        }
    }
}

fn tune(visuals: &mut Visuals) {
    let r = CornerRadius::same(6);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = r;
    }
    visuals.selection.bg_fill = ACCENT.gamma_multiply(if visuals.dark_mode { 0.55 } else { 0.35 });
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.slider_trailing_fill = true;
}

/// Installs the style for both themes; egui follows the system theme by default.
pub fn install(ctx: &egui::Context) {
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            tune(&mut style.visuals);
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
            style.spacing.interact_size.y = 24.0;
            style.spacing.slider_width = 180.0;
        });
    }
}
