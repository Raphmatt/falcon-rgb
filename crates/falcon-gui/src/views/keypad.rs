//! The drawn keypad: 4 × 5 keys and 4 light bars at their real relative positions
//! (`analysis/key-layout.json`), lit with the lighting preview.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, vec2,
};
use falcon_core::layout::{BAR_COUNT, COLS, KEY_COUNT, KEY_LEGENDS, LightBar};
use falcon_core::{Effect, KeyAssignment, Rgb, codes};

use super::lighting::{brightness_label, effect_label};
use crate::app::{FalconApp, Tab, Target};
use crate::theme;

/// Key positions in the Windows skin's pixels (`key-layout.json`).
const KEY_X: [f32; COLS] = [141.0, 190.0, 240.0, 289.0];
const KEY_Y: [f32; 5] = [110.0, 159.0, 209.0, 258.0, 307.0];
const KEY_SIZE: f32 = 43.0;
const BAR_X: [f32; 2] = [101.0, 360.0];
const BAR_Y: [f32; 2] = [116.0, 280.0];
const BAR_SIZE: [f32; 2] = [7.0, 101.0];
/// The keypad case in the same coordinates.
const CASE: [f32; 4] = [84.0, 84.0, 384.0, 392.0];

pub struct Geometry {
    pub case: Rect,
    pub keys: [Rect; KEY_COUNT],
    pub bars: [Rect; BAR_COUNT],
    pub scale: f32,
}

impl Geometry {
    /// The key or light bar under `p` (bars get a few points of slack, they are thin).
    pub fn target_at(&self, p: Pos2) -> Option<Target> {
        if let Some(k) = self.keys.iter().position(|r| r.contains(p)) {
            return Some(Target::Key(k));
        }
        self.bars
            .iter()
            .position(|r| r.expand(4.0 * self.scale).contains(p))
            .map(Target::Bar)
    }

    /// Every target touched by a pointer moving from `a` to `b`, in order, without repeats.
    /// Sampled every couple of points so a fast drag skips nothing between two frames.
    pub fn targets_along(&self, a: Pos2, b: Pos2) -> Vec<Target> {
        let steps = ((b - a).length() / 2.0).ceil().max(1.0) as usize;
        let mut out: Vec<Target> = Vec::new();
        for i in 0..=steps {
            if let Some(t) = self.target_at(a.lerp(b, i as f32 / steps as f32))
                && out.last() != Some(&t)
            {
                out.push(t);
            }
        }
        out
    }
}

/// Fits the keypad into `avail`, centred.
pub fn geometry(avail: Rect) -> Geometry {
    let ref_w = CASE[2] - CASE[0];
    let ref_h = CASE[3] - CASE[1];
    let scale = (avail.width() / ref_w).min(avail.height() / ref_h).max(0.1) * 0.96;
    let origin = avail.center() - vec2(ref_w, ref_h) * scale / 2.0;
    let map = |x: f32, y: f32| origin + vec2(x - CASE[0], y - CASE[1]) * scale;
    let rect = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(map(x, y), vec2(w, h) * scale);
    Geometry {
        case: Rect::from_min_max(map(CASE[0], CASE[1]), map(CASE[2], CASE[3])),
        keys: std::array::from_fn(|k| rect(KEY_X[k % COLS], KEY_Y[k / COLS], KEY_SIZE, KEY_SIZE)),
        bars: std::array::from_fn(|b| {
            let (side, row) = match LightBar::ALL[b] {
                LightBar::LeftUpper => (0, 0),
                LightBar::RightUpper => (1, 0),
                LightBar::LeftLower => (0, 1),
                LightBar::RightLower => (1, 1),
            };
            rect(BAR_X[side], BAR_Y[row], BAR_SIZE[0], BAR_SIZE[1])
        }),
        scale,
    }
}

fn short_mod(code: u8) -> String {
    match code {
        0xE0 | 0xE4 => "Ctrl".into(),
        0xE1 | 0xE5 => "Shift".into(),
        0xE2 | 0xE6 => "Alt".into(),
        0xE3 | 0xE7 => "Cmd".into(),
        c => codes::code_label(c),
    }
}

