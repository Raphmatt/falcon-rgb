//! Application state and the top-level layout. Device I/O lives in [`crate::worker`]; the
//! panels are in [`crate::views`].

mod actions;
mod persist;
mod updates;

use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use falcon_core::protocol::PROFILE_COUNT;
use falcon_core::{Config, Lighting, Profile, Rgb};
use web_time::Instant;

pub use persist::ConfigStore;

use crate::autoswitch::{self, FrontApp};
use crate::worker::{Connection, Mode, WorkerHandle};
use crate::{preview, theme, views};

/// Command-line options (in the browser: the URL query, `?simulate`).
#[derive(Debug, Clone, Default)]
pub struct Args {
    pub simulate: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub config: Option<PathBuf>,
}

/// What the user clicked on the keypad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Key(usize),
    Bar(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Keys,
    Lighting,
    Layer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    Set,
    Get,
    Input,
    Info,
    Error,
}

/// One line of the packet/activity log.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// Seconds since app start.
    pub at: f64,
    pub kind: LogKind,
    pub text: String,
    /// Hex of the packet (empty for activity lines).
    pub hex: String,
}

const LOG_CAP: usize = 3000;
/// Shown when macOS refuses to open the keypad (TCC Input Monitoring).
pub const PERMISSION_TEXT: &str = "macOS blocks access to the keypad: allow Falcon RGB under System Settings → Privacy & Security → Input Monitoring, then restart the app. Ad-hoc signed builds may need to be allowed again after each rebuild.";
/// Auto-apply waits this long after the last lighting edit. Every `0x02` lands in the keypad's
/// flash (a lighting-only write survives a power cycle), so dragging a slider must not turn
/// into a stream of writes: wait for the user to pause, and skip values the keypad already has.
const APPLY_DEBOUNCE: Duration = Duration::from_millis(1000);

/// An in-app macro recording into `profiles[profile].keys[key]`.
#[derive(Debug, Clone, Copy)]
pub struct Recording {
    pub profile: usize,
    pub key: usize,
    /// egui input time of the previous event.
    pub last: Option<f64>,
}

pub struct FalconApp {
    pub config: Config,
    file_snapshot: Config,
    /// What the keypad is believed to hold per layer (keys + lighting).
    device_snapshot: [Profile; PROFILE_COUNT],
    pub config_store: ConfigStore,
    worker: WorkerHandle,
    pub mode: Mode,
    pub conn: Connection,
    /// Active layer as reported by the keypad (`0x81` or a layer event).
    pub device_profile: Option<usize>,
    pub target: Option<Target>,
    pub tab: Tab,
    /// Customize mode paint colour.
    pub paint: Rgb,
    pub auto_apply: bool,
    lighting_changed_at: Option<Instant>,
    /// Lighting sent with the in-flight apply.
    applying: Option<(usize, Lighting)>,
    /// Commands sent that have not reported `Done` yet.
    pub busy: usize,
    pub progress: Option<(usize, usize, String)>,
    /// Latest message for the status bar: (is_error, text).
    pub status: Option<(bool, String)>,
    pub log: VecDeque<LogLine>,
    pub show_log: bool,
    pub log_hide_get: bool,
    pub recording: Option<Recording>,
    pub key_filter: String,
    pub confirm_reset: bool,
    pub new_linked_app: String,
    pub front_app: Option<FrontApp>,
    /// Layer file picked for import, possibly arriving frames after the dialog (browser).
    pub import_inbox: crate::files::Inbox,
    watcher: autoswitch::Watcher,
    started: Instant,
}

