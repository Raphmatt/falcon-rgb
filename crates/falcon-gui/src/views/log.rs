//! Packet and activity log: every feature report sent/received and every input report, as hex.

use eframe::egui::{self, Color32, RichText};

use crate::app::{FalconApp, LogKind, LogLine};
use crate::theme;

fn kind_tag(kind: LogKind) -> (&'static str, Color32) {
    match kind {
        LogKind::Set => ("SET", theme::ACCENT),
        LogKind::Get => ("GET", theme::OK),
        LogKind::Input => ("IN ", Color32::from_rgb(0xBF, 0x5A, 0xF2)),
        LogKind::Info => ("·  ", Color32::GRAY),
        LogKind::Error => ("ERR", theme::ERR),
    }
}

fn line_text(l: &LogLine) -> String {
    let (tag, _) = kind_tag(l.kind);
    if l.hex.is_empty() {
        format!("{:>9.3}s {tag} {}", l.at, l.text)
    } else {
        format!("{:>9.3}s {tag} {}\n            {}", l.at, l.text, l.hex)
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Packet log").strong());
        ui.label(RichText::new(format!("{} lines", app.log.len())).weak());
        ui.checkbox(&mut app.log_hide_get, "Hide GET replies");
        if ui.button("Copy").clicked() {
            let text: Vec<String> = app.log.iter().map(line_text).collect();
            ui.ctx().copy_text(text.join("\n"));
        }
        if ui.button("Clear").clicked() {
            app.log.clear();
        }
    });
    ui.separator();
    let mono = egui::TextStyle::Monospace;
    let row_h = ui.text_style_height(&mono);
    let hide_get = app.log_hide_get;
    let lines: Vec<&LogLine> = app
        .log
        .iter()
        .filter(|l| !(hide_get && l.kind == LogKind::Get))
        .collect();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, row_h * 2.0 + 2.0, lines.len(), |ui, range| {
            for l in &lines[range] {
                let (tag, color) = kind_tag(l.kind);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:>9.3}s", l.at)).monospace().weak());
                    ui.label(RichText::new(tag).monospace().color(color));
                    ui.vertical(|ui| {
                        let text = RichText::new(&l.text).monospace();
                        ui.label(if l.kind == LogKind::Error {
                            text.color(theme::ERR)
                        } else {
                            text
                        });
                        let hex = if l.hex.is_empty() {
                            " "
                        } else {
                            l.hex.as_str()
                        };
                        ui.label(RichText::new(hex).monospace().small().weak());
                    });
                });
            }
        });
}
