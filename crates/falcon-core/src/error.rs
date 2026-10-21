use thiserror::Error;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong talking to the keypad or handling its configuration.
#[derive(Debug, Error)]
pub enum Error {
    #[error("profile index {0} out of range (expected 0..=4)")]
    InvalidProfile(usize),
    #[error("invalid configuration: {0}")]
    Validation(#[from] ValidationError),
    #[error("reply to command {cmd:#04x} too short: expected at least {expected} bytes, got {got}")]
    ShortReply {
        cmd: u8,
        expected: usize,
        got: usize,
    },
    #[error("malformed reply to command {cmd:#04x}: {reason}")]
    MalformedReply { cmd: u8, reason: String },
    #[error("unexpected {what} value {value:#04x}")]
    UnexpectedValue { what: &'static str, value: u8 },
    #[error(
        "refusing to send a {0}-byte feature report: on macOS only 264 bytes work, anything else makes the keypad stop answering until it is replugged"
    )]
    ReportLengthBlocked(usize),
    #[error(
        "macOS blocks access to the keypad (Input Monitoring): allow the app that opens it (Falcon RGB, or your terminal for the CLI) under System Settings → Privacy & Security → Input Monitoring, then restart that app. Ad-hoc signed builds may need to be allowed again after each rebuild. ({0})"
    )]
    PermissionDenied(String),
    #[error("no MAX Falcon-20 RGB (195D:6008) found; check the cable and USB permissions")]
    DeviceNotFound,
    #[error("keypad found, but none of its collections has usage page 0xFF01 / usage 1")]
    ConfigCollectionNotFound,
    #[error("HID: {0}")]
    Hid(String),
    #[error("backup: {0}")]
    Backup(String),
    #[error("simulated device: {0}")]
    Sim(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// A configuration value the device cannot represent. Returned by the `validate` methods of the
/// [`crate::model`] types and before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("{0} macro events; the device holds at most 240 (spec §5)")]
    TooManyEvents(usize),
    #[error("macro or combo has no events")]
    NoEvents,
    #[error("event {index}: delay {delay} × 10 ms exceeds 0x7FFF (327.67 s)")]
    DelayTooLong { index: usize, delay: u16 },
    #[error("event {index}: key code 0 would end the event list early")]
    ZeroCode { index: usize },
    #[error("repeat count 0; use Repeat::Toggle for \"until pressed again\"")]
    RepeatZero,
    #[error("brightness {0} out of range 1..=5 (1 = LEDs off)")]
    Brightness(u8),
    #[error("lighting effect {0} out of range 0..=10")]
    Effect(u8),
    #[error("wave direction {0} out of range 0..=3")]
    Direction(u8),
    #[error("single key code 0 does nothing on the device; use Disabled")]
    ZeroKey,
    #[error(
        "{0:#04x} is a reserved device code (E8 Windows combo, F8 macro, F7 host key) and needs data the single-key form does not send; use Combo, Macro or LaunchProgram"
    )]
    ReservedCode(u8),
    #[error("launch-program path is empty")]
    EmptyPath,
    #[error("key {}: {source}", .key + 1)]
    Key {
        key: usize,
        #[source]
        source: Box<ValidationError>,
    },
    #[error("profile {}: {source}", .profile + 1)]
    Profile {
        profile: usize,
        #[source]
        source: Box<ValidationError>,
    },
}

impl ValidationError {
    pub(crate) fn in_key(self, key: usize) -> Self {
        Self::Key {
            key,
            source: Box::new(self),
        }
    }

    pub(crate) fn in_profile(self, profile: usize) -> Self {
        Self::Profile {
            profile,
            source: Box::new(self),
        }
    }
}
