//! Lighting panel: effect, colour, brightness, wave direction, per-key painting, apply.

use eframe::egui::{self, RichText};
use falcon_core::layout::LightBar;
use falcon_core::model::BRIGHTNESS_OFF;
use falcon_core::{Effect, Rgb, WaveDirection};

use super::{heading, hint};
use crate::app::{FalconApp, Target};
use crate::theme;

const SWATCHES: [Rgb; 10] = [
    Rgb::new(0xFF, 0x00, 0x00),
    Rgb::new(0xFF, 0x80, 0x00),
    Rgb::new(0xFF, 0xFF, 0x00),
    Rgb::new(0x00, 0xFF, 0x00),
    Rgb::new(0x00, 0xFF, 0xFF),
    Rgb::new(0x00, 0x00, 0xFF),
    Rgb::new(0x80, 0x00, 0xFF),
    Rgb::new(0xFF, 0x00, 0xFF),
    Rgb::new(0xFF, 0xFF, 0xFF),
    Rgb::new(0x00, 0x00, 0x00),
];

pub fn effect_label(e: Effect) -> String {
    match e {
        Effect::Static => "Static".into(),
        Effect::Breathing => "Breathing".into(),
        Effect::FadeIn => "Fade in".into(),
        Effect::FadeOut => "Fade out".into(),
        Effect::LastKey => "Last key stays lit".into(),
        Effect::Wave => "RGB wave".into(),
        Effect::Random => "Random single key".into(),
        Effect::Custom => "Custom (per key)".into(),
        Effect::Other(b) => format!("Effect {b} (unknown)"),
    }
}

/// Brightness 1..=5 for display: 1 switches the LEDs off on the keypad (block byte 0).
pub fn brightness_label(brightness: u8) -> String {
    if brightness == BRIGHTNESS_OFF {
        "off".into()
    } else {
        brightness.to_string()
    }
}

fn direction_label(d: WaveDirection) -> String {
    match d {
        WaveDirection::RightToLeft => "← To left".into(),
        WaveDirection::LeftToRight => "To right →".into(),
        WaveDirection::TopToBottom => "↓ Down".into(),
        WaveDirection::BottomToTop => "↑ Up".into(),
        WaveDirection::Other(b) => format!("raw {b}"),
    }
}

/// Colour picker + hex field + swatches. Returns true on change.
pub fn color_row(ui: &mut egui::Ui, id_salt: &str, c: &mut Rgb) -> bool {
    let before = *c;
    ui.horizontal(|ui| {
        let mut arr = [c.r, c.g, c.b];
        if ui.color_edit_button_srgb(&mut arr).changed() {
            *c = Rgb::new(arr[0], arr[1], arr[2]);
        }
        let id = egui::Id::unique(("hex-colour", id_salt));
        let mut text = ui
            .data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| format!("#{c}"));
        let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(72.0));
        if resp.changed()
            && let Ok(v) = text.parse::<Rgb>()
        {
            *c = v;
        }
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text));
        } else {
            ui.data_mut(|d| d.remove::<String>(id));
        }
        for s in SWATCHES {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
            ui.painter().rect(
                rect,
                4.0,
                theme::color(s),
                egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
                egui::StrokeKind::Inside,
            );
            if resp.on_hover_text(format!("#{s}")).clicked() {
                *c = s;
            }
        }
    });
    *c != before
}

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp) {
    let p = app.active();
    let mut l = app.profile().lighting.clone();
    let before = l.clone();

    heading(ui, &format!("Lighting of layer {}", p + 1));
    egui::Grid::new("lighting-grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label("Effect");
            egui::ComboBox::from_id_salt("effect")
                .selected_text(effect_label(l.effect))
                .width(200.0)
                .show_ui(ui, |ui| {
                    for e in Effect::ALL {
                        ui.selectable_value(&mut l.effect, e, effect_label(e));
                    }
                });
            ui.end_row();

            ui.label("Brightness");
            ui.add(
                egui::Slider::new(&mut l.brightness, 1..=5)
                    .custom_formatter(|v, _| brightness_label(v as u8))
                    .custom_parser(|s| match s.trim() {
                        "off" => Some(f64::from(BRIGHTNESS_OFF)),
                        n => n.parse().ok(),
                    }),
            )
            .on_hover_text("1 switches the LEDs off; 5 is full brightness");
            ui.end_row();

            if l.effect.uses_color() || matches!(l.effect, Effect::Other(_)) {
                ui.label("Colour");
                color_row(ui, "effect", &mut l.color);
                ui.end_row();
            }
            if l.effect.uses_direction() {
                ui.label("Direction");
                ui.horizontal_wrapped(|ui| {
                    for d in WaveDirection::ALL {
                        ui.selectable_value(&mut l.direction, d, direction_label(d));
                    }
                    if let WaveDirection::Other(_) = l.direction {
                        ui.label(
                            RichText::new(format!("({})", direction_label(l.direction))).weak(),
                        );
                    }
                });
                ui.end_row();
            }
        });

    if l.effect == Effect::Custom {
        custom_editor(ui, app, &mut l);
    } else if l.effect == Effect::Random {
        hint(ui, "Random colours; no settings besides brightness.");
    }

    if l != before {
        app.profile_mut().lighting = l;
        app.lighting_edited();
    }

    ui.add_space(10.0);
    ui.separator();
    ui.horizontal(|ui| {
        let can = app.connected();
        if ui
            .add_enabled(can, egui::Button::new("Apply to keypad"))
            .on_hover_text("Send this layer's lighting now (Save also writes the keys)")
            .clicked()
        {
            app.apply_lighting();
        }
        ui.checkbox(&mut app.auto_apply, "Auto-apply")
            .on_hover_text("Send lighting changes about a second after you stop editing");
    });
    hint(
        ui,
        "Apply updates only the lighting; the keypad stores it right away (it survives unplugging). Save writes the whole layer.",
    );
    hint(ui, "The on-screen preview only approximates the effects.");
}

fn custom_editor(ui: &mut egui::Ui, app: &mut FalconApp, l: &mut falcon_core::Lighting) {
    heading(ui, "Per-key colours");
    hint(ui, "Click keys or light bars on the keypad to paint them.");
    ui.horizontal(|ui| {
        ui.label("Paint");
        color_row(ui, "paint", &mut app.paint);
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Fill keys").clicked() {
            l.key_colors = [app.paint; 20];
        }
        if ui.button("Fill bars").clicked() {
            l.bar_colors = [app.paint; 4];
        }
        if ui.button("Clear all").clicked() {
            l.key_colors = [Rgb::BLACK; 20];
            l.bar_colors = [Rgb::BLACK; 4];
        }
        if ui.button("Windows defaults").clicked() {
            let d = falcon_core::Lighting::default_for_profile(app.active());
            l.key_colors = d.key_colors;
            l.bar_colors = d.bar_colors;
        }
    });
    let selected = match app.target {
        Some(Target::Key(k)) => Some((format!("Key {}", k + 1), &mut l.key_colors[k])),
        Some(Target::Bar(b)) => Some((
            format!("Light bar {}", LightBar::ALL[b].short_name()),
            &mut l.bar_colors[b],
        )),
        None => None,
    };
    if let Some((name, c)) = selected {
        ui.horizontal(|ui| {
            ui.label(format!("{name}:"));
            color_row(ui, "selected", c);
        });
        if ui.small_button("Use as paint colour").clicked() {
            app.paint = *c;
        }
    }
}
