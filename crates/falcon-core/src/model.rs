//! User-facing configuration: five profiles of key assignments and lighting (spec §5, §6, §9).
//!
//! These are plain data types for the CLI, the GUI and JSON persistence. They know nothing about
//! packets; [`crate::encode`] turns them into device bytes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::codes::{self, Function, fncode};
use crate::error::ValidationError;
use crate::layout::{self, BAR_COUNT, KEY_COUNT};
use crate::protocol::{MAX_DELAY_10MS, MAX_MACRO_EVENTS, PROFILE_COUNT};

/// A colour. Serialised as `"RRGGBB"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(0xFF, 0xFF, 0xFF);
    pub const RED: Self = Self::new(0xFF, 0, 0);
    pub const GREEN: Self = Self::new(0, 0xFF, 0);
    pub const BLUE: Self = Self::new(0, 0, 0xFF);
    pub const YELLOW: Self = Self::new(0xFF, 0xFF, 0);
    pub const MAGENTA: Self = Self::new(0xFF, 0, 0xFF);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl FromStr for Rgb {
    type Err = String;

    /// Accepts `RRGGBB` or `#RRGGBB`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.trim().trim_start_matches('#');
        let bad = || format!("invalid colour {s:?} (expected RRGGBB)");
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        Ok(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Lighting effect, block byte 125 (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// 0: static colour.
    #[default]
    Static,
    /// 1
    Breathing,
    /// 2
    FadeIn,
    /// 3
    FadeOut,
    /// 4: last keystroke stays lit.
    LastKey,
    /// 5: RGB wave, uses the direction.
    Wave,
    /// 6: random single key.
    Random,
    /// 7: per-key colours ("Customize").
    Custom,
    /// Any other value; 8..=10 may exist in firmware **[HW-VERIFY]**.
    Other(u8),
}

impl Effect {
    /// The eight effects the Windows UI offers.
    pub const ALL: [Self; 8] = [
        Self::Static,
        Self::Breathing,
        Self::FadeIn,
        Self::FadeOut,
        Self::LastKey,
        Self::Wave,
        Self::Random,
        Self::Custom,
    ];

    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Static => 0,
            Self::Breathing => 1,
            Self::FadeIn => 2,
            Self::FadeOut => 3,
            Self::LastKey => 4,
            Self::Wave => 5,
            Self::Random => 6,
            Self::Custom => 7,
            Self::Other(b) => b,
        }
    }

    pub const fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Static,
            1 => Self::Breathing,
            2 => Self::FadeIn,
            3 => Self::FadeOut,
            4 => Self::LastKey,
            5 => Self::Wave,
            6 => Self::Random,
            7 => Self::Custom,
            b => Self::Other(b),
        }
    }

    /// Kebab-case name used by the CLI.
    pub fn name(self) -> String {
        match self {
            Self::Static => "static".into(),
            Self::Breathing => "breathing".into(),
            Self::FadeIn => "fade-in".into(),
            Self::FadeOut => "fade-out".into(),
            Self::LastKey => "last-key".into(),
            Self::Wave => "wave".into(),
            Self::Random => "random".into(),
            Self::Custom => "custom".into(),
            Self::Other(b) => format!("effect-{b}"),
        }
    }

    /// Whether the effect colour is used (effects 0..=4).
    pub const fn uses_color(self) -> bool {
        matches!(
            self,
            Self::Static | Self::Breathing | Self::FadeIn | Self::FadeOut | Self::LastKey
        )
    }

    /// Whether the wave direction is used.
    pub const fn uses_direction(self) -> bool {
        matches!(self, Self::Wave)
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

impl FromStr for Effect {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let n = s.trim().replace('_', "-").to_ascii_lowercase();
        if let Some(e) = Self::ALL.into_iter().find(|e| e.name() == n) {
            return Ok(e);
        }
        match n.as_str() {
            "normal" => Ok(Self::Static),
            "customize" | "per-key" => Ok(Self::Custom),
            _ => n
                .strip_prefix("effect-")
                .unwrap_or(&n)
                .parse::<u8>()
                .map(Self::from_byte)
                .map_err(|_| format!("unknown effect {s:?}")),
        }
    }
}

