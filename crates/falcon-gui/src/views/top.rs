//! Toolbar: layer (profile) bar, save buttons and the simulator switch.

use eframe::egui::{self, Button, RichText};
use falcon_core::protocol::PROFILE_COUNT;

use crate::app::FalconApp;
use crate::theme;
use crate::worker::Mode;

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Falcon RGB").strong().size(17.0));
        ui.add_space(12.0);
        layer_bar(ui, app);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut sim = app.mode == Mode::Simulated;
            if ui
                .toggle_value(&mut sim, "Simulated")
                .on_hover_text("Use an in-memory keypad instead of USB (also: --simulate)")
                .changed()
            {
                app.set_mode(if sim { Mode::Simulated } else { Mode::Hardware });
            }
            ui.separator();
            let all_dirty = (0..PROFILE_COUNT).any(|p| app.layer_dirty(p));
            if ui
                .add_enabled(app.recording.is_none(), Button::new("Write all layers"))
                .on_hover_text("Save the file and download all five layers to the keypad")
                .clicked()
            {
                app.save(true);
            }
            let dirty = app.file_dirty() || app.layer_dirty(app.active());
            let save = Button::new(RichText::new("Save").strong().color(if dirty {
                egui::Color32::WHITE
            } else {
                ui.visuals().text_color()
            }))
            .fill(if dirty {
                theme::ACCENT
            } else {
                ui.visuals().widgets.inactive.weak_bg_fill
            });
            if ui
                .add_enabled(app.recording.is_none(), save)
                .on_hover_text(
                    "Save the config file and download the current layer to the keypad (⌘S)",
                )
                .clicked()
            {
                app.save(false);
            }
            ui.separator();
            if ui
                .add_enabled(app.can_redo(), Button::new("Redo"))
                .on_hover_text("⌘⇧Z / Ctrl+Y")
                .clicked()
            {
                app.redo();
            }
            if ui
                .add_enabled(app.can_undo(), Button::new("Undo"))
                .on_hover_text("⌘Z / Ctrl+Z")
                .clicked()
            {
                app.undo();
            }
            if all_dirty && !dirty {
                ui.label(RichText::new("other layers changed").small().weak());
            }
        });
    });
    ui.add_space(4.0);
}

fn layer_bar(ui: &mut egui::Ui, app: &mut FalconApp) {
    let active = app.active();
    for p in 0..PROFILE_COUNT {
        let name = app.config.profiles[p].name.clone();
        let mut text = RichText::new(format!("{}  {}", p + 1, name));
        if p == active {
            text = text.strong();
        }
        let dirty = app.layer_dirty(p);
        let on_device = app.device_profile == Some(p);
        let btn = Button::selectable(p == active, text).min_size(egui::vec2(96.0, 28.0));
        let resp = ui.add(btn);
        // Dot: layer differs from the keypad; underline: the keypad's active layer.
        if dirty {
            let r = resp.rect;
            ui.painter()
                .circle_filled(r.right_top() + egui::vec2(-6.0, 6.0), 3.5, theme::WARN);
        }
        if on_device {
            let r = resp.rect;
            ui.painter().line_segment(
                [
                    r.left_bottom() + egui::vec2(8.0, 1.0),
                    r.right_bottom() + egui::vec2(-8.0, 1.0),
                ],
                egui::Stroke::new(2.0, theme::OK),
            );
        }
        let resp = resp.on_hover_text(format!(
            "Layer {}{}{}",
            p + 1,
            if on_device {
                " · active on the keypad"
            } else {
                ""
            },
            if dirty {
                " · changes not on the keypad yet"
            } else {
                ""
            }
        ));
        if resp.clicked() && p != active {
            app.select_layer(p);
        }
    }
}
