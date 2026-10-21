//! Status bar: connection, firmware, device layer, progress, last message, log toggle.

use eframe::egui::{self, RichText};

use crate::app::{FalconApp, PERMISSION_TEXT};
use crate::worker::{Connection, Mode};
use crate::{host, theme};

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp) {
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        let (dot, text, tip) = match &app.conn {
            Connection::Connected {
                simulated: true, ..
            } => (theme::ACCENT, "Simulated keypad".to_owned(), String::new()),
            Connection::Connected { detail, .. } => {
                (theme::OK, "Keypad connected".to_owned(), detail.clone())
            }
            Connection::Searching if app.mode == Mode::Hardware => (
                theme::WARN,
                "Keypad not found — plug it in (re-scanning)".to_owned(),
                String::new(),
            ),
            Connection::Searching => (theme::WARN, "Connecting…".to_owned(), String::new()),
            Connection::Failed(e) => (theme::ERR, "Keypad error".to_owned(), e.clone()),
            Connection::PermissionDenied => (
                theme::ERR,
                "No permission to open the keypad".to_owned(),
                PERMISSION_TEXT.to_owned(),
            ),
        };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.5, dot);
        let resp = ui.label(text);
        if !tip.is_empty() {
            resp.on_hover_text(tip);
        }
        if let Connection::Connected { firmware, .. } = &app.conn {
            ui.separator();
            match firmware {
                Some(f) => ui.label(format!("Firmware {f}")),
                None => ui
                    .label("Firmware n/a")
                    .on_hover_text("The keypad does not answer the 264-byte version query (0x80)"),
            };
            if let Some(p) = app.device_profile {
                ui.separator();
                ui.label(format!("Keypad on layer {}", p + 1))
                    .on_hover_text("From the 0x81 query or the last layer switch");
            }
            if ui
                .small_button("Refresh")
                .on_hover_text("Re-read firmware version and active layer")
                .clicked()
            {
                app.refresh();
            }
        }
        if let Some((done, total, what)) = &app.progress {
            ui.separator();
            ui.add(
                egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                    .desired_width(120.0)
                    .text(what.clone()),
            );
        } else if app.busy > 0 {
            ui.separator();
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if app.show_log {
                "Hide log"
            } else {
                "Packet log"
            };
            if ui.selectable_label(app.show_log, label).clicked() {
                app.show_log = !app.show_log;
            }
            egui::widgets::global_theme_preference_buttons(ui);
            if app.file_dirty() {
                ui.label(RichText::new("● unsaved changes").color(theme::WARN));
            }
            if let Some((is_err, msg)) = &app.status {
                let color = if *is_err {
                    theme::ERR
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.add(egui::Label::new(RichText::new(msg).color(color)).truncate());
            }
        });
    });
    ui.add_space(2.0);
}

/// Banner under the toolbar while macOS denies access to the keypad.
pub fn permission_banner(ui: &mut egui::Ui, app: &mut FalconApp) {
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(PERMISSION_TEXT).color(theme::ERR));
        if cfg!(target_os = "macos")
            && ui.button("Open Input Monitoring settings").clicked()
            && let Err(e) = host::open_input_monitoring_settings()
        {
            app.error(format!("Cannot open System Settings: {e}"));
        }
    });
    ui.add_space(4.0);
}
