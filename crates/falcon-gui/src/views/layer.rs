//! Layer settings: name, reset, import/export, linked apps (auto switch), simulator tools.

use eframe::egui::{self, RichText};
use falcon_core::Profile;
use falcon_core::protocol::PROFILE_COUNT;

use super::{heading, hint};
use crate::app::FalconApp;
use crate::theme;
use crate::worker::{Mode, SimAction};
use crate::{autoswitch, files};

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
            import_layer(ui, app);
        }
    });

    heading(ui, "Whole keypad");
    hint(
        ui,
        "A backup holds every layer as stored on the keypad (keys and lighting) plus the macros this app knows. Restore writes it all back.",
    );
    ui.horizontal_wrapped(|ui| {
        let connected = app.connected();
        if ui
            .add_enabled(connected, egui::Button::new("Back up keypad…"))
            .clicked()
        {
            app.backup_keypad();
        }
        if ui
            .add_enabled(connected, egui::Button::new("Restore backup…"))
            .on_hover_text("A .json backup, or a raw layer dump (.bin) from falcon dump --out")
            .clicked()
        {
            let ctx = ui.ctx().clone();
            app.restore_inbox
                .open("Restore backup", &["json", "bin"], move || {
                    ctx.request_repaint()
                });
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
        if files::CAN_PICK_PROGRAMS
            && ui.button("Browse…").clicked()
            && let Some(path) = files::pick_program("App that selects this layer")
        {
            app.profile_mut().linked_apps.push(path);
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
    let result = serde_json::to_string_pretty(&profile)
        .map_err(|e| e.to_string())
        .and_then(|json| {
            let name = format!("{}.falcon-layer.json", profile.name);
            files::save_text("Export layer", &name, &json)
        });
    match result {
        Ok(Some(place)) => app.info(format!("Layer exported to {place}")),
        Ok(None) => {}
        Err(e) => app.error(format!("Export failed: {e}")),
    }
}

/// Opens the picker; [`finish_import`] applies the file once it arrives.
fn import_layer(ui: &egui::Ui, app: &mut FalconApp) {
    let ctx = ui.ctx().clone();
    app.import_inbox
        .open("Import layer", &["json"], move || ctx.request_repaint());
}

/// Applies a picked layer file to the active layer (called every frame by the app).
pub fn finish_import(app: &mut FalconApp) {
    let Some(picked) = app.import_inbox.take() else {
        return;
    };
    let result = picked.and_then(|(name, data)| {
        let p = serde_json::from_slice::<Profile>(&data).map_err(|e| e.to_string())?;
        p.validate().map_err(|e| e.to_string())?;
        Ok((name, p))
    });
    match result {
        Ok((name, profile)) => {
            *app.profile_mut() = profile;
            app.lighting_edited();
            app.info(format!(
                "Imported {name} into layer {} (not saved yet)",
                app.active() + 1
            ));
        }
        Err(e) => app.error(format!("Import failed: {e}")),
    }
}

/// Confirmation dialog for "Reset layer".
/// Parses a picked backup file; a valid one waits in [`restore_dialog`] for confirmation.
pub fn finish_restore_pick(app: &mut FalconApp) {
    let Some(picked) = app.restore_inbox.take() else {
        return;
    };
    match picked.and_then(|(name, data)| {
        falcon_core::Backup::from_bytes(&data)
            .map(|b| (name, b))
            .map_err(|e| e.to_string())
    }) {
        Ok(pending) => app.pending_restore = Some(pending),
        Err(e) => app.error(format!("Not a usable backup: {e}")),
    }
}

pub fn restore_dialog(ui: &mut egui::Ui, app: &mut FalconApp) {
    let Some((name, backup)) = &app.pending_restore else {
        return;
    };
    let lines: Vec<String> = backup.layers.iter().map(|l| l.summary()).collect();
    let title = format!("Restore {name}?");
    let mut decision = None;
    let modal = egui::Modal::new(egui::Id::unique("restore-backup")).show(ui.ctx(), |ui| {
        ui.set_width(460.0);
        ui.heading(title);
        ui.label("These layers on the keypad are overwritten with the backup:");
        ui.add_space(4.0);
        for line in &lines {
            ui.label(RichText::new(line).small());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let go = egui::Button::new(RichText::new("Restore").color(egui::Color32::WHITE))
                .fill(theme::ERR);
            if ui.add(go).clicked() {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
    });
    if modal.should_close() && decision.is_none() {
        decision = Some(false);
    }
    match decision {
        Some(true) => app.restore_confirmed(),
        Some(false) => app.pending_restore = None,
        None => {}
    }
}

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
