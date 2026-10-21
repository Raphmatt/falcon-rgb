//! The panels of the main window.

pub mod keypad;
pub mod keys;
pub mod layer;
pub mod lighting;
pub mod log;
pub mod macro_edit;
pub mod picker;
pub mod status;
pub mod top;

use eframe::egui;

use crate::app::{FalconApp, Tab};

/// Right-hand inspector: tabs for keys, lighting and layer settings.
pub fn inspector(ui: &mut egui::Ui, app: &mut FalconApp) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for (tab, label) in [
            (Tab::Keys, "Key assignment"),
            (Tab::Lighting, "Lighting"),
            (Tab::Layer, "Layer"),
        ] {
            if ui.selectable_label(app.tab == tab, label).clicked() {
                app.tab = tab;
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match app.tab {
            Tab::Keys => keys::show(ui, app),
            Tab::Lighting => lighting::show(ui, app),
            Tab::Layer => layer::show(ui, app),
        });
}

/// Section heading used in the inspector.
pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).strong().size(15.0));
    ui.add_space(2.0);
}

/// Small grey explanatory text.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}
