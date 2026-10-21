//! User actions that talk to the worker or change the configuration as a whole.

use std::time::Duration;

use eframe::egui;
use falcon_core::protocol::{MAX_MACRO_EVENTS, PROFILE_COUNT};
use falcon_core::{KeyAssignment, Macro, MacroEvent, Profile};
use web_time::Instant;

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

    /// Reads every layer from the keypad into a backup file (saved when it arrives).
    pub fn backup_keypad(&mut self) {
        if !self.connected() {
            self.error("Connect the keypad first");
            return;
        }
        self.send_job(Command::Backup(Box::new(self.config.clone())));
    }

    /// Writes the confirmed backup to the keypad.
    pub fn restore_confirmed(&mut self) {
        let Some((_, backup)) = self.pending_restore.take() else {
            return;
        };
        if !self.connected() {
            self.error("Connect the keypad first");
            return;
        }
        self.stop_recording();
        self.send_job(Command::Restore(Box::new(backup)));
    }

    /// A restored layer is on the keypad: the config and the snapshot follow it.
    pub(super) fn mark_restored(&mut self, layer: &falcon_core::LayerBackup) {
        let p = layer.layer - 1;
        if p >= PROFILE_COUNT {
            return;
        }
        let unknown = layer.apply_to_profile(&mut self.config.profiles[p]);
        self.device_snapshot[p] = self.config.profiles[p].clone();
        self.reset_history();
        if !unknown.is_empty() {
            let keys: Vec<String> = unknown.iter().map(ToString::to_string).collect();
            self.info(format!(
                "Layer {}: macro key {} keeps its configured macro (the backup has none for it)",
                p + 1,
                keys.join(", ")
            ));
        }
    }

    /// Browser: let the user pick the keypad (WebHID chooser).
    pub fn pair(&mut self) {
        self.send_job(Command::Pair);
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

    /// Layer `p` as stored on the keypad replaces the configured one: lighting and every key
    /// whose device byte differs ([`falcon_core::LayerBackup::apply_to_profile`]); keys that
    /// already match keep their richer config value. Macro keys whose macro the app does not
    /// know stay as configured and are marked as differing (orange dot), since Save would
    /// replace them.
    pub(super) fn adopt_keypad_block(&mut self, p: usize, block: &falcon_core::protocol::Block) {
        if p >= PROFILE_COUNT {
            return;
        }
        let layer = falcon_core::LayerBackup {
            layer: p + 1,
            block: *block,
            macros: Vec::new(),
        };
        let before = self.config.profiles[p].clone();
        let unknown = layer.apply_to_profile(&mut self.config.profiles[p]);
        if self.config.profiles[p] != before {
            self.reset_history();
        }
        self.device_snapshot[p] = self.config.profiles[p].clone();
        for &key in &unknown {
            // Not the configured value: the layer shows as differing from the keypad.
            self.device_snapshot[p].keys[key - 1] = KeyAssignment::Macro(Macro::default());
        }
        if !unknown.is_empty() {
            let keys: Vec<String> = unknown.iter().map(ToString::to_string).collect();
            self.info(format!(
                "Layer {}: key {} plays a macro on the keypad that cannot be read back; Save replaces it with what the app shows",
                p + 1,
                keys.join(", ")
            ));
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
        match self.config_store.save(&self.config) {
            Ok(true) => self.file_snapshot = self.config.clone(),
            Ok(false) => {}
            Err(e) => {
                self.error(format!("Not saved: {e}"));
                return;
            }
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

    /// Undo/redo shortcuts, unless a text field has focus (it has its own undo) or keystrokes
    /// are being recorded. ⌘Z / Ctrl+Z undo; ⌘⇧Z, ⌘Y, Ctrl+Y and Ctrl+Shift+Z redo.
    pub(super) fn undo_shortcuts(&mut self, ui: &egui::Ui) {
        use egui::{Key, KeyboardShortcut as S, Modifiers as M};
        if self.recording.is_some()
            || self.key_capture.is_some()
            || ui.ctx().egui_wants_keyboard_input()
        {
            return;
        }
        let (undo, redo) = ui.input_mut(|i| {
            // Shifted forms first: consume_shortcut ignores extra Shift on the plain ones.
            let redo = [
                S::new(M::COMMAND | M::SHIFT, Key::Z),
                S::new(M::CTRL | M::SHIFT, Key::Z),
                S::new(M::COMMAND, Key::Y),
                S::new(M::CTRL, Key::Y),
            ]
            .iter()
            .any(|s| i.consume_shortcut(s));
            let undo = !redo
                && [S::new(M::COMMAND, Key::Z), S::new(M::CTRL, Key::Z)]
                    .iter()
                    .any(|s| i.consume_shortcut(s));
            (undo, redo)
        });
        if undo {
            self.undo();
        } else if redo {
            self.redo();
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.has_undo(&self.config.profiles)
    }

    pub fn can_redo(&self) -> bool {
        self.history.has_redo(&self.config.profiles)
    }

    pub fn undo(&mut self) {
        match self.history.undo(&self.config.profiles).cloned() {
            Some(state) => self.apply_history(state, "Undone"),
            None => self.info("Nothing to undo"),
        }
    }

    pub fn redo(&mut self) {
        match self.history.redo(&self.config.profiles).cloned() {
            Some(state) => self.apply_history(state, "Redone"),
            None => self.info("Nothing to redo"),
        }
    }

    fn apply_history(&mut self, state: [Profile; PROFILE_COUNT], what: &str) {
        self.stop_recording();
        self.key_capture = None;
        let lighting =
            (0..PROFILE_COUNT).any(|p| self.config.profiles[p].lighting != state[p].lighting);
        self.config.profiles = state;
        if lighting {
            self.lighting_edited();
        }
        self.info(format!("{what} (not saved yet)"));
    }

    /// The keypad changed the layers (read-back, restore, its own lighting keys): start a new
    /// history, so undo never reverts the app to something the keypad no longer holds.
    pub(super) fn reset_history(&mut self) {
        self.history = super::new_history();
    }

    /// A paint stroke starts: the state before it becomes one undo step.
    pub fn begin_paint_stroke(&mut self) {
        self.history.add_undo(&self.config.profiles);
    }

    /// Paints `t` with the paint colour, if it does not have it yet.
    pub fn paint_target(&mut self, t: super::Target) {
        let paint = self.paint;
        let lighting = &mut self.profile_mut().lighting;
        let slot = match t {
            super::Target::Key(k) => &mut lighting.key_colors[k],
            super::Target::Bar(b) => &mut lighting.bar_colors[b],
        };
        if *slot != paint {
            *slot = paint;
            self.lighting_edited();
        }
    }

    /// "Press a key…": the next key press in the window becomes the key's assignment (with the
    /// held modifiers for a combo). Esc cancels. All key and text input is swallowed meanwhile.
    pub(super) fn capture_key(&mut self, raw: &mut egui::RawInput) {
        let Some(cap) = self.key_capture else {
            return;
        };
        let mut pressed = None;
        raw.events.retain(|e| match e {
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat: false,
                modifiers,
            } => {
                if pressed.is_none() {
                    pressed = Some((physical_key.unwrap_or(*key), *modifiers));
                }
                false
            }
            egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Paste(_) => false,
            _ => true,
        });
        let Some((key, mods)) = pressed else {
            return;
        };
        self.key_capture = None;
        if key == egui::Key::Escape && !mods.any() {
            self.info("Key capture cancelled");
            return;
        }
        let Some(code) = keymap::hid_usage(key) else {
            self.error(format!(
                "{} cannot be assigned here; pick it from the list",
                key.name()
            ));
            return;
        };
        let assignment = if cap.combo {
            let mut codes = Vec::new();
            for (on, m) in [
                (mods.ctrl, 0xE0),
                (mods.shift, 0xE1),
                (mods.alt, 0xE2),
                (mods.mac_cmd, 0xE3),
            ] {
                if on {
                    codes.push(m);
                }
            }
            codes.push(code);
            KeyAssignment::Combo(codes)
        } else {
            KeyAssignment::Key(code)
        };
        let label = assignment.label();
        self.config.profiles[cap.profile].keys[cap.key] = assignment;
        self.info(format!(
            "Layer {} key {} → {label} (not saved yet)",
            cap.profile + 1,
            cap.key + 1
        ));
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
