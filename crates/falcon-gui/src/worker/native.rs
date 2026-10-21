//! Desktop worker: a background thread that owns the transport. Every device call blocks for
//! the protocol delays, so the UI only exchanges [`Command`]s and [`Update`]s with it.
//!
//! In hardware mode the worker re-scans for the keypad every [`SCAN_INTERVAL`] (hot-plug) and
//! checks that a connected keypad is still present; in simulated mode it talks to a
//! [`SharedSim`]. Between commands it polls input reports (spec §7).

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use falcon_core::protocol::PROFILE_COUNT;
use falcon_core::transport::hid::{self, HidTransport};
use falcon_core::{
    Backup, DeviceEvent, Error, Falcon, FirmwareVersion, Profile, Recorder, ReportLen, Transport,
};

use super::{Command, Connection, Job, Mode, SharedSim, SimAction, Update};

/// How often to look for the keypad while none is connected.
const SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// How often to retry while macOS denies access: quietly, in case the permission is granted
/// without restarting the app.
const PERMISSION_RETRY: Duration = Duration::from_secs(5);
/// How often to check that a connected keypad is still plugged in.
const PRESENCE_INTERVAL: Duration = Duration::from_secs(3);
/// Wait for commands this long between event polls while connected.
const IDLE_CONNECTED: Duration = Duration::from_millis(15);
const IDLE_DISCONNECTED: Duration = Duration::from_millis(250);
/// Per-iteration input-report read timeout.
const EVENT_POLL: Duration = Duration::from_millis(5);

type Device = Falcon<Recorder<Box<dyn Transport>>>;

/// Owner side of the worker thread. Dropping it shuts the thread down.
pub struct WorkerHandle {
    tx: Sender<Command>,
    rx: Receiver<Update>,
    thread: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    /// Starts the worker; `notify` runs after every batch of updates (e.g. request a repaint).
    pub fn spawn(mode: Mode, notify: impl Fn() + Send + 'static) -> Self {
        let (tx, cmd_rx) = mpsc::channel();
        let (up_tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("falcon-worker".into())
            .spawn(move || Worker::new(mode, cmd_rx, up_tx, Box::new(notify)).run())
            .expect("spawn worker thread");
        Self {
            tx,
            rx,
            thread: Some(thread),
        }
    }

    pub fn send(&self, cmd: Command) {
        // A dead worker has already reported its last state; nothing else to do.
        let _ = self.tx.send(cmd);
    }

    pub fn try_iter(&self) -> impl Iterator<Item = Update> + '_ {
        self.rx.try_iter()
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    mode: Mode,
    rx: Receiver<Command>,
    tx: Sender<Update>,
    notify: Box<dyn Fn() + Send>,
    dev: Option<Device>,
    sim: SharedSim,
    /// HID path of the connected keypad (hardware mode).
    hid_path: Option<String>,
    last_connection: Option<Connection>,
    next_scan: Instant,
    next_presence: Instant,
    /// Whether the last scan found nothing, so the next arrival is a hot-plug.
    was_absent: bool,
}

impl Worker {
    fn new(
        mode: Mode,
        rx: Receiver<Command>,
        tx: Sender<Update>,
        notify: Box<dyn Fn() + Send>,
    ) -> Self {
        let now = Instant::now();
        Self {
            mode,
            rx,
            tx,
            notify,
            dev: None,
            sim: SharedSim::new(),
            hid_path: None,
            last_connection: None,
            next_scan: now,
            next_presence: now + PRESENCE_INTERVAL,
            was_absent: false,
        }
    }