/// Short text for a keycap.
pub fn cap_label(a: &KeyAssignment, k: usize) -> String {
    match a {
        KeyAssignment::Default => KEY_LEGENDS[k].to_owned(),
        KeyAssignment::Disabled => "—".into(),
        KeyAssignment::Key(c) => codes::code_label(*c),
        KeyAssignment::Combo(cs) => cs
            .iter()
            .map(|&c| short_mod(c))
            .collect::<Vec<_>>()
            .join("+"),
        KeyAssignment::Macro(m) => format!("Macro\n{} ev", m.events.len()),
        KeyAssignment::Function(f) => f.label().to_owned(),
        KeyAssignment::LaunchProgram(p) => {
            let name = p
                .trim_end_matches('/')
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(p);
            format!("Open\n{}", name.trim_end_matches(".app"))
        }
    }
}

/// Largest font (from `size` down) at which every word of `text` fits into `max_w`, so labels
/// wrap between words instead of inside them.
fn fit_font(painter: &egui::Painter, text: &str, max_w: f32, size: f32) -> FontId {
    let mut font = FontId::proportional(size);
    for step in [1.0, 0.88, 0.76, 0.66] {
        font = FontId::proportional(size * step);
        let widest = text
            .split_whitespace()
            .map(|w| {
                painter
                    .layout_no_wrap(w.to_owned(), font.clone(), Color32::WHITE)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        if widest <= max_w {
            break;
        }
    }
    font
}

fn luminance(c: Rgb) -> f32 {
    (0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b)) / 255.0
}

fn glow(painter: &egui::Painter, rect: Rect, c: Rgb, radius: f32, strength: f32) {
    let lum = luminance(c).max(0.15);
    if c == Rgb::BLACK {
        return;
    }
    for i in 1..=3 {
        let a = (strength * lum * (0.45 - 0.12 * i as f32) * 255.0).clamp(0.0, 255.0) as u8;
        let col = Color32::from_rgba_unmultiplied(c.r, c.g, c.b, a);
        painter.rect_filled(
            rect.expand(radius * i as f32),
            radius * (1.0 + i as f32),
            col,
        );
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp) {
    let dark = ui.visuals().dark_mode;
    let pc = theme::pad_colors(dark);
    let profile = app.profile().clone();
    let l = &profile.lighting;

    ui.vertical_centered(|ui| {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(&profile.name).size(20.0).strong());
        ui.label(
            egui::RichText::new(format!(
                "Layer {} · {} · brightness {}",
                app.active() + 1,
                effect_label(l.effect),
                brightness_label(l.brightness)
            ))
            .weak(),
        );
    });

    let avail = ui.available_rect_before_wrap().shrink2(vec2(8.0, 30.0));
    let g = geometry(avail);
    let s = g.scale;
    let frame = app.preview_frame();
    let painter = ui.painter_at(ui.available_rect_before_wrap());
    let paint_mode = app.tab == Tab::Lighting && l.effect == Effect::Custom;

    // Case.
    painter.rect(
        g.case,
        18.0 * s,
        pc.case,
        Stroke::new(1.5 * s, pc.case_edge),
        StrokeKind::Inside,
    );

    let mut clicked: Option<Target> = None;

    for (b, rect) in g.bars.iter().enumerate() {
        let c = frame.bars[b];
        glow(&painter, *rect, c, 4.0 * s, 1.0);
        let fill = if c == Rgb::BLACK {
            pc.bar_off
        } else {
            theme::color(c)
        };
        painter.rect_filled(*rect, 3.0 * s, fill);
        let mut resp = ui.interact(
            rect.expand(4.0 * s),
            ui.scope_id().with(("bar", b)),
            Sense::click(),
        );
        // No tooltips while painting: they would follow the brush around.
        if !paint_mode {
            resp = resp.on_hover_text(format!("Light bar {}", LightBar::ALL[b].short_name()));
        }
        if app.target == Some(Target::Bar(b)) {
            painter.rect_stroke(
                rect.expand(3.0 * s),
                4.0 * s,
                Stroke::new(2.0, theme::ACCENT),
                StrokeKind::Outside,
            );
        }
        if resp.clicked() {
            clicked = Some(Target::Bar(b));
        }
    }

    let recording_key = app
        .recording
        .filter(|r| r.profile == app.active())
        .map(|r| r.key);
    for (k, rect) in g.keys.iter().enumerate() {
        let c = frame.keys[k];
        let lit = c != Rgb::BLACK;
        glow(&painter, *rect, c, 3.0 * s, 0.9);
        let led = theme::color(c);
        let fill = if lit {
            pc.keycap.lerp_to_gamma(led, 0.22)
        } else {
            pc.keycap
        };
        let edge = if lit { led } else { pc.keycap_edge };
        painter.rect(
            *rect,
            6.0 * s,
            fill,
            Stroke::new(1.5 * s, edge),
            StrokeKind::Inside,
        );

        let a = &profile.keys[k];
        if !matches!(a, KeyAssignment::Default) {
            painter.text(
                rect.min + vec2(4.0, 3.0) * s,
                Align2::LEFT_TOP,
                KEY_LEGENDS[k],
                FontId::proportional(6.5 * s),
                pc.legend,
            );
        }
        let color = match a {
            KeyAssignment::Default => pc.label,
            _ => theme::ACCENT,
        };
        let text = cap_label(a, k);
        let max_w = rect.width() - 6.0 * s;
        let font = fit_font(&painter, &text, max_w, 8.5 * s);
        let galley = painter.layout(text, font, color, max_w);
        let pos = rect.center() - galley.size() / 2.0 + vec2(0.0, 2.0 * s);
        painter.galley(pos, galley, color);
        if a.is_host_action() {
            painter.circle_filled(rect.right_top() + vec2(-5.0, 5.0) * s, 2.5 * s, theme::OK);
        }

        let mut resp = ui.interact(*rect, ui.scope_id().with(("key", k)), Sense::click());
        if !paint_mode {
            resp = resp.on_hover_text(format!("Key {} · {}\n{}", k + 1, KEY_LEGENDS[k], a.label()));
        }
        let selected = app.target == Some(Target::Key(k));
        if recording_key == Some(k) {
            painter.rect_stroke(
                rect.expand(3.0 * s),
                8.0 * s,
                Stroke::new(2.5, theme::ERR),
                StrokeKind::Outside,
            );
        } else if selected {
            painter.rect_stroke(
                rect.expand(3.0 * s),
                8.0 * s,
                Stroke::new(2.5, theme::ACCENT),
                StrokeKind::Outside,
            );
        } else if resp.hovered() {
            painter.rect_stroke(
                rect.expand(2.0 * s),
                7.0 * s,
                Stroke::new(1.0, ui.visuals().weak_text_color()),
                StrokeKind::Outside,
            );
        }
        if resp.clicked() {
            clicked = Some(Target::Key(k));
        }
        if resp.double_clicked() && !paint_mode {
            app.tab = Tab::Keys;
        }
    }

    if paint_mode {
        paint_strokes(ui, app, &g);
    } else {
        app.paint_last = None;
    }
    if let Some(t) = clicked {
        app.target = Some(t);
        if matches!(t, Target::Bar(_)) {
            app.tab = Tab::Lighting;
        }
    }

    let hint = if paint_mode {
        "Click or drag over keys and light bars to paint them · ⌘Z / Ctrl+Z undoes"
    } else {
        "Click a key to edit it · double-click opens its assignment"
    };
    painter.text(
        Pos2::new(g.case.center().x, g.case.max.y + 16.0),
        Align2::CENTER_CENTER,
        hint,
        FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
}

/// Painting (Lighting tab, Custom effect): a press inside the keypad starts a stroke, and every
/// key and light bar the pointer passes over while the button is held takes the paint colour.
/// One stroke is one undo step. Over the keypad the pointer becomes a brush in that colour.
fn paint_strokes(ui: &egui::Ui, app: &mut FalconApp, g: &Geometry) {
    let area = g.case.expand(6.0 * g.scale);
    let (pressed, down, pos, hover) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.interact_pos(),
            i.pointer.hover_pos(),
        )
    });
    if pressed && pos.is_some_and(|p| area.contains(p)) {
        app.begin_paint_stroke();
        app.paint_last = pos;
    }
    if !down {
        app.paint_last = None;
    }
    if let (Some(last), Some(p)) = (app.paint_last, pos) {
        for t in g.targets_along(last, p) {
            app.paint_target(t);
        }
        app.paint_last = Some(p);
    }
    if let Some(h) = hover.filter(|h| area.contains(*h)) {
        ui.ctx().set_cursor_icon(CursorIcon::None);
        draw_brush(ui.ctx(), h, app.paint, ui.visuals().dark_mode);
    }
}