/// Wave direction, block byte 127: where the rainbow wave moves (all four values confirmed on
/// hardware, 2026-10-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WaveDirection {
    /// 0: right to left.
    RightToLeft,
    /// 1: left to right.
    #[default]
    LeftToRight,
    /// 2: top to bottom.
    TopToBottom,
    /// 3: bottom to top.
    BottomToTop,
    /// Any other raw value, as read back; validation rejects it.
    Other(u8),
}

impl WaveDirection {
    pub const ALL: [Self; 4] = [
        Self::RightToLeft,
        Self::LeftToRight,
        Self::TopToBottom,
        Self::BottomToTop,
    ];

    pub const fn to_byte(self) -> u8 {
        match self {
            Self::RightToLeft => 0,
            Self::LeftToRight => 1,
            Self::TopToBottom => 2,
            Self::BottomToTop => 3,
            Self::Other(b) => b,
        }
    }

    pub const fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::RightToLeft,
            1 => Self::LeftToRight,
            2 => Self::TopToBottom,
            3 => Self::BottomToTop,
            b => Self::Other(b),
        }
    }

    /// Snake-case name used in the config file.
    pub const fn config_name(self) -> Option<&'static str> {
        match self {
            Self::RightToLeft => Some("right_to_left"),
            Self::LeftToRight => Some("left_to_right"),
            Self::TopToBottom => Some("top_to_bottom"),
            Self::BottomToTop => Some("bottom_to_top"),
            Self::Other(_) => None,
        }
    }

    /// Config-file name, including the names of older versions, which stored the byte Windows
    /// writes for its "Left" (1) and "Right" (2) buttons.
    fn from_config_name(name: &str) -> Option<Self> {
        match name {
            "left" => Some(Self::LeftToRight),
            "right" => Some(Self::TopToBottom),
            n => Self::ALL.into_iter().find(|d| d.config_name() == Some(n)),
        }
    }

    /// CLI name, e.g. `to-left`.
    pub fn cli_name(self) -> String {
        match self {
            Self::RightToLeft => "to-left".into(),
            Self::LeftToRight => "to-right".into(),
            Self::TopToBottom => "down".into(),
            Self::BottomToTop => "up".into(),
            Self::Other(b) => b.to_string(),
        }
    }

    /// Arrow pointing where the wave moves.
    pub const fn arrow(self) -> &'static str {
        match self {
            Self::RightToLeft => "←",
            Self::LeftToRight => "→",
            Self::TopToBottom => "↓",
            Self::BottomToTop => "↑",
            Self::Other(_) => "?",
        }
    }
}

impl fmt::Display for WaveDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(b) => write!(f, "unknown ({b})"),
            d => write!(f, "{} {}", d.arrow(), d.cli_name()),
        }
    }
}

impl FromStr for WaveDirection {
    type Err = String;

    /// `to-left`, `to-right`, `down`, `up` (or the long names such as `right-to-left`), or the
    /// raw byte 0..=3. Bare `left`/`right` are refused: they could mean either end.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().replace('_', "-").to_ascii_lowercase().as_str() {
            "to-left" | "right-to-left" => Ok(Self::RightToLeft),
            "to-right" | "left-to-right" => Ok(Self::LeftToRight),
            "down" | "top-to-bottom" => Ok(Self::TopToBottom),
            "up" | "bottom-to-top" => Ok(Self::BottomToTop),
            "left" | "right" => Err(format!(
                "direction {s:?} is ambiguous; use to-left or to-right"
            )),
            other => match other.parse::<u8>() {
                Ok(b @ 0..=3) => Ok(Self::from_byte(b)),
                _ => Err(format!(
                    "invalid direction {s:?} (to-left, to-right, down, up or 0..3)"
                )),
            },
        }
    }
}

impl Serialize for WaveDirection {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.config_name() {
            Some(name) => s.serialize_str(name),
            None => s.serialize_u8(self.to_byte()),
        }
    }
}

impl<'de> Deserialize<'de> for WaveDirection {
    /// Accepts the current names, a raw byte, and the older `"left"`, `"right"` and
    /// `{"other": n}` forms, always keeping the byte that was sent before.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Name(String),
            Byte(u8),
            Legacy { other: u8 },
        }
        match Repr::deserialize(d)? {
            Repr::Name(n) => Self::from_config_name(&n)
                .ok_or_else(|| serde::de::Error::custom(format!("unknown wave direction {n:?}"))),
            Repr::Byte(b) | Repr::Legacy { other: b } => Ok(Self::from_byte(b)),
        }
    }
}

