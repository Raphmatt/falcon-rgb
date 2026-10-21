//! Handling of worker updates, keypad events and frontmost-app changes.

use falcon_core::protocol::{LONG_LEN, describe_request};
use falcon_core::{
    DeviceEvent, Effect, InputReport, KeyAssignment, PacketDir, PacketRecord, WaveDirection, hex,
    layout,
};

use super::{FalconApp, LogKind, PERMISSION_TEXT};
use crate::autoswitch;
use crate::host;
use crate::worker::{Connection, Job, Update};

/// Hex of the bytes up to the last non-zero one, plus a note about the cut zeros.
pub fn trimmed_hex(data: &[u8]) -> String {
    let used = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let mut s = hex::to_hex(&data[..used.max(2).min(data.len())]);
    if used.max(2) < data.len() {
        s.push_str(&format!("  … zeros to {}", data.len()));
    }
    s
}

pub fn packet_line(r: &PacketRecord) -> (LogKind, String) {
    match r.dir {
        PacketDir::SetFeature => (LogKind::Set, describe_request(&r.data)),
        PacketDir::GetFeature => {
            let cmd = r.data.get(1).copied().unwrap_or(0);
            let len = r.data.len();
            let note = if len == LONG_LEN || len == 8 {
                String::new()
            } else {
                " (unexpected length)".into()
            };
            (
                LogKind::Get,
                format!("reply to {cmd:#04x}, {len} bytes{note}"),
            )
        }
        PacketDir::Input => (
            LogKind::Input,
            match falcon_core::protocol::parse_input_report(&r.data) {
                Some(ev) => format!("input: {ev:?}"),
                None => "input: unrecognised report".into(),
            },
        ),
    }
}

impl FalconApp {
    pub(super) fn process_updates(&mut self) {
        let updates: Vec<Update> = self.worker.try_iter().collect();
        for u in updates {
            self.handle_update(u);
        }
    }

    fn handle_update(&mut self, u: Update) {
        match u {
            Update::Connection(c) => {
                match &c {
                    Connection::Connected {
                        simulated,
                        firmware,
                        ..
                    } => self.info(format!(
                        "{} connected, firmware {}",
                        if *simulated {
                            "Simulated keypad"
                        } else {
                            "Keypad"
                        },
                        firmware.map_or("not available".into(), |f| f.to_string())
                    )),
                    Connection::Searching if self.conn.is_connected() => {
                        self.info("Keypad disconnected")
                    }
                    Connection::Failed(e) => self.error(format!("Keypad: {e}")),
                    Connection::PermissionDenied => self.error(PERMISSION_TEXT),
                    Connection::Searching => {}
                }
                if !c.is_connected() {
                    self.device_profile = None;
                }
                self.conn = c;
            }
            Update::ActiveProfile(p) => {
                self.device_profile = Some(p);
                if p != self.config.active_profile {
                    self.stop_recording();
                    self.config.active_profile = p;
                }
            }
            Update::Event(report) => self.handle_event(report),
            Update::Packets(records) => {
                for r in records {
                    let (kind, text) = packet_line(&r);
                    self.push_log(kind, text, trimmed_hex(&r.data));
                }
            }
            Update::Progress { done, total, what } => self.progress = Some((done, total, what)),
            Update::Written(p, profile) => self.mark_written(p, *profile),
            Update::Done { job, result } => {
                self.busy = self.busy.saturating_sub(1);
                if matches!(job, Job::ApplyLighting(_)) {
                    self.finish_apply(result.is_ok());
                }
                if matches!(job, Job::Write(_)) {
                    self.progress = None;
                }
                match (job, result) {
                    (Job::Write(ps), Ok(())) => self.info(match ps.as_slice() {
                        [p] => format!("Saved; layer {} written to the keypad", p + 1),
                        _ => format!("Saved; {} layers written to the keypad", ps.len()),
                    }),
                    (Job::ApplyLighting(p), Ok(())) => {
                        self.info(format!("Lighting of layer {} applied", p + 1))
                    }
                    (_, Ok(())) => {}
                    (job, Err(e)) => self.error(format!("{job:?} failed: {e}")),
                }
            }
        }
    }

