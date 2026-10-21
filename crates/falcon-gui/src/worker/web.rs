//! Browser worker: one async task on the page's event loop (wasm has no threads). It speaks the
//! same [`Command`]/[`Update`] protocol as the desktop worker ([`super::native`]).
//!
//! - Hardware mode: WebHID ([`super::webhid`]) through [`AsyncFalcon`]. A keypad the user
//!   allowed before is opened automatically; otherwise [`Connection::NeedsPairing`] until the
//!   user clicks "Connect keypad…" ([`Command::Pair`], which opens the browser's chooser; the
//!   click's user activation is still valid when the task gets to it). `navigator.hid`
//!   connect/disconnect events drive hot-plug, plus a cheap `getDevices` re-scan.
//! - Simulated mode: the same sync [`SharedSim`] as on the desktop. Its delays are virtual, so
//!   calling it from the task never blocks the page.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use falcon_core::protocol::{Block, PROFILE_COUNT};
use falcon_core::{
    AsyncFalcon, AsyncRecorder, Backup, Config, DeviceEvent, Falcon, FirmwareVersion, InputReport,
    LayerBackup, Lighting, PacketRecord, Profile, Recorder, ReportLen, Result,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{HidConnectionEvent, HidDevice};
use web_time::Instant;

use super::webhid::{self, WebHid};
use super::{Command, Connection, Job, Mode, SharedSim, SimAction, Update};

/// How often to look for an allowed keypad while none is open.
const SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Pause between loop iterations (command check + event poll).
const IDLE_CONNECTED: Duration = Duration::from_millis(15);
const IDLE_DISCONNECTED: Duration = Duration::from_millis(250);

/// Owner side of the worker task. Dropping it stops the task.
pub struct WorkerHandle {
    tx: Sender<Command>,
    rx: Receiver<Update>,
}

impl WorkerHandle {
    /// Starts the worker; `notify` runs after every batch of updates (e.g. request a repaint).
    pub fn spawn(mode: Mode, notify: impl Fn() + Send + 'static) -> Self {
        let (tx, cmd_rx) = mpsc::channel();
        let (up_tx, rx) = mpsc::channel();
        wasm_bindgen_futures::spawn_local(Worker::new(mode, cmd_rx, up_tx, Box::new(notify)).run());
        Self { tx, rx }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    pub fn try_iter(&self) -> impl Iterator<Item = Update> + '_ {
        self.rx.try_iter()
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

/// The open keypad: simulated (sync, virtual delays) or real (WebHID, async).
enum Dev {
    Sim(Falcon<Recorder<SharedSim>>),
    Hid(AsyncFalcon<AsyncRecorder<WebHid>>),
}

impl Dev {
    async fn firmware(&mut self) -> Option<FirmwareVersion> {
        // 264 bytes only (spec §2): the keypad does not implement it, so this is "n/a".
        match self {
            Self::Sim(d) => d.firmware_version(ReportLen::Long),
            Self::Hid(d) => d.firmware_version(ReportLen::Long).await,
        }
        .ok()
        .flatten()
    }

    async fn backup(&mut self, config: &Config) -> Result<Backup> {
        match self {
            Self::Sim(d) => d.backup(config, super::now_unix()),
            Self::Hid(d) => d.backup(config, super::now_unix()).await,
        }
    }

    async fn restore_layer(&mut self, p: usize, layer: &LayerBackup) -> Result<()> {
        match self {
            Self::Sim(d) => d.restore_layer(p, layer),
            Self::Hid(d) => d.restore_layer(p, layer).await,
        }
    }

    async fn read_block(&mut self, p: usize) -> Result<Block> {
        match self {
            Self::Sim(d) => d.read_block(p),
            Self::Hid(d) => d.read_block(p).await,
        }
    }

    async fn current_profile(&mut self) -> Result<usize> {
        match self {
            Self::Sim(d) => d.current_profile(),
            Self::Hid(d) => d.current_profile().await,
        }
    }

    async fn select_profile(&mut self, p: usize) -> Result<()> {
        match self {
            Self::Sim(d) => d.select_profile(p),
            Self::Hid(d) => d.select_profile(p).await,
        }
    }

    async fn write_profile(&mut self, p: usize, profile: &Profile) -> Result<()> {
        match self {
            Self::Sim(d) => d.write_profile(p, profile),
            Self::Hid(d) => d.write_profile(p, profile).await,
        }
    }

    async fn apply_lighting(&mut self, p: usize, lighting: &Lighting) -> Result<()> {
        match self {
            Self::Sim(d) => d.apply_lighting(p, lighting),
            Self::Hid(d) => d.apply_lighting(p, lighting).await,
        }
    }

    fn poll_event(&mut self) -> Result<Option<InputReport>> {
        match self {
            Self::Sim(d) => d.poll_event(Duration::ZERO),
            Self::Hid(d) => d.poll_event(),
        }
    }

    fn take_log(&mut self) -> Vec<PacketRecord> {
        match self {
            Self::Sim(d) => {
                let log = d.transport().log().to_vec();
                d.transport_mut().clear_log();
                log
            }
            Self::Hid(d) => {
                let log = d.transport().log().to_vec();
                d.transport_mut().clear_log();
                log
            }
        }
    }
}

type HidListener = Closure<dyn FnMut(HidConnectionEvent)>;

struct Worker {
    mode: Mode,
    rx: Receiver<Command>,
    tx: Sender<Update>,
    notify: Box<dyn Fn() + Send>,
    dev: Option<Dev>,
    sim: SharedSim,
    last_connection: Option<Connection>,
    next_scan: Instant,
    /// Set by the `navigator.hid` listeners for the keypad.
    plugged: Rc<Cell<bool>>,
    unplugged: Rc<Cell<bool>>,
    _listeners: Option<(HidListener, HidListener)>,
}

impl Worker {
    fn new(
        mode: Mode,
        rx: Receiver<Command>,
        tx: Sender<Update>,
        notify: Box<dyn Fn() + Send>,
    ) -> Self {
        Self {
            mode,
            rx,
            tx,
            notify,
            dev: None,
            sim: SharedSim::new(),
            last_connection: None,
            next_scan: Instant::now(),
            plugged: Rc::default(),
            unplugged: Rc::default(),
            _listeners: None,
        }
    }

    async fn run(mut self) {
        self.install_listeners();
        self.connect(false).await;
        loop {
            loop {
                match self.rx.try_recv() {
                    Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => return,
                    Ok(cmd) => self.handle(cmd).await,
                    Err(TryRecvError::Empty) => break,
                }
            }
            self.tick().await;
            let idle = if self.dev.is_some() {
                IDLE_CONNECTED
            } else {
                IDLE_DISCONNECTED
            };
            webhid::sleep(idle).await;
        }
    }

    fn install_listeners(&mut self) {
        let Some(hid) = webhid::hid() else {
            return;
        };
        let flag = self.plugged.clone();
        let on_connect = HidListener::new(move |e: HidConnectionEvent| {
            if webhid::is_keypad(&e.device()) {
                flag.set(true);
            }
        });
        let flag = self.unplugged.clone();
        let on_disconnect = HidListener::new(move |e: HidConnectionEvent| {
            if webhid::is_keypad(&e.device()) {
                flag.set(true);
            }
        });
        hid.set_onconnect(Some(on_connect.as_ref().unchecked_ref()));
        hid.set_ondisconnect(Some(on_disconnect.as_ref().unchecked_ref()));
        self._listeners = Some((on_connect, on_disconnect));
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
        self.next_scan = Instant::now() + SCAN_INTERVAL;
        self.set_connection(why.map_or(Connection::Searching, Connection::Failed));
    }

    /// Opens the simulator, or an already allowed keypad. `arrived`: it was just plugged in.
    async fn connect(&mut self, arrived: bool) {
        self.dev = None;
        match self.mode {
            Mode::Simulated => {
                self.sim = SharedSim::new();
                self.dev = Some(Dev::Sim(Falcon::new(Recorder::new(self.sim.clone()))));
                self.after_connect(true, "in-memory keypad".into()).await;
            }
            Mode::Hardware => {
                self.next_scan = Instant::now() + SCAN_INTERVAL;
                let Some(hid) = webhid::hid() else {
                    self.set_connection(Connection::Unsupported);
                    return;
                };
                match webhid::granted(&hid).await {
                    Ok(Some(device)) => {
                        let _ = self.open(device, arrived).await;
                    }
                    // Keep "not found" after an unplug; a first visit has to pair.
                    Ok(None) if self.last_connection.is_some() => {
                        if !matches!(self.last_connection, Some(Connection::NeedsPairing)) {
                            self.set_connection(Connection::Searching);
                        }
                    }
                    Ok(None) => self.set_connection(Connection::NeedsPairing),
                    Err(e) => self.set_connection(Connection::Failed(e.to_string())),
                }
            }
        }
    }

    async fn open(&mut self, device: HidDevice, arrived: bool) -> Result<(), String> {
        match WebHid::open(device).await {
            Ok(t) => {
                let detail = format!("WebHID: {}", t.name());
                let mut dev = AsyncFalcon::new(AsyncRecorder::new(t));
                if arrived {
                    // Windows waits after the device appears, before 0x80 (spec §2).
                    dev.settle().await;
                }
                self.unplugged.set(false);
                self.dev = Some(Dev::Hid(dev));
                self.after_connect(false, detail).await;
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_connection(Connection::Failed(msg.clone()));
                Err(msg)
            }
        }
    }

    async fn after_connect(&mut self, simulated: bool, detail: String) {
        let firmware = match self.dev.as_mut() {
            Some(d) => d.firmware().await,
            None => None,
        };
        self.set_connection(Connection::Connected {
            simulated,
            firmware,
            detail,
        });
        let _ = self.read_active_profile().await;
        if !simulated {
            self.read_layers().await;
        }
        self.flush_log();
    }

    /// Every layer's block from the keypad (`0x82` ×5), as in the desktop worker.
    async fn read_layers(&mut self) {
        for p in 0..PROFILE_COUNT {
            let Some(dev) = self.dev.as_mut() else {
                return;
            };
            let Ok(block) = dev.read_block(p).await else {
                return;
            };
            self.emit(Update::KeypadBlock(p, Box::new(block)));
        }
    }

    async fn read_active_profile(&mut self) -> Result<(), String> {
        let dev = self.dev.as_mut().ok_or("keypad not connected")?;
        let p = dev.current_profile().await.map_err(|e| e.to_string())?;
        self.emit(Update::ActiveProfile(p));
        Ok(())
    }

    async fn handle(&mut self, cmd: Command) {
        let (job, result) = match cmd {
            Command::Shutdown => return,
            Command::SetMode(mode) => {
                self.mode = mode;
                self.last_connection = None;
                self.connect(false).await;
                return;
            }
            Command::Refresh => (Job::Refresh, self.refresh().await),
            Command::SelectProfile(p) => {
                let r = match self.dev.as_mut() {
                    Some(d) => d.select_profile(p).await.map_err(|e| e.to_string()),
                    None => Err("keypad not connected".into()),
                };
                if r.is_ok() {
                    self.emit(Update::ActiveProfile(p));
                }
                (Job::SelectProfile(p), r)
            }
            Command::ApplyLighting { profile, lighting } => {
                let r = match self.dev.as_mut() {
                    Some(d) => d
                        .apply_lighting(profile, &lighting)
                        .await
                        .map_err(|e| e.to_string()),
                    None => Err("keypad not connected".into()),
                };
                (Job::ApplyLighting(profile), r)
            }
            Command::WriteProfiles(list) => {
                let ids = list.iter().map(|(p, _)| *p).collect();
                (Job::Write(ids), self.write_profiles(list).await)
            }
            Command::Simulate(action) => (Job::Simulate, self.simulate(action)),
            Command::Pair => (Job::Pair, self.pair().await),
            Command::Backup(config) => {
                let r = match self.dev.as_mut() {
                    Some(d) => d.backup(&config).await.map_err(|e| e.to_string()),
                    None => Err("keypad not connected".into()),
                };
                (
                    Job::Backup,
                    r.map(|b| self.emit(Update::BackupReady(Box::new(b)))),
                )
            }
            Command::Restore(backup) => (Job::Restore, self.restore(*backup).await),
        };
        self.flush_log();
        self.emit(Update::Done { job, result });
    }

    async fn refresh(&mut self) -> Result<(), String> {
        if self.dev.is_none() {
            self.connect(false).await;
        }
        let detail = match &self.last_connection {
            Some(Connection::Connected { detail, .. }) => detail.clone(),
            _ => return Err("keypad not connected".into()),
        };
        let firmware = match self.dev.as_mut() {
            Some(d) => d.firmware().await,
            None => None,
        };
        self.set_connection(Connection::Connected {
            simulated: self.mode == Mode::Simulated,
            firmware,
            detail,
        });
        if self.mode == Mode::Hardware {
            self.read_layers().await;
        }
        self.read_active_profile().await
    }

    /// Shows the browser's chooser and opens the picked keypad.
    async fn pair(&mut self) -> Result<(), String> {
        if self.mode != Mode::Hardware {
            return Err("turn off the simulated keypad first".into());
        }
        if matches!(self.dev, Some(Dev::Hid(_))) {
            return Ok(());
        }
        let hid = webhid::hid().ok_or("this browser has no WebHID")?;
        match webhid::request(&hid).await.map_err(|e| e.to_string())? {
            Some(device) => self.open(device, false).await,
            None => Ok(()),
        }
    }

    /// Writes every layer of `backup`, then selects its active layer.
    async fn restore(&mut self, backup: Backup) -> Result<(), String> {
        backup.validate().map_err(|e| e.to_string())?;
        let total = backup.layers.len();
        for (i, layer) in backup.layers.into_iter().enumerate() {
            self.emit(Update::Progress {
                done: i,
                total,
                what: format!("Restoring layer {}", layer.layer),
            });
            let dev = self.dev.as_mut().ok_or("keypad not connected")?;
            dev.restore_layer(layer.layer - 1, &layer)
                .await
                .map_err(|e| format!("layer {}: {e}", layer.layer))?;
            self.flush_log();
            self.emit(Update::Restored(Box::new(layer)));
        }
        if let Some(a) = backup.active_layer
            && let Some(dev) = self.dev.as_mut()
            && dev.select_profile(a - 1).await.is_ok()
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

    async fn write_profiles(&mut self, list: Vec<(usize, Profile)>) -> Result<(), String> {
        let total = list.len();
        for (i, (p, profile)) in list.into_iter().enumerate() {
            self.emit(Update::Progress {
                done: i,
                total,
                what: format!("Writing layer {}", p + 1),
            });
            let dev = self.dev.as_mut().ok_or("keypad not connected")?;
            dev.write_profile(p, &profile)
                .await
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
        if !matches!(self.dev, Some(Dev::Sim(_))) {
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

    /// Hot-plug and event polling.
    async fn tick(&mut self) {
        if self.mode == Mode::Hardware {
            if matches!(self.dev, Some(Dev::Hid(_))) && self.unplugged.replace(false) {
                self.disconnect(None);
                return;
            }
            let arrived = self.plugged.replace(false);
            if self.dev.is_none() && (arrived || Instant::now() >= self.next_scan) {
                self.connect(arrived).await;
            }
        }
        self.poll_events();
    }

    fn poll_events(&mut self) {
        for _ in 0..16 {
            let Some(dev) = self.dev.as_mut() else {
                return;
            };
            match dev.poll_event() {
                Ok(Some(report)) => {
                    if let Some(DeviceEvent::ProfileChanged { profile }) = report.event {
                        self.emit(Update::ActiveProfile(profile));
                    }
                    self.emit(Update::Event(report));
                    self.flush_log();
                }
                Ok(None) => break,
                Err(e) => {
                    self.disconnect(Some(format!("lost the keypad: {e}")));
                    break;
                }
            }
        }
    }

    fn flush_log(&mut self) {
        let Some(dev) = self.dev.as_mut() else {
            return;
        };
        let log = dev.take_log();
        if !log.is_empty() {
            self.emit(Update::Packets(log));
        }
    }
}