/// Lighting of one profile (spec §3 and §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lighting {
    pub effect: Effect,
    /// 1..=5, sent as `brightness − 1`. On hardware byte 0 switches the LEDs **off**, so 1 = off
    /// and 2..=5 = dim to full ([`BRIGHTNESS_OFF`]).
    pub brightness: u8,
    pub direction: WaveDirection,
    /// Effect colour (effects 0..=4).
    pub color: Rgb,
    /// Per-key colours by k, used by [`Effect::Custom`]. Always sent.
    pub key_colors: [Rgb; KEY_COUNT],
    /// Light-bar colours by [`layout::LightBar`] slot.
    pub bar_colors: [Rgb; BAR_COUNT],
}

/// Default effect colours per profile, as in the Windows software (bytes R, G, B).
const DEFAULT_COLORS: [Rgb; PROFILE_COUNT] =
    [Rgb::WHITE, Rgb::BLUE, Rgb::RED, Rgb::GREEN, Rgb::MAGENTA];
/// Default wave direction bytes per profile; the factory blocks hold the same.
const DEFAULT_DIRECTIONS: [u8; PROFILE_COUNT] = [1, 1, 0, 1, 1];
/// [`Lighting::brightness`] value that switches the LEDs off (block byte 0).
pub const BRIGHTNESS_OFF: u8 = 1;

impl Lighting {
    /// Windows defaults for profile `profile` (0-based; out-of-range values use profile 0). The
    /// factory blocks match except for brightness: profile 2 ships with its LEDs off (byte 0).
    pub fn default_for_profile(profile: usize) -> Self {
        let p = if profile < PROFILE_COUNT { profile } else { 0 };
        let mut key_colors = [Rgb::BLACK; KEY_COUNT];
        // The arrow cluster 8/4/5/6/2 (k = 5, 8, 9, 10, 13).
        key_colors[5] = Rgb::RED;
        key_colors[8] = Rgb::BLUE;
        key_colors[9] = Rgb::YELLOW;
        key_colors[10] = Rgb::BLUE;
        key_colors[13] = Rgb::GREEN;
        Self {
            effect: Effect::Static,
            brightness: 5,
            direction: WaveDirection::from_byte(DEFAULT_DIRECTIONS[p]),
            color: DEFAULT_COLORS[p],
            key_colors,
            bar_colors: [Rgb::BLACK; BAR_COUNT],
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if !(1..=5).contains(&self.brightness) {
            return Err(ValidationError::Brightness(self.brightness));
        }
        if self.effect.to_byte() > 10 {
            return Err(ValidationError::Effect(self.effect.to_byte()));
        }
        if self.direction.to_byte() > 3 {
            return Err(ValidationError::Direction(self.direction.to_byte()));
        }
        Ok(())
    }
}

impl Default for Lighting {
    fn default() -> Self {
        Self::default_for_profile(0)
    }
}

/// How often a macro plays (macro block bytes 0..1, big-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    /// Play N times (1..=0xFFFE).
    Times(u16),
    /// Repeat while the key is held (`0xFFFF`).
    WhileHeld,
    /// Repeat until the key is pressed again (`0`).
    Toggle,
}

impl Default for Repeat {
    fn default() -> Self {
        Self::Times(1)
    }
}

impl Repeat {
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::Times(n) => n,
            Self::WhileHeld => 0xFFFF,
            Self::Toggle => 0,
        }
    }

    pub const fn from_u16(v: u16) -> Self {
        match v {
            0 => Self::Toggle,
            0xFFFF => Self::WhileHeld,
            n => Self::Times(n),
        }
    }
}

impl FromStr for Repeat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "held" | "while-held" => Ok(Self::WhileHeld),
            "toggle" => Ok(Self::Toggle),
            n => match n.parse::<u16>() {
                Ok(n @ 1..=0xFFFE) => Ok(Self::Times(n)),
                _ => Err(format!(
                    "invalid repeat {s:?} (1..65534, \"held\" or \"toggle\")"
                )),
            },
        }
    }
}

/// One macro event (spec §5): key `code` goes down or up after waiting `delay_10ms` × 10 ms
/// since the previous event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MacroEvent {
    pub down: bool,
    /// 1..=0x7FFF. In a [`Macro`], 0 is sent as 1, as Windows does.
    pub delay_10ms: u16,
    /// HID usage, vendor code, or a macro-only code ([`codes::MACRO_MOUSE_CODES`]).
    pub code: u8,
}

