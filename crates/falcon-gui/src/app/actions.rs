//! User actions that talk to the worker or change the configuration as a whole.

use std::time::{Duration, Instant};

use eframe::egui;
use falcon_core::protocol::{MAX_MACRO_EVENTS, PROFILE_COUNT};
use falcon_core::{KeyAssignment, Macro, MacroEvent, Profile, store};

use super::{APPLY_DEBOUNCE, FalconApp, Recording, initial_device_snapshot};
use crate::keymap;
use crate::worker::{Command, Connection, Mode, SimAction};

impl FalconApp {
    pub(super) fn send_job(&mut self, cmd: Command) {
        self.busy += 1;
        self.worker.send(cmd);
    }

    pub fn connected(&self) -> bool {
        self.conn.is_connected()
    }

    /// Makes `p` the edited layer and switches the keypad to it.
    pub fn select_layer(&mut self, p: usize) {
        if p >= PROFILE_COUNT {
            return;
        }
        self.stop_recording();
        self.config.active_profile = p;
        if self.connected() {
            self.send_job(Command::SelectProfile(p));
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.conn = Connection::Searching;
        self.device_profile = None;
        self.device_snapshot = initial_device_snapshot(mode, &self.config);
        self.worker.send(Command::SetMode(mode));
        self.info(match mode {
            Mode::Simulated => "Switched to the simulated keypad",
            Mode::Hardware => "Looking for the keypad over USB…",
        });
    }

    pub fn refresh(&mut self) {
        self.send_job(Command::Refresh);
    }

    pub fn simulate(&mut self, action: SimAction) {
        self.send_job(Command::Simulate(action));
    }

    /// Sends the active layer's lighting now (stored by the keypad right away; Save also writes
    /// the keys).
    pub fn apply_lighting(&mut self) {
        self.lighting_changed_at = None;
        if !self.connected() {
            self.error("Keypad not connected");
            return;
        }
        let p = self.active();
        let lighting = self.profile().lighting.clone();
        if let Err(e) = lighting.validate() {
            self.error(format!("Lighting not applied: {e}"));
            return;
        }
        self.applying = Some((p, lighting.clone()));
        self.send_job(Command::ApplyLighting {
            profile: p,
            lighting: Box::new(lighting),
        });
    }

    /// Call after any lighting edit; schedules an auto-apply.
    pub fn lighting_edited(&mut self) {
        if self.auto_apply {
            self.lighting_changed_at = Some(Instant::now());
        }
    }

    /// Fires a debounced auto-apply; returns how long to wait before checking again.
    pub(super) fn auto_apply_tick(&mut self) -> Option<Duration> {
        let since = self.lighting_changed_at?;
        if !self.auto_apply || !self.connected() {
            self.lighting_changed_at = None;
            return None;
        }
        let elapsed = since.elapsed();
        if elapsed < APPLY_DEBOUNCE {
            return Some(APPLY_DEBOUNCE - elapsed);
        }
        if self.applying.is_some() {
            // One apply at a time; try again when it finishes.
            return Some(Duration::from_millis(50));
        }
        // Edited back to what the keypad holds: no flash write needed.
        let p = self.active();
        if self.profile().lighting == self.device_snapshot[p].lighting {
            self.lighting_changed_at = None;
            return None;
        }
        self.apply_lighting();
        None
    }

    pub(super) fn finish_apply(&mut self, ok: bool) {
        if let Some((p, l)) = self.applying.take()
            && ok
        {
            self.device_snapshot[p].lighting = l;
        }
    }

    pub(super) fn mark_written(&mut self, p: usize, profile: Profile) {
        if p < PROFILE_COUNT {
            self.device_snapshot[p] = profile;
        }
    }

    /// "Save": config file, then a full download of the active layer (`all`: every layer).
    pub fn save(&mut self, all: bool) {
        self.stop_recording();
        if let Err(e) = self.config.validate() {
            self.error(format!("Not saved: {e}"));
            return;
        }
        if let Some(path) = self.config_path.clone() {
            if let Err(e) = store::save_config(&path, &self.config) {
                self.error(format!("Could not write {}: {e}", path.display()));
                return;
            }
            self.file_snapshot = self.config.clone();
        }
        if !self.connected() {
            self.info("Saved to file. Keypad not connected; save again once it is.");
            return;
        }
        let layers: Vec<usize> = if all {
            (0..PROFILE_COUNT).collect()
        } else {
            vec![self.active()]
        };
        let list = layers
            .into_iter()
            .map(|p| (p, self.config.profiles[p].clone()))
            .collect();
        self.progress = Some((0, 1, "Writing…".into()));
        self.send_job(Command::WriteProfiles(list));
    }

    /// Restores the active layer's keys and lighting to the Windows defaults (name and linked
    /// apps stay).
    pub fn reset_layer(&mut self) {
        self.stop_recording();
        let p = self.active();
        let fresh = Profile::default_for(p);
        let profile = self.profile_mut();
        profile.keys = fresh.keys;
        profile.lighting = fresh.lighting;
        self.lighting_edited();
        self.info(format!("Layer {} reset to defaults (not saved yet)", p + 1));
    }

    pub fn start_recording(&mut self, key: usize) {
        let profile = self.active();
        let slot = &mut self.config.profiles[profile].keys[key];
        if !matches!(slot, KeyAssignment::Macro(_)) {
            *slot = KeyAssignment::Macro(Macro::default());
        }
        self.recording = Some(Recording {
            profile,
            key,
            last: None,
        });
        self.info("Recording: press keys in this window, then click Stop");
    }

    pub fn stop_recording(&mut self) {
        if self.recording.take().is_some() {
            self.info("Recording stopped");
        }
    }

    /// While recording, turns key events into macro events and hides them from the UI (so
    /// Space or Enter don't also press buttons).
    pub(super) fn capture_recording(&mut self, raw: &mut egui::RawInput) {
        let Some(mut rec) = self.recording else {
            return;
        };
        let now = raw.time.unwrap_or_else(|| self.seconds());
        let mut captured = Vec::new();
        raw.events.retain(|e| match e {
            egui::Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                ..
            } => {
                if !repeat && let Some(code) = keymap::hid_usage(physical_key.unwrap_or(*key)) {
                    captured.push((*pressed, code));
                }
                false
            }
            egui::Event::Text(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => {
                false
            }
            _ => true,
        });
        if captured.is_empty() {
            return;
        }
        let KeyAssignment::Macro(m) = &mut self.config.profiles[rec.profile].keys[rec.key] else {
            self.recording = None;
            return;
        };
        let mut full = false;
        for (down, code) in captured {
            if m.events.len() >= MAX_MACRO_EVENTS {
                full = true;
                break;
            }
            let delay = rec.last.map_or(1, |t| keymap::delay_units(now - t));
            m.events.push(if down {
                MacroEvent::down(code, delay)
            } else {
                MacroEvent::up(code, delay)
            });
            rec.last = Some(now);
        }
        full |= m.events.len() >= MAX_MACRO_EVENTS;
        self.recording = Some(rec);
        if full {
            self.recording = None;
            self.info(format!(
                "Recording stopped: {MAX_MACRO_EVENTS} events is the device limit"
            ));
        }
    }
}