    fn run(mut self) {
        self.connect();
        loop {
            let idle = if self.dev.is_some() {
                IDLE_CONNECTED
            } else {
                IDLE_DISCONNECTED
            };
            match self.rx.recv_timeout(idle) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
    }

    fn emit(&self, u: Update) {
        let _ = self.tx.send(u);
        (self.notify)();
    }

    fn set_connection(&mut self, c: Connection) {
        if self.last_connection.as_ref() != Some(&c) {
            self.last_connection = Some(c.clone());
            self.emit(Update::Connection(c));
        }
    }

    fn disconnect(&mut self, why: Option<String>) {
        self.flush_log();
        self.dev = None;
        self.hid_path = None;
        self.was_absent = true;
        self.next_scan = Instant::now() + SCAN_INTERVAL;
        self.set_connection(why.map_or(Connection::Searching, Connection::Failed));
    }

    /// Opens the simulator, or tries the keypad once.
    fn connect(&mut self) {
        self.dev = None;
        self.hid_path = None;
        match self.mode {
            Mode::Simulated => {
                self.sim = SharedSim::new();
                let t: Box<dyn Transport> = Box::new(self.sim.clone());
                self.dev = Some(Falcon::new(Recorder::new(t)));
                self.after_connect(true, "in-memory keypad".into());
            }
            Mode::Hardware => {
                self.next_scan = Instant::now() + SCAN_INTERVAL;
                match HidTransport::open() {
                    Ok(t) => {
                        let path = t.config_info().path.clone();
                        let t: Box<dyn Transport> = Box::new(t);
                        let mut dev = Falcon::new(Recorder::new(t));
                        if self.was_absent {
                            // Windows waits after the device appears, before 0x80 (spec §2).
                            dev.settle();
                        }
                        self.dev = Some(dev);
                        self.hid_path = Some(path.clone());
                        self.was_absent = false;
                        self.next_presence = Instant::now() + PRESENCE_INTERVAL;
                        self.after_connect(false, path);
                    }
                    Err(Error::DeviceNotFound) => {
                        self.was_absent = true;
                        self.set_connection(Connection::Searching);
                    }
                    Err(Error::PermissionDenied(_)) => {
                        self.was_absent = false;
                        self.next_scan = Instant::now() + PERMISSION_RETRY;
                        // Reported once: set_connection ignores repeats.
                        self.set_connection(Connection::PermissionDenied);
                    }
                    Err(e) => {
                        self.was_absent = false;
                        self.set_connection(Connection::Failed(e.to_string()));
                    }
                }
            }
        }
    }

    fn after_connect(&mut self, simulated: bool, detail: String) {
        let firmware = self.read_firmware();
        self.set_connection(Connection::Connected {
            simulated,
            firmware,
            detail,
        });
        // Unknown until the next refresh if 0x81 fails; the connection itself is fine.
        let _ = self.read_active_profile();
        if !simulated {
            self.read_layers();
        }
        self.flush_log();
    }

    /// Reads every layer's block from the keypad (`0x82` ×5), so the app shows what the keypad
    /// really holds (lighting and key map) rather than what the config file last said.
    fn read_layers(&mut self) {
        for p in 0..PROFILE_COUNT {
            let Some(dev) = self.dev.as_mut() else {
                return;
            };
            // A failed read surfaces on the next command; keep the configured layer.
            let Ok(block) = dev.read_block(p) else {
                return;
            };
            self.emit(Update::KeypadBlock(p, Box::new(block)));
        }
    }

    /// `0x80`, 264 bytes only: the 8-byte form Windows uses makes the keypad hang on macOS. The
    /// firmware does not implement the long form, so this is `None` (not available).
    fn read_firmware(&mut self) -> Option<FirmwareVersion> {
        let dev = self.dev.as_mut()?;
        dev.firmware_version(ReportLen::Long).ok().flatten()
    }

    /// `0x81`: the edited layer follows the keypad's active layer.
    fn read_active_profile(&mut self) -> Result<(), String> {
        let dev = self.dev.as_mut().ok_or("keypad not connected")?;
        let p = dev.current_profile().map_err(|e| e.to_string())?;
        self.emit(Update::ActiveProfile(p));
        Ok(())
    }

    fn handle(&mut self, cmd: Command) {
        let (job, result) = match cmd {
            Command::Shutdown => return,
            Command::SetMode(mode) => {
                self.mode = mode;
                self.was_absent = false;
                self.last_connection = None;
                self.connect();
                return;
            }
            Command::Refresh => {
                let r = self.refresh();
                (Job::Refresh, r)
            }
            Command::SelectProfile(p) => {
                let r = self.with_dev(|d| d.select_profile(p));
                if r.is_ok() {
                    self.emit(Update::ActiveProfile(p));
                }
                (Job::SelectProfile(p), r)
            }
            Command::ApplyLighting { profile, lighting } => {
                let r = self.with_dev(|d| d.apply_lighting(profile, &lighting));
                (Job::ApplyLighting(profile), r)
            }
            Command::WriteProfiles(list) => {
                let ids = list.iter().map(|(p, _)| *p).collect();
                let r = self.write_profiles(list);
                (Job::Write(ids), r)
            }
            Command::Simulate(action) => (Job::Simulate, self.simulate(action)),
            // Desktop builds find the keypad on their own.
            Command::Pair => (Job::Pair, Ok(())),
            Command::Backup(config) => {
                let r = match self.dev.as_mut() {
                    Some(d) => d
                        .backup(&config, super::now_unix())
                        .map_err(|e| e.to_string()),
                    None => Err("keypad not connected".into()),
                };
                (
                    Job::Backup,
                    r.map(|b| self.emit(Update::BackupReady(Box::new(b)))),
                )
            }
            Command::Restore(backup) => (Job::Restore, self.restore(*backup)),
        };
        self.flush_log();
        self.emit(Update::Done { job, result });
    }

    fn with_dev(
        &mut self,
        f: impl FnOnce(&mut Device) -> falcon_core::Result<()>,
    ) -> Result<(), String> {
        let dev = self.dev.as_mut().ok_or("keypad not connected")?;
        f(dev).map_err(|e| e.to_string())
    }

    fn refresh(&mut self) -> Result<(), String> {
        if self.dev.is_none() {
            self.connect();
        }
        let detail = match &self.last_connection {
            Some(Connection::Connected { detail, .. }) => detail.clone(),
            _ => return Err("keypad not connected".into()),
        };
        let firmware = self.read_firmware();
        self.set_connection(Connection::Connected {
            simulated: self.mode == Mode::Simulated,
            firmware,
            detail,
        });
        if self.mode == Mode::Hardware {
            self.read_layers();
        }
        self.read_active_profile()
    }

    /// Writes every layer of `backup`, then selects its active layer.
    fn restore(&mut self, backup: Backup) -> Result<(), String> {
        backup.validate().map_err(|e| e.to_string())?;
        let total = backup.layers.len();
        for (i, layer) in backup.layers.into_iter().enumerate() {
            self.emit(Update::Progress {
                done: i,
                total,
                what: format!("Restoring layer {}", layer.layer),
            });
            self.with_dev(|d| d.restore_layer(layer.layer - 1, &layer))
                .map_err(|e| format!("layer {}: {e}", layer.layer))?;
            self.flush_log();
            self.emit(Update::Restored(Box::new(layer)));
        }
        if let Some(a) = backup.active_layer
            && self.with_dev(|d| d.select_profile(a - 1)).is_ok()
        {
            self.emit(Update::ActiveProfile(a - 1));
        }
        self.emit(Update::Progress {
            done: total,
            total,
            what: "Done".into(),
        });
        Ok(())
    }

    fn write_profiles(&mut self, list: Vec<(usize, Profile)>) -> Result<(), String> {
        let total = list.len();
        for (i, (p, profile)) in list.into_iter().enumerate() {
            self.emit(Update::Progress {
                done: i,
                total,
                what: format!("Writing layer {}", p + 1),
            });
            self.with_dev(|d| d.write_profile(p, &profile))
                .map_err(|e| format!("layer {}: {e}", p + 1))?;
            self.flush_log();
            self.emit(Update::Written(p, Box::new(profile)));
        }
        self.emit(Update::Progress {
            done: total,
            total,
            what: "Done".into(),
        });
        Ok(())
    }

    fn simulate(&mut self, action: SimAction) -> Result<(), String> {
        if self.mode != Mode::Simulated || self.dev.is_none() {
            return Err("only available with the simulated keypad".into());
        }
        match action {
            SimAction::PressLayer(p) => self.sim.press_layer(p).map_err(|e| e.to_string()),
            SimAction::PressHostKey(k) => {
                self.sim.press_host_key(k);
                Ok(())
            }
            SimAction::DemoEvents => {
                self.sim.push_demo_events();
                Ok(())
            }
        }
    }

    /// Hot-plug scan, presence check and event polling.
    fn tick(&mut self) {
        let now = Instant::now();
        if self.mode == Mode::Hardware {
            if self.dev.is_none() && now >= self.next_scan {
                self.connect();
            } else if self.dev.is_some() && now >= self.next_presence {
                self.next_presence = now + PRESENCE_INTERVAL;
                if !self.still_present() {
                    self.disconnect(None);
                    return;
                }
            }
        }
        self.poll_events();
    }

    fn still_present(&self) -> bool {
        let Some(path) = &self.hid_path else {
            return false;
        };
        // An enumeration error says nothing about the keypad; keep the connection.
        hid::list_collections().map_or(true, |all| all.iter().any(|c| &c.path == path))
    }

    fn poll_events(&mut self) {
        // Bounded, so a chatty device cannot starve the command queue.
        for _ in 0..16 {
            let Some(dev) = self.dev.as_mut() else {
                return;
            };
            match dev.poll_event(EVENT_POLL) {
                Ok(Some(report)) => {
                    if let Some(DeviceEvent::ProfileChanged { profile }) = report.event {
                        self.emit(Update::ActiveProfile(profile));
                    }
                    self.emit(Update::Event(report));
                    self.flush_log();
                }
                Ok(None) => break,
                Err(e) => {
                    let msg = format!("lost the keypad: {e}");
                    self.disconnect(Some(msg));
                    break;
                }
            }
        }
    }

    /// Sends new packet-log entries to the UI and forgets them.
    fn flush_log(&mut self) {
        let Some(dev) = self.dev.as_mut() else {
            return;
        };
        if dev.transport().log().is_empty() {
            return;
        }
        let log = dev.transport().log().to_vec();
        dev.transport_mut().clear_log();
        self.emit(Update::Packets(log));
    }
}