impl MacroEvent {
    pub const fn down(code: u8, delay_10ms: u16) -> Self {
        Self {
            down: true,
            delay_10ms,
            code,
        }
    }

    pub const fn up(code: u8, delay_10ms: u16) -> Self {
        Self {
            down: false,
            delay_10ms,
            code,
        }
    }

    /// `[b0, b1, code]`: bit 7 of b0 = down, 15-bit delay. Values above 0x7FFF saturate
    /// (validation rejects them before this point). No 0 → 1 clamp here: Windows applies that
    /// only to recorded macros, see [`KeyAssignment::macro_events`].
    pub fn encode(self) -> [u8; 3] {
        let delay = self.delay_10ms.min(MAX_DELAY_10MS);
        let [hi, lo] = delay.to_be_bytes();
        [hi | if self.down { 0x80 } else { 0 }, lo, self.code]
    }

    pub fn decode(bytes: [u8; 3]) -> Self {
        Self {
            down: bytes[0] & 0x80 != 0,
            delay_10ms: u16::from_be_bytes([bytes[0] & 0x7F, bytes[1]]),
            code: bytes[2],
        }
    }
}

/// Delay of every combo event: 1 × 10 ms, as recorded macros get (what played on hardware).
pub const COMBO_DELAY_10MS: u16 = 1;

/// Events uploaded for a key combo (func `0x1D2`) or a synthesised shortcut (Show Desktop,
/// Zoom): the full chord, presses in order and releases in reverse, 10 ms apart, played as an
/// `F8` macro with repeat 1. Ctrl+C becomes `80 01 E0 | 80 01 06 | 00 01 06 | 00 01 E0`.
///
/// Windows writes key-map byte `E8` and uploads only the presses (`80 05 E0 | 80 05 06`), but on
/// this firmware `E8` types nothing, with or without releases; the same chord as an `F8` macro
/// works (hardware session 2026-10-20). Any number of keys fits, up to the event limit.
pub fn combo_events(codes: &[u8]) -> Vec<MacroEvent> {
    codes
        .iter()
        .map(|&c| MacroEvent::down(c, COMBO_DELAY_10MS))
        .chain(
            codes
                .iter()
                .rev()
                .map(|&c| MacroEvent::up(c, COMBO_DELAY_10MS)),
        )
        .collect()
}

fn validate_events(events: &[MacroEvent]) -> Result<(), ValidationError> {
    if events.is_empty() {
        return Err(ValidationError::NoEvents);
    }
    if events.len() > MAX_MACRO_EVENTS {
        return Err(ValidationError::TooManyEvents(events.len()));
    }
    for (index, e) in events.iter().enumerate() {
        if e.code == 0 {
            return Err(ValidationError::ZeroCode { index });
        }
        if e.delay_10ms > MAX_DELAY_10MS {
            return Err(ValidationError::DelayTooLong {
                index,
                delay: e.delay_10ms,
            });
        }
    }
    Ok(())
}

/// A recorded macro, played on the device.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Macro {
    #[serde(default)]
    pub repeat: Repeat,
    pub events: Vec<MacroEvent>,
}

impl Macro {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.repeat == Repeat::Times(0) {
            return Err(ValidationError::RepeatZero);
        }
        validate_events(&self.events)
    }
}

/// What one key does (spec §5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAssignment {
    /// Factory function of the key.
    #[default]
    Default,
    /// Key does nothing (`00`).
    Disabled,
    /// A single HID usage or vendor code ([`codes::KEY_NAMES`]).
    Key(u8),
    /// Keys pressed together, e.g. `[LCTRL, C]`: `F8` plus a macro of the chord (see
    /// [`combo_events`]). A one-key combo is sent as [`KeyAssignment::Key`], as Windows does.
    Combo(Vec<u8>),
    /// A macro: `F8` plus a macro upload.
    Macro(Macro),
    /// A device function (media, mouse, profile, Windows shortcut).
    Function(Function),
    /// Host-side: launch this program when the key's event arrives (`F7`). The app has to be
    /// running for it to work.
    LaunchProgram(String),
}

