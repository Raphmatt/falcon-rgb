//! Protocol, data model and transports for the MAX Falcon-20 RGB keypad (USB `195D:6008`).
//!
//! The protocol is described in `docs/protocol.md` (cited as "spec §n", hardware findings in
//! §11); it was worked out for interoperability and checked on a real keypad. Items not yet confirmed on hardware are marked **[HW-VERIFY]** and their
//! constants live in [`protocol`].
//!
//! Layering, bottom to top:
//!
//! - [`protocol`], [`layout`], [`codes`], [`descriptor`], [`hex`]: constants, tables, packet
//!   builders and parsers. No I/O.
//! - [`model`]: the user-facing configuration ([`Config`], [`Profile`], [`KeyAssignment`],
//!   [`Lighting`]), serde-serialisable and validated.
//! - [`encode`]: model → device bytes and pure command plans ([`Step`]s), shared by the real
//!   device, the simulator and `--dry-run`.
//! - [`transport`]: the [`Transport`] trait, the [`SimulatedDevice`], the logging/dry-run
//!   [`Recorder`] and (feature `hid`) the hidapi transport.
//! - [`device`]: [`Falcon`], the high-level API that executes plans.
//! - [`asynch`]: [`AsyncFalcon`], the same API over a non-blocking [`AsyncTransport`] (WebHID).
//! - [`store`]: JSON persistence of the [`Config`]; [`backup`]: whole-keypad backups.

pub mod asynch;
pub mod backup;
pub mod codes;
pub mod descriptor;
pub mod device;
pub mod encode;
mod error;
pub mod hex;
pub mod layout;
pub mod model;
pub mod protocol;
pub mod store;
pub mod transport;

pub use asynch::{AsyncFalcon, AsyncRecorder, AsyncTransport};
pub use backup::{Backup, LayerBackup};
pub use codes::Function;
pub use device::{Falcon, InputReport};
pub use encode::{Step, StepKind};
pub use error::{Error, Result, ValidationError};
pub use model::{
    Config, Effect, KeyAssignment, Lighting, Macro, MacroEvent, Profile, Repeat, Rgb, Settings,
    WaveDirection,
};
pub use protocol::{Block, DeviceEvent, FirmwareVersion, ReportLen};
pub use transport::{PacketDir, PacketRecord, Recorder, SimulatedDevice, Transport};