impl FalconApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        theme::install(&cc.egui_ctx);
        let mode = if args.simulate {
            Mode::Simulated
        } else {
            Mode::Hardware
        };
        #[cfg(not(target_arch = "wasm32"))]
        let config_store = ConfigStore::default_for(args.simulate, args.config);
        #[cfg(target_arch = "wasm32")]
        let config_store = ConfigStore::default_for(args.simulate);
        let (config, load_error) = match config_store.load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e)),
        };
        let ctx = cc.egui_ctx.clone();
        let worker = WorkerHandle::spawn(mode, move || ctx.request_repaint());
        let ctx = cc.egui_ctx.clone();
        let watcher = autoswitch::Watcher::spawn(move || ctx.request_repaint());
        let mut app = Self {
            device_snapshot: initial_device_snapshot(mode, &config),
            file_snapshot: config.clone(),
            config,
            config_store,
            worker,
            mode,
            conn: Connection::Searching,
            device_profile: None,
            target: Some(Target::Key(0)),
            tab: Tab::Keys,
            paint: Rgb::new(0x3D, 0x8B, 0xFD),
            auto_apply: false,
            lighting_changed_at: None,
            applying: None,
            busy: 0,
            progress: None,
            status: None,
            log: VecDeque::new(),
            show_log: false,
            log_hide_get: false,
            recording: None,
            key_filter: String::new(),
            confirm_reset: false,
            new_linked_app: String::new(),
            front_app: None,
            import_inbox: Default::default(),
            watcher,
            started: Instant::now(),
        };
        match load_error {
            Some(e) => app.error(format!(
                "Could not read the saved config ({e}); using defaults. Saving overwrites it."
            )),
            None => app.info(format!("Config: {}", app.config_store.describe())),
        }
        app
    }

    pub fn active(&self) -> usize {
        self.config.active_profile.min(PROFILE_COUNT - 1)
    }

    pub fn profile(&self) -> &Profile {
        &self.config.profiles[self.active()]
    }

    pub fn profile_mut(&mut self) -> &mut Profile {
        let p = self.active();
        &mut self.config.profiles[p]
    }

    /// Unsaved changes compared with the config file.
    pub fn file_dirty(&self) -> bool {
        self.config.profiles != self.file_snapshot.profiles
            || self.config.settings != self.file_snapshot.settings
    }

    /// Layer `p` differs from what the keypad is believed to hold.
    pub fn layer_dirty(&self, p: usize) -> bool {
        let (a, b) = (&self.config.profiles[p], &self.device_snapshot[p]);
        a.keys != b.keys || a.lighting != b.lighting
    }

    pub fn seconds(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn push_log(&mut self, kind: LogKind, text: String, hex: String) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        let at = self.seconds();
        self.log.push_back(LogLine {
            at,
            kind,
            text,
            hex,
        });
    }

    pub fn info(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.push_log(LogKind::Info, msg.clone(), String::new());
        self.status = Some((false, msg));
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.push_log(LogKind::Error, msg.clone(), String::new());
        self.status = Some((true, msg));
    }

    /// Current lighting preview for the keypad view.
    pub fn preview_frame(&self) -> preview::Frame {
        let last_key = match self.target {
            Some(Target::Key(k)) => Some(k),
            _ => None,
        };
        preview::render(
            &self.profile().lighting,
            self.started.elapsed().as_secs_f32(),
            last_key,
        )
    }
}

/// The simulator boots with factory blocks; real hardware is assumed to match the file.
fn initial_device_snapshot(mode: Mode, config: &Config) -> [Profile; PROFILE_COUNT] {
    match mode {
        Mode::Simulated => std::array::from_fn(Profile::default_for),
        Mode::Hardware => config.profiles.clone(),
    }
}

impl eframe::App for FalconApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_updates();
        self.watcher.set_enabled(self.config.settings.auto_switch);
        self.process_front_apps();
        views::layer::finish_import(self);
        if let Some(wait) = self.auto_apply_tick() {
            ctx.request_repaint_after(wait);
        }
        if preview::is_animated(self.profile().lighting.effect) {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.capture_recording(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.recording.is_none()
            && ui.input_mut(|i| {
                i.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::S,
                ))
            })
        {
            self.save(false);
        }
        egui::Panel::top("toolbar").show(ui, |ui| views::top::show(ui, self));
        if self.conn == Connection::PermissionDenied {
            egui::Panel::top("permission")
                .show(ui, |ui| views::status::permission_banner(ui, self));
        }
        egui::Panel::bottom("status").show(ui, |ui| views::status::show(ui, self));
        if self.show_log {
            egui::Panel::bottom("log")
                .resizable(true)
                .default_size(220.0)
                .min_size(120.0)
                .show(ui, |ui| views::log::show(ui, self));
        }
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(430.0)
            .min_size(360.0)
            .show(ui, |ui| views::inspector(ui, self));
        egui::CentralPanel::default().show(ui, |ui| views::keypad::show(ui, self));
        views::layer::reset_dialog(ui, self);
    }
}
