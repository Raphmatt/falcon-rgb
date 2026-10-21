//! Macro editor: event list, repeat mode, insert tools and in-app recording.

use eframe::egui::{self, RichText};
use falcon_core::protocol::{MAX_DELAY_10MS, MAX_MACRO_EVENTS};
use falcon_core::{KeyAssignment, MacroEvent, Repeat};

use super::{hint, picker};
use crate::app::FalconApp;
use crate::theme;

/// Insert-row state, kept in egui's temp memory.
#[derive(Debug, Clone, Copy)]
struct Insert {
    code: u8,
    delay_ms: u32,
    bulk_ms: u32,
}

impl Default for Insert {
    fn default() -> Self {
        Self {
            code: 0x04,
            delay_ms: 50,
            bulk_ms: 20,
        }
    }
}

enum Action {
    StartRecording,
    StopRecording,
}

fn ms_to_units(ms: u32) -> u16 {
    (ms / 10).clamp(1, u32::from(MAX_DELAY_10MS)) as u16
}

/// Delay editor in ms (10 ms steps, 10..=327 670 ms).
fn delay_ms(ui: &mut egui::Ui, units: &mut u16) {
    let mut ms = u32::from(*units) * 10;
    let resp = ui.add(
        egui::DragValue::new(&mut ms)
            .range(10..=327_670)
            .speed(10)
            .suffix(" ms"),
    );
    if resp.changed() {
        *units = ms_to_units(ms);
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut FalconApp, k: usize) {
    let p = app.active();
    let recording = app.recording.is_some_and(|r| r.key == k && r.profile == p);
    let insert_id = ui.scope_id().with(("macro-insert", p, k));
    let mut ins: Insert = ui.data_mut(|d| *d.get_temp_mut_or_default(insert_id));
    let mut action = None;

    let KeyAssignment::Macro(m) = &mut app.config.profiles[p].keys[k] else {
        return;
    };

    ui.horizontal(|ui| {
        if recording {
            let stop = egui::Button::new(RichText::new("Stop").color(egui::Color32::WHITE))
                .fill(theme::ERR);
            if ui.add(stop).clicked() {
                action = Some(Action::StopRecording);
            }
            let t = ui.input(|i| i.time);
            let blink = if (t * 2.0) as i64 % 2 == 0 {
                "●"
            } else {
                "○"
            };
            ui.label(RichText::new(format!("{blink} recording key presses")).color(theme::ERR));
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        } else if ui
            .button("● Record")
            .on_hover_text("Records key presses in this window (appends to the list)")
            .clicked()
        {
            action = Some(Action::StartRecording);
        }
        let n = m.events.len();
        let color = if n > MAX_MACRO_EVENTS {
            theme::ERR
        } else {
            ui.visuals().weak_text_color()
        };
        ui.label(RichText::new(format!("{n} / {MAX_MACRO_EVENTS} events")).color(color));
        if ui.add_enabled(n > 0, egui::Button::new("Clear")).clicked() {
            m.events.clear();
        }
    });
    if recording {
        hint(
            ui,
            "Numpad keys record as digit-row keys; media keys and OS shortcuts (⌘Q, ⌘Tab) can't be recorded.",
        );
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Repeat:");
        let mut times = match m.repeat {
            Repeat::Times(n) => n.max(1),
            _ => 1,
        };
        if ui
            .radio(matches!(m.repeat, Repeat::Times(_)), "times")
            .clicked()
        {
            m.repeat = Repeat::Times(times);
        }
        if ui
            .add(egui::DragValue::new(&mut times).range(1..=0xFFFE))
            .changed()
        {
            m.repeat = Repeat::Times(times);
        }
        if ui
            .radio(m.repeat == Repeat::WhileHeld, "while held")
            .clicked()
        {
            m.repeat = Repeat::WhileHeld;
        }
        if ui
            .radio(m.repeat == Repeat::Toggle, "until pressed again")
            .clicked()
        {
            m.repeat = Repeat::Toggle;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        picker::macro_code_combo(ui, insert_id.with("code"), &mut ins.code);
        let mut units = ms_to_units(ins.delay_ms);
        delay_ms(ui, &mut units);
        ins.delay_ms = u32::from(units) * 10;
        let d = units;
        let room = MAX_MACRO_EVENTS.saturating_sub(m.events.len());
        if ui
            .add_enabled(room >= 2, egui::Button::new("+ Tap"))
            .clicked()
        {
            m.events.push(MacroEvent::down(ins.code, d));
            m.events.push(MacroEvent::up(ins.code, d));
        }
        if ui
            .add_enabled(room >= 1, egui::Button::new("+ Press"))
            .clicked()
        {
            m.events.push(MacroEvent::down(ins.code, d));
        }
        if ui
            .add_enabled(room >= 1, egui::Button::new("+ Release"))
            .clicked()
        {
            m.events.push(MacroEvent::up(ins.code, d));
        }
    });
    ui.horizontal(|ui| {
        let mut units = ms_to_units(ins.bulk_ms);
        if ui.button("Set all delays to").clicked() {
            for e in &mut m.events {
                e.delay_10ms = units;
            }
        }
        delay_ms(ui, &mut units);
        ins.bulk_ms = u32::from(units) * 10;
    });

    ui.add_space(4.0);
    event_list(ui, &mut m.events, (p, k));
    ui.data_mut(|d| d.insert_temp(insert_id, ins));

    match action {
        Some(Action::StartRecording) => app.start_recording(k),
        Some(Action::StopRecording) => app.stop_recording(),
        None => {}
    }
}

fn event_list(ui: &mut egui::Ui, events: &mut Vec<MacroEvent>, salt: (usize, usize)) {
    if events.is_empty() {
        hint(ui, "No events yet: record, or add taps above.");
        return;
    }
    let mut remove = None;
    let mut swap = None;
    let len = events.len();
    egui::ScrollArea::vertical()
        .id_salt(("macro-events", salt))
        .max_height(340.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new(("macro-grid", salt))
                .striped(true)
                .num_columns(5)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    for (i, e) in events.iter_mut().enumerate() {
                        ui.label(RichText::new(format!("{:>3}", i + 1)).monospace().weak());
                        let dir = if e.down { "↓ press" } else { "↑ release" };
                        if ui
                            .add(egui::Button::new(dir).min_size(egui::vec2(78.0, 0.0)))
                            .on_hover_text("Toggle press / release")
                            .clicked()
                        {
                            e.down = !e.down;
                        }
                        picker::macro_code_combo(ui, ("ev-code", salt, i), &mut e.code);
                        delay_ms(ui, &mut e.delay_10ms);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(i > 0, egui::Button::new("⏶").small())
                                .clicked()
                            {
                                swap = Some((i - 1, i));
                            }
                            if ui
                                .add_enabled(i + 1 < len, egui::Button::new("⏷").small())
                                .clicked()
                            {
                                swap = Some((i, i + 1));
                            }
                            if ui.add(egui::Button::new("x").small()).clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    hint(ui, "Delay = wait before the event, in 10 ms steps.");
    if let Some((a, b)) = swap {
        events.swap(a, b);
    }
    if let Some(i) = remove {
        events.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_conversion_clamps() {
        assert_eq!(ms_to_units(0), 1);
        assert_eq!(ms_to_units(50), 5);
        assert_eq!(ms_to_units(327_670), 0x7FFF);
        assert_eq!(ms_to_units(u32::MAX), 0x7FFF);
    }
}
