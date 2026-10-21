//! Messages between the UI and the worker thread.

use falcon_core::protocol::Block;
use falcon_core::{
    Backup, Config, FirmwareVersion, InputReport, LayerBackup, Lighting, PacketRecord, Profile,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Hardware,
    Simulated,
}

/// Device-side actions the simulator can fake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimAction {
    /// A layer key was pressed on the keypad (0-based profile).
    PressLayer(usize),
    /// A host-reporting key was pressed (k index).
    PressHostKey(usize),
    /// Queue one example of every input report.
    DemoEvents,
}

#[derive(Debug, Clone)]
pub enum Command {
    SetMode(Mode),
    /// Re-read firmware version and active profile.
    Refresh,
    SelectProfile(usize),
    ApplyLighting {
        profile: usize,
        lighting: Box<Lighting>,
    },
    /// Full downloads (Windows "Save") of these profiles, in order.
    WriteProfiles(Vec<(usize, Profile)>),
    Simulate(SimAction),
    /// Read every layer into a [`Backup`]; macros of macro keys come from this config.
    Backup(Box<Config>),
    /// Write every layer of the backup back (and select its active layer).
    Restore(Box<Backup>),
    /// Browser: show the WebHID device chooser (must follow a click). Ignored natively.
    Pair,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// Hardware mode, no keypad found (yet).
    Searching,
    Connected {
        simulated: bool,
        firmware: Option<FirmwareVersion>,
        /// HID path or "in-memory keypad".
        detail: String,
    },
    /// A keypad is there but could not be opened.
    Failed(String),
    /// macOS refused the open: the app lacks Input Monitoring permission. Retried quietly.
    PermissionDenied,
    /// Browser: no keypad allowed for this site yet; the user has to pick it once.
    NeedsPairing,
    /// Browser without WebHID (Safari, Firefox, or not a secure context).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Unsupported,
}

impl Connection {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// Which command a [`Update::Done`] belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Refresh,
    SelectProfile(usize),
    ApplyLighting(usize),
    Write(Vec<usize>),
    Simulate,
    Pair,
    Backup,
    Restore,
}

#[derive(Debug, Clone)]
pub enum Update {
    Connection(Connection),
    /// The keypad's active profile (0-based): after our select, a layer key, or `0x81` at
    /// connect and refresh (its 1-based reply is verified on hardware).
    ActiveProfile(usize),
    Event(InputReport),
    Packets(Vec<PacketRecord>),
    Progress {
        done: usize,
        total: usize,
        what: String,
    },
    /// One profile of a [`Command::WriteProfiles`] reached the device.
    Written(usize, Box<Profile>),
    /// Result of [`Command::Backup`].
    BackupReady(Box<Backup>),
    /// One layer of a [`Command::Restore`] reached the keypad.
    Restored(Box<LayerBackup>),
    /// A layer's block as stored on the real keypad (`0x82` at connect and refresh): the app
    /// adopts its lighting and key map.
    KeypadBlock(usize, Box<Block>),
    Done {
        job: Job,
        result: Result<(), String>,
    },
}
