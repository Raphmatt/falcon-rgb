//! Layer settings: name, reset, import/export, linked apps (auto switch), simulator tools.

use eframe::egui::{self, RichText};
use falcon_core::Profile;
use falcon_core::protocol::PROFILE_COUNT;

use super::{heading, hint};
use crate::app::FalconApp;
use crate::autoswitch;
use crate::theme;
use crate::worker::{Mode, SimAction};

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp) {
    let p = app.active();
    heading(ui, &format!("Layer {}", p + 1));
    ui.horizontal(|ui| {
        ui.label("Name");
        let mut name = app.profile().name.clone();
        if ui
            .add(egui::TextEdit::singleline(&mut name).desired_width(200.0))
            .changed()
        {
            app.profile_mut().name = name;
        }
    });
    hint(
        ui,
        "Names are stored in the config file only, not on the keypad.",
    );
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Reset layer…").clicked() {
            app.confirm_reset = true;
        }
        if ui.button("Export layer…").clicked() {
            export_layer(app);
        }
        if ui.button("Import layer…").clicked() {
            import_layer(app);
        }
    });

    linked_apps(ui, app);

    if app.mode == Mode::Simulated {
        heading(ui, "Simulated keypad");
        hint(
            ui,
            "Fake what the hardware would report, to try the app without a keypad.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Press layer key:");
            for q in 0..PROFILE_COUNT {
                if ui.small_button(format!("L{}", q + 1)).clicked() {
                    app.simulate(SimAction::PressLayer(q));
                }
            }
        });
        if ui
            .button("Send demo events")
            .on_hover_text("One example of every input report (spec §7)")
            .clicked()
        {
            app.simulate(SimAction::DemoEvents);
        }
    }
}

fn linked_apps(ui: &mut egui::Ui, app: &mut FalconApp) {
    heading(ui, "Switch to this layer for apps");
    let mut auto = app.config.settings.auto_switch;
    if ui
        .add_enabled(
            autoswitch::SUPPORTED,
            egui::Checkbox::new(&mut auto, "Automatic layer switching (all layers)"),
        )
        .changed()
    {
        app.config.settings.auto_switch = auto;
    }
    if !autoswitch::SUPPORTED {
        hint(ui, "Not available on this platform yet.");
    }
    hint(
        ui,
        "When one of these apps comes to the front, Falcon RGB selects this layer. Enter an app name (Safari) or its path.",
    );
    let mut remove = None;
    for (i, entry) in app.profile().linked_apps.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(entry).monospace());
            if ui.small_button("x").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        app.profile_mut().linked_apps.remove(i);
    }
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut app.new_linked_app)
                .hint_text("App name or path")
                .desired_width(180.0),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let entry = app.new_linked_app.trim().to_owned();
        if (ui.button("Add").clicked() || enter) && !entry.is_empty() {
            app.profile_mut().linked_apps.push(entry);
            app.new_linked_app.clear();
        }
        if ui.button("Browse…").clicked() {
            let mut dlg = rfd::FileDialog::new().set_title("App that selects this layer");
            if cfg!(target_os = "macos") {
                dlg = dlg.set_directory("/Applications");
            }
            if let Some(path) = dlg.pick_file() {
                app.profile_mut()
                    .linked_apps
                    .push(path.display().to_string());
            }
        }
    });
    if let Some(front) = &app.front_app {
        hint(
            ui,
            &format!("Frontmost app: {} ({})", front.name, front.path),
        );
    }
}

fn export_layer(app: &mut FalconApp) {
    let profile = app.profile().clone();
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export layer")
        .set_file_name(format!("{}.falcon-layer.json", profile.name))
        .add_filter("Falcon layer", &["json"])
        .save_file()
    else {
        return;
    };
    let result = serde_json::to_string_pretty(&profile)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
    match result {
        Ok(()) => app.info(format!("Layer exported to {}", path.display())),
        Err(e) => app.error(format!("Export failed: {e}")),
    }
}

fn import_layer(app: &mut FalconApp) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Import layer")
        .add_filter("Falcon layer", &["json"])
        .pick_file()
    else {
        return;
    };
    let result = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str::<Profile>(&s).map_err(|e| e.to_string()))
        .and_then(|p| p.validate().map(|()| p).map_err(|e| e.to_string()));
    match result {
        Ok(profile) => {
            *app.profile_mut() = profile;
            app.lighting_edited();
            app.info(format!(
                "Imported {} into layer {} (not saved yet)",
                path.display(),
                app.active() + 1
            ));
        }
        Err(e) => app.error(format!("Import failed: {e}")),
    }
}

/// Confirmation dialog for "Reset layer".
pub fn reset_dialog(ui: &mut egui::Ui, app: &mut FalconApp) {
    if !app.confirm_reset {
        return;
    }
    let p = app.active();
    let modal = egui::Modal::new(egui::Id::unique("reset-layer")).show(ui.ctx(), |ui| {
        ui.set_width(320.0);
        ui.heading(format!("Reset layer {}?", p + 1));
        ui.label("All key assignments and lighting of this layer go back to the factory defaults. Nothing is sent until you Save.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let reset = egui::Button::new(RichText::new("Reset").color(egui::Color32::WHITE))
                .fill(theme::ERR);
            if ui.add(reset).clicked() {
                app.reset_layer();
                app.confirm_reset = false;
            }
            if ui.button("Cancel").clicked() {
                app.confirm_reset = false;
            }
        });
    });
    if modal.should_close() {
        app.confirm_reset = false;
    }
}