impl KeyAssignment {
    /// The Windows UI function code this assignment corresponds to; `None`
    /// for [`KeyAssignment::Default`].
    pub fn function_code(&self) -> Option<u16> {
        match self {
            Self::Default => None,
            Self::Disabled => Some(fncode::DISABLE),
            Self::Key(_) => Some(fncode::SINGLE_KEY),
            Self::Combo(c) if c.len() == 1 => Some(fncode::SINGLE_KEY),
            Self::Combo(_) => Some(fncode::COMBO),
            Self::Macro(_) => Some(fncode::MACRO),
            Self::Function(f) => Some(f.code()),
            Self::LaunchProgram(_) => Some(fncode::LAUNCH_PROGRAM),
        }
    }

    /// Macro events uploaded with `0x05`, if this assignment needs any:
    ///
    /// - recorded macros: delay 0 is replaced by 1, as the Windows software does;
    /// - combos and Windows shortcuts (Show Desktop, Zoom): the full chord with delay 1
    ///   ([`combo_events`]), e.g. Win+D = `80 01 E3 80 01 07 00 01 07 00 01 E3`.
    pub fn macro_events(&self) -> Option<(Repeat, Vec<MacroEvent>)> {
        match self {
            Self::Combo(codes) if codes.len() != 1 => Some((Repeat::Times(1), combo_events(codes))),
            Self::Macro(m) => Some((
                m.repeat,
                m.events
                    .iter()
                    .map(|e| MacroEvent {
                        delay_10ms: e.delay_10ms.max(1),
                        ..*e
                    })
                    .collect(),
            )),
            Self::Function(f) => f
                .synthesized_combo()
                .map(|codes| (Repeat::Times(1), combo_events(codes))),
            _ => None,
        }
    }

    /// Whether the host must react to this key's events ([`crate::DeviceEvent::HostKey`]).
    pub fn is_host_action(&self) -> bool {
        matches!(self, Self::LaunchProgram(_))
    }

    /// The usage written by the single-key rule (`0x1D1`): [`KeyAssignment::Key`] or a one-key
    /// combo.
    pub fn single_key_code(&self) -> Option<u8> {
        match self {
            Self::Key(code) => Some(*code),
            Self::Combo(c) if c.len() == 1 => Some(c[0]),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Some(code) = self.single_key_code() {
            return match code {
                0 => Err(ValidationError::ZeroKey),
                0xE8 | 0xF8 | 0xF7 => Err(ValidationError::ReservedCode(code)),
                _ => Ok(()),
            };
        }
        match self {
            Self::Combo(codes) => validate_events(&combo_events(codes)),
            Self::Macro(m) => m.validate(),
            Self::LaunchProgram(path) if path.trim().is_empty() => Err(ValidationError::EmptyPath),
            _ => Ok(()),
        }
    }

    /// Short human-readable description.
    pub fn label(&self) -> String {
        match self {
            Self::Default => "Default".into(),
            Self::Disabled => "Disabled".into(),
            Self::Key(code) => codes::code_label(*code),
            Self::Combo(c) => c
                .iter()
                .map(|&c| codes::code_label(c))
                .collect::<Vec<_>>()
                .join(" + "),
            Self::Macro(m) => format!("Macro ({} events)", m.events.len()),
            Self::Function(f) => f.label().into(),
            Self::LaunchProgram(p) => format!("Launch {p}"),
        }
    }
}

/// One profile (layer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    /// Assignments by k (`row * 4 + col`).
    pub keys: [KeyAssignment; KEY_COUNT],
    pub lighting: Lighting,
    /// Host-side auto switch: executables/app names that activate this profile (max 5 in
    /// Windows; not enforced).
    pub linked_apps: Vec<String>,
}

impl Profile {
    /// Defaults for profile `profile` (0-based).
    pub fn default_for(profile: usize) -> Self {
        Self {
            name: format!("Profile {}", profile + 1),
            keys: Default::default(),
            lighting: Lighting::default_for_profile(profile),
            linked_apps: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        for (k, key) in self.keys.iter().enumerate() {
            key.validate().map_err(|e| e.in_key(k))?;
        }
        self.lighting.validate()
    }

    /// Factory code of key `k` for display, e.g. in a GUI "Default (Num 7)".
    pub fn factory_label(k: usize) -> Option<String> {
        layout::factory_code(k).map(codes::code_label)
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self::default_for(0)
    }
}

/// Global settings (Windows `config.ini [Setting]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 0..=3 → `1 << n` on the wire (experimental, never sent automatically).
    pub polling_rate: u8,
    /// 0/1 (experimental, never sent automatically).
    pub key_mode: u8,
    /// Switch profiles by foreground app (host-side).
    pub auto_switch: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            polling_rate: 3,
            key_mode: 0,
            auto_switch: false,
        }
    }
}