    fn handle_event(&mut self, report: InputReport) {
        let Some(event) = report.event else {
            return;
        };
        let p = self.device_profile.unwrap_or(self.active());
        let lighting_change = |app: &mut Self, f: &dyn Fn(&mut falcon_core::Lighting)| {
            f(&mut app.config.profiles[p].lighting);
            f(&mut app.device_snapshot[p].lighting);
        };
        match event {
            DeviceEvent::EffectChanged { effect } => {
                lighting_change(self, &|l| l.effect = Effect::from_byte(effect));
                self.info(format!(
                    "Keypad changed the effect to {}",
                    Effect::from_byte(effect)
                ));
            }
            DeviceEvent::BrightnessChanged { brightness } => {
                lighting_change(self, &|l| l.brightness = brightness.clamp(1, 5));
                self.info(format!("Keypad changed the brightness to {brightness}"));
            }
            DeviceEvent::DirectionChanged { direction } => {
                // Raw byte; whether the event uses the block encoding is [HW-VERIFY].
                lighting_change(self, &|l| l.direction = WaveDirection::from_byte(direction));
                self.info(format!("Keypad changed the wave direction ({direction})"));
            }
            DeviceEvent::ProfileChanged { profile } => {
                self.info(format!("Keypad switched to layer {}", profile + 1));
            }
            DeviceEvent::HostKey { hw } => self.run_host_key(p, hw),
            DeviceEvent::HostCode { code } => {
                // Windows re-reads 0x81 here (profile functions); do the same.
                self.info(format!("Keypad reported host code {code:#04x}"));
                self.refresh();
            }
            // Echoes of our own commands and mouse reports: already in the packet log.
            DeviceEvent::CommandAck { .. } | DeviceEvent::MouseReport => {}
        }
    }

    /// A host-reporting key was pressed: run its launch-program assignment, if any.
    fn run_host_key(&mut self, p: usize, hw: u8) {
        let Some(k) = layout::hw_to_k(hw) else {
            self.error(format!("Host key event for unknown hw index {hw}"));
            return;
        };
        match &self.config.profiles[p].keys[k] {
            KeyAssignment::LaunchProgram(path) => {
                let path = path.clone();
                match host::launch(&path) {
                    Ok(()) => self.info(format!("Key {}: launched {path}", k + 1)),
                    Err(e) => self.error(format!("Key {}: cannot launch {path}: {e}", k + 1)),
                }
            }
            other => {
                let label = other.label();
                self.info(format!(
                    "Key {} reported a host event; its assignment ({label}) has no host action",
                    k + 1
                ));
            }
        }
    }

    pub(super) fn process_front_apps(&mut self) {
        let apps: Vec<_> = self.watcher.try_iter().collect();
        for app in apps {
            if self.config.settings.auto_switch
                && let Some(p) = autoswitch::match_profile(&self.config.profiles, &app)
                && p != self.active()
            {
                self.select_layer(p);
                self.info(format!("{} is in front: layer {}", app.name, p + 1));
            }
            self.front_app = Some(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn hex_is_trimmed() {
        let mut d = vec![0u8; 264];
        d[..3].copy_from_slice(&[0x07, 0x82, 0x01]);
        assert_eq!(trimmed_hex(&d), "07 82 01  … zeros to 264");
        assert_eq!(trimmed_hex(&[0x07, 0x80]), "07 80");
        assert_eq!(trimmed_hex(&[0, 0, 0]), "00 00  … zeros to 3");
        assert_eq!(trimmed_hex(&[]), "");
    }

    #[test]
    fn packet_lines_describe_direction() {
        let rec = |dir, data: Vec<u8>| PacketRecord {
            at: Duration::ZERO,
            dir,
            data,
            sent: true,
        };
        let (k, t) = packet_line(&rec(PacketDir::SetFeature, vec![7, 0x06, 0, 0, 0]));
        assert_eq!((k, t.as_str()), (LogKind::Set, "0x06 commit"));
        let (k, t) = packet_line(&rec(PacketDir::GetFeature, vec![7, 0x80, 1, 0, 0, 0, 0, 0]));
        assert_eq!(k, LogKind::Get);
        assert!(t.contains("0x80") && t.contains("8 bytes"));
        let (k, t) = packet_line(&rec(PacketDir::Input, vec![0x04, 0x01, 0x07, 0x02]));
        assert_eq!(k, LogKind::Input);
        assert!(t.contains("BrightnessChanged"));
        let (_, t) = packet_line(&rec(PacketDir::Input, vec![0x04, 0x1F, 0x81, 0x00, 0x02]));
        assert!(t.contains("CommandAck"), "{t}");
    }
}