/// A small pencil whose tip (at `tip`, the hot spot) shows the paint colour.
fn draw_brush(ctx: &egui::Context, tip: Pos2, color: Rgb, dark: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("paint-brush"),
    ));
    let (outline, body) = if dark {
        (Color32::from_gray(235), Color32::from_gray(70))
    } else {
        (Color32::from_gray(30), Color32::from_gray(225))
    };
    let dir = vec2(1.0, -1.0).normalized();
    let half = vec2(-dir.y, dir.x) * 4.0;
    let nib = tip + dir * 9.0;
    let end = tip + dir * 26.0;
    let stroke = Stroke::new(1.2, outline);
    painter.add(Shape::convex_polygon(
        vec![nib + half, end + half, end - half, nib - half],
        body,
        stroke,
    ));
    painter.add(Shape::convex_polygon(
        vec![tip, nib - half, nib + half],
        theme::color(color),
        stroke,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use falcon_core::{Function, Macro};

    #[test]
    fn strokes_reach_every_key_they_cross() {
        let avail = Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(600.0, 500.0));
        let g = geometry(avail);
        assert_eq!(g.target_at(g.keys[6].center()), Some(Target::Key(6)));
        assert_eq!(g.target_at(g.bars[1].center()), Some(Target::Bar(1)));
        assert_eq!(g.target_at(g.case.min), None);
        // One frame's jump from the first to the last key of the top row hits all four.
        let row = g.targets_along(g.keys[0].center(), g.keys[3].center());
        assert_eq!(
            row,
            vec![
                Target::Key(0),
                Target::Key(1),
                Target::Key(2),
                Target::Key(3)
            ]
        );
        // A click (no movement) paints just the key under the pointer.
        assert_eq!(
            g.targets_along(g.keys[9].center(), g.keys[9].center()),
            vec![Target::Key(9)]
        );
    }

    #[test]
    fn geometry_fits_and_does_not_overlap() {
        let avail = Rect::from_min_size(Pos2::new(10.0, 20.0), vec2(600.0, 500.0));
        let g = geometry(avail);
        assert!(avail.contains_rect(g.case));
        for (i, a) in g.keys.iter().enumerate() {
            assert!(g.case.contains_rect(*a));
            for b in &g.keys[i + 1..] {
                assert!(!a.intersects(*b));
            }
            for bar in &g.bars {
                assert!(!a.intersects(*bar));
            }
        }
        // Row-major k: key 1 is right of key 0, key 4 below key 0.
        assert!(g.keys[1].min.x > g.keys[0].max.x);
        assert!(g.keys[4].min.y > g.keys[0].max.y);
        // Bars: left upper, right upper, left lower, right lower.
        assert!(g.bars[0].max.x < g.keys[0].min.x && g.bars[1].min.x > g.keys[3].max.x);
        assert!(g.bars[2].min.y > g.bars[0].max.y && g.bars[3].min.y > g.bars[1].max.y);
    }

    #[test]
    fn cap_labels() {
        assert_eq!(cap_label(&KeyAssignment::Default, 4), "Num 7");
        assert_eq!(
            cap_label(&KeyAssignment::Combo(vec![0xE3, 0xE1, 0x04]), 0),
            "Cmd+Shift+A"
        );
        assert_eq!(
            cap_label(
                &KeyAssignment::LaunchProgram("/Applications/Safari.app".into()),
                0
            ),
            "Open\nSafari"
        );
        assert_eq!(
            cap_label(&KeyAssignment::Function(Function::VolumeUp), 0),
            "Volume Up"
        );
        assert_eq!(
            cap_label(&KeyAssignment::Macro(Macro::default()), 0),
            "Macro\n0 ev"
        );
    }
}