/// The whole configuration as persisted by [`crate::store`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub profiles: [Profile; PROFILE_COUNT],
    /// Last selected profile (0-based).
    pub active_profile: usize,
    pub settings: Settings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profiles: std::array::from_fn(Profile::default_for),
            active_profile: 0,
            settings: Settings::default(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (p, profile) in self.profiles.iter().enumerate() {
            profile.validate().map_err(|e| e.in_profile(p))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_parse_and_display() {
        assert_eq!("ff8000".parse::<Rgb>(), Ok(Rgb::new(0xFF, 0x80, 0)));
        assert_eq!("#00FF00".parse::<Rgb>(), Ok(Rgb::GREEN));
        assert!("12345".parse::<Rgb>().is_err());
        assert!("gg0000".parse::<Rgb>().is_err());
        assert!("ÿÿÿ".parse::<Rgb>().is_err());
        assert_eq!(Rgb::new(1, 2, 0xAB).to_string(), "0102AB");
    }

    #[test]
    fn event_encoding() {
        assert_eq!(MacroEvent::down(0xE3, 0).encode(), [0x80, 0x00, 0xE3]);
        assert_eq!(MacroEvent::up(0x07, 1).encode(), [0x00, 0x01, 0x07]);
        assert_eq!(MacroEvent::down(4, 0x7FFF).encode(), [0xFF, 0xFF, 4]);
        assert_eq!(MacroEvent::up(4, 0x1234).encode(), [0x12, 0x34, 4]);
        // Saturates instead of corrupting the down bit.
        assert_eq!(MacroEvent::up(4, 0xFFFF).encode(), [0x7F, 0xFF, 4]);
        let e = MacroEvent::down(0x2C, 300);
        assert_eq!(MacroEvent::decode(e.encode()), e);
    }

    #[test]
    fn combo_is_a_full_chord_with_delay_1() {
        let enc: Vec<[u8; 3]> = combo_events(&[0xE0, 0xE1, 0x06])
            .iter()
            .map(|e| e.encode())
            .collect();
        assert_eq!(
            enc,
            vec![
                [0x80, 1, 0xE0],
                [0x80, 1, 0xE1],
                [0x80, 1, 0x06],
                [0x00, 1, 0x06],
                [0x00, 1, 0xE1],
                [0x00, 1, 0xE0]
            ]
        );
        assert_eq!(
            KeyAssignment::Combo(vec![0xE1, 0x05]).macro_events(),
            Some((
                Repeat::Times(1),
                vec![
                    MacroEvent::down(0xE1, 1),
                    MacroEvent::down(0x05, 1),
                    MacroEvent::up(0x05, 1),
                    MacroEvent::up(0xE1, 1)
                ]
            ))
        );
        // A one-key combo is a single key, without a macro.
        let one = KeyAssignment::Combo(vec![0x06]);
        assert_eq!(one.function_code(), Some(fncode::SINGLE_KEY));
        assert_eq!(one.macro_events(), None);
        assert_eq!(one.single_key_code(), Some(0x06));
    }

    #[test]
    fn single_key_validation() {
        assert_eq!(KeyAssignment::Key(0x04).validate(), Ok(()));
        assert_eq!(
            KeyAssignment::Key(0).validate(),
            Err(ValidationError::ZeroKey)
        );
        for code in [0xE8, 0xF8, 0xF7] {
            assert_eq!(
                KeyAssignment::Key(code).validate(),
                Err(ValidationError::ReservedCode(code))
            );
        }
        assert_eq!(
            KeyAssignment::Combo(vec![0xF8]).validate(),
            Err(ValidationError::ReservedCode(0xF8))
        );
        assert_eq!(
            KeyAssignment::Combo(vec![0xE0, 0]).validate(),
            Err(ValidationError::ZeroCode { index: 1 })
        );
    }

    #[test]
    fn direction_is_validated() {
        let mut l = Lighting::default();
        for d in WaveDirection::ALL {
            l.direction = d;
            assert_eq!(l.validate(), Ok(()));
        }
        l.direction = WaveDirection::Other(4);
        assert_eq!(l.validate(), Err(ValidationError::Direction(4)));
    }

    #[test]
    fn direction_bytes_and_names() {
        let bytes: Vec<u8> = WaveDirection::ALL.iter().map(|d| d.to_byte()).collect();
        assert_eq!(bytes, vec![0, 1, 2, 3]);
        for d in WaveDirection::ALL {
            assert_eq!(WaveDirection::from_byte(d.to_byte()), d);
            assert_eq!(d.cli_name().parse::<WaveDirection>(), Ok(d));
        }
        assert_eq!(
            "to-left".parse::<WaveDirection>(),
            Ok(WaveDirection::RightToLeft)
        );
        assert_eq!(
            "UP".parse::<WaveDirection>(),
            Ok(WaveDirection::BottomToTop)
        );
        assert_eq!(
            "left_to_right".parse::<WaveDirection>(),
            Ok(WaveDirection::LeftToRight)
        );
        assert_eq!("2".parse::<WaveDirection>(), Ok(WaveDirection::TopToBottom));
        assert!("left".parse::<WaveDirection>().is_err());
        assert!("right".parse::<WaveDirection>().is_err());
        assert!("4".parse::<WaveDirection>().is_err());
    }

    #[test]
    fn direction_json_is_backward_compatible() {
        let parse = |json: &str| serde_json::from_str::<WaveDirection>(json).unwrap();
        // Older config files: "left" was byte 1, "right" byte 2, profile 3 {"other": 0}.
        assert_eq!(parse(r#""left""#).to_byte(), 1);
        assert_eq!(parse(r#""right""#).to_byte(), 2);
        assert_eq!(parse(r#"{"other": 0}"#), WaveDirection::RightToLeft);
        assert_eq!(parse("3"), WaveDirection::BottomToTop);
        assert_eq!(parse("9"), WaveDirection::Other(9));
        assert!(serde_json::from_str::<WaveDirection>(r#""sideways""#).is_err());
        for d in WaveDirection::ALL
            .into_iter()
            .chain([WaveDirection::Other(7)])
        {
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(parse(&json), d, "{json}");
        }
        assert_eq!(
            serde_json::to_string(&WaveDirection::TopToBottom).unwrap(),
            r#""top_to_bottom""#
        );
    }

    #[test]
    fn recorded_macro_delay_zero_becomes_one() {
        let key = KeyAssignment::Macro(Macro {
            repeat: Repeat::Times(1),
            events: vec![MacroEvent::down(4, 0), MacroEvent::up(4, 7)],
        });
        let (_, ev) = key.macro_events().unwrap();
        assert_eq!(ev, vec![MacroEvent::down(4, 1), MacroEvent::up(4, 7)]);
    }

    #[test]
    fn macro_limits() {
        let ok = Macro {
            repeat: Repeat::Times(1),
            events: vec![MacroEvent::down(4, 1); 240],
        };
        assert_eq!(ok.validate(), Ok(()));
        let too_many = Macro {
            events: vec![MacroEvent::down(4, 1); 241],
            ..ok.clone()
        };
        assert_eq!(
            too_many.validate(),
            Err(ValidationError::TooManyEvents(241))
        );
        let zero = Macro {
            events: vec![MacroEvent::down(0, 1)],
            ..ok.clone()
        };
        assert_eq!(zero.validate(), Err(ValidationError::ZeroCode { index: 0 }));
        let long = Macro {
            events: vec![MacroEvent::down(4, 0x8000)],
            ..ok.clone()
        };
        assert!(matches!(
            long.validate(),
            Err(ValidationError::DelayTooLong { .. })
        ));
        let empty = Macro {
            events: vec![],
            ..ok.clone()
        };
        assert_eq!(empty.validate(), Err(ValidationError::NoEvents));
        let rep0 = Macro {
            repeat: Repeat::Times(0),
            ..ok
        };
        assert_eq!(rep0.validate(), Err(ValidationError::RepeatZero));
        // A combo uploads a press and a release per key.
        assert_eq!(KeyAssignment::Combo(vec![4; 120]).validate(), Ok(()));
        assert_eq!(
            KeyAssignment::Combo(vec![4; 121]).validate(),
            Err(ValidationError::TooManyEvents(242))
        );
    }

    #[test]
    fn profile_validation_names_the_key() {
        let mut p = Profile::default_for(0);
        p.keys[3] = KeyAssignment::Combo(vec![]);
        let err = p.validate().unwrap_err();
        assert_eq!(err.to_string(), "key 4: macro or combo has no events");
        p.keys[3] = KeyAssignment::Default;
        p.lighting.brightness = 0;
        assert_eq!(p.validate(), Err(ValidationError::Brightness(0)));
    }

    #[test]
    fn repeat_values() {
        assert_eq!(Repeat::from_u16(0), Repeat::Toggle);
        assert_eq!(Repeat::from_u16(0xFFFF), Repeat::WhileHeld);
        assert_eq!(Repeat::Times(3).to_u16(), 3);
        assert_eq!("held".parse::<Repeat>(), Ok(Repeat::WhileHeld));
        assert_eq!("5".parse::<Repeat>(), Ok(Repeat::Times(5)));
        assert!("0".parse::<Repeat>().is_err());
    }

    #[test]
    fn effect_and_direction_parse() {
        assert_eq!("fade-in".parse::<Effect>(), Ok(Effect::FadeIn));
        assert_eq!("last_key".parse::<Effect>(), Ok(Effect::LastKey));
        assert_eq!("9".parse::<Effect>(), Ok(Effect::Other(9)));
        assert_eq!("5".parse::<Effect>(), Ok(Effect::Wave));
        for e in Effect::ALL {
            assert_eq!(Effect::from_byte(e.to_byte()), e);
        }
        assert_eq!(
            "to-right".parse::<WaveDirection>(),
            Ok(WaveDirection::LeftToRight)
        );
        assert_eq!("0".parse::<WaveDirection>(), Ok(WaveDirection::RightToLeft));
    }

    #[test]
    fn defaults_per_profile() {
        let c = Config::default();
        let colors: Vec<Rgb> = c.profiles.iter().map(|p| p.lighting.color).collect();
        assert_eq!(
            colors,
            vec![Rgb::WHITE, Rgb::BLUE, Rgb::RED, Rgb::GREEN, Rgb::MAGENTA]
        );
        assert_eq!(c.profiles[2].lighting.direction, WaveDirection::RightToLeft);
        assert_eq!(c.profiles[0].lighting.direction, WaveDirection::LeftToRight);
        assert_eq!(c.profiles[0].lighting.key_colors[9], Rgb::YELLOW);
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn json_round_trip() {
        let mut c = Config::default();
        c.profiles[1].keys[0] = KeyAssignment::Key(0x04);
        c.profiles[1].keys[1] = KeyAssignment::Combo(vec![0xE0, 0x06]);
        c.profiles[1].keys[2] = KeyAssignment::Macro(Macro {
            repeat: Repeat::WhileHeld,
            events: vec![MacroEvent::down(4, 1), MacroEvent::up(4, 5)],
        });
        c.profiles[1].keys[3] = KeyAssignment::Function(Function::VolumeUp);
        c.profiles[1].keys[4] = KeyAssignment::LaunchProgram("/Applications/Calculator.app".into());
        c.profiles[1].keys[5] = KeyAssignment::Disabled;
        c.profiles[1].lighting.effect = Effect::Other(9);
        c.profiles[4].linked_apps.push("Safari".into());
        let json = serde_json::to_string_pretty(&c).unwrap();
        assert!(json.contains("\"color\": \"0000FF\""));
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn old_config_files_still_load() {
        let old = r#"{"profiles": [
            {"lighting": {"direction": "right", "brightness": 1}},
            {"lighting": {"direction": "left"}},
            {"lighting": {"direction": {"other": 0}}},
            {},
            {}
        ]}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        let bytes: Vec<u8> = c.profiles[..3]
            .iter()
            .map(|p| p.lighting.direction.to_byte())
            .collect();
        assert_eq!(bytes, vec![2, 1, 0]);
        assert_eq!(c.profiles[0].lighting.brightness, BRIGHTNESS_OFF);
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn partial_json_uses_defaults() {
        let c: Config = serde_json::from_str(r#"{"active_profile": 2}"#).unwrap();
        assert_eq!(c.active_profile, 2);
        assert_eq!(c.profiles[3].lighting.color, Rgb::GREEN);
    }
}
