//! Key codes, UI function codes and the function → device-byte table (spec §5).
//!
//! The key map holds one byte per key: either a standard HID keyboard usage (`0x04..=0xE7`) or a
//! vendor code (layers, LED, media, mouse). The vendor codes reuse the HID "keypad extended"
//! range, so the vendor meaning wins when naming a code.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A named key code usable as a single-key assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyName {
    pub code: u8,
    /// Canonical CLI/JSON name (upper case, no spaces).
    pub name: &'static str,
    /// Human-readable label for UIs.
    pub label: &'static str,
}

const fn k(code: u8, name: &'static str, label: &'static str) -> KeyName {
    KeyName { code, name, label }
}

/// Every named code: standard HID keyboard usages, then the vendor codes of spec §5.
pub const KEY_NAMES: &[KeyName] = &[
    k(0x04, "A", "A"),
    k(0x05, "B", "B"),
    k(0x06, "C", "C"),
    k(0x07, "D", "D"),
    k(0x08, "E", "E"),
    k(0x09, "F", "F"),
    k(0x0A, "G", "G"),
    k(0x0B, "H", "H"),
    k(0x0C, "I", "I"),
    k(0x0D, "J", "J"),
    k(0x0E, "K", "K"),
    k(0x0F, "L", "L"),
    k(0x10, "M", "M"),
    k(0x11, "N", "N"),
    k(0x12, "O", "O"),
    k(0x13, "P", "P"),
    k(0x14, "Q", "Q"),
    k(0x15, "R", "R"),
    k(0x16, "S", "S"),
    k(0x17, "T", "T"),
    k(0x18, "U", "U"),
    k(0x19, "V", "V"),
    k(0x1A, "W", "W"),
    k(0x1B, "X", "X"),
    k(0x1C, "Y", "Y"),
    k(0x1D, "Z", "Z"),
    k(0x1E, "1", "1"),
    k(0x1F, "2", "2"),
    k(0x20, "3", "3"),
    k(0x21, "4", "4"),
    k(0x22, "5", "5"),
    k(0x23, "6", "6"),
    k(0x24, "7", "7"),
    k(0x25, "8", "8"),
    k(0x26, "9", "9"),
    k(0x27, "0", "0"),
    k(0x28, "ENTER", "Enter"),
    k(0x29, "ESC", "Esc"),
    k(0x2A, "BACKSPACE", "Backspace"),
    k(0x2B, "TAB", "Tab"),
    k(0x2C, "SPACE", "Space"),
    k(0x2D, "MINUS", "-"),
    k(0x2E, "EQUAL", "="),
    k(0x2F, "LBRACKET", "["),
    k(0x30, "RBRACKET", "]"),
    k(0x31, "BACKSLASH", "\\"),
    k(0x32, "NONUS_HASH", "# (non-US)"),
    k(0x33, "SEMICOLON", ";"),
    k(0x34, "QUOTE", "'"),
    k(0x35, "GRAVE", "`"),
    k(0x36, "COMMA", ","),
    k(0x37, "PERIOD", "."),
    k(0x38, "SLASH", "/"),
    k(0x39, "CAPSLOCK", "Caps Lock"),
    k(0x3A, "F1", "F1"),
    k(0x3B, "F2", "F2"),
    k(0x3C, "F3", "F3"),
    k(0x3D, "F4", "F4"),
    k(0x3E, "F5", "F5"),
    k(0x3F, "F6", "F6"),
    k(0x40, "F7", "F7"),
    k(0x41, "F8", "F8"),
    k(0x42, "F9", "F9"),
    k(0x43, "F10", "F10"),
    k(0x44, "F11", "F11"),
    k(0x45, "F12", "F12"),
    k(0x46, "PRINTSCREEN", "Print Screen"),
    k(0x47, "SCROLLLOCK", "Scroll Lock"),
    k(0x48, "PAUSE", "Pause"),
    k(0x49, "INSERT", "Insert"),
    k(0x4A, "HOME", "Home"),
    k(0x4B, "PAGEUP", "Page Up"),
    k(0x4C, "DELETE", "Delete"),
    k(0x4D, "END", "End"),
    k(0x4E, "PAGEDOWN", "Page Down"),
    k(0x4F, "RIGHT", "Right"),
    k(0x50, "LEFT", "Left"),
    k(0x51, "DOWN", "Down"),
    k(0x52, "UP", "Up"),
    k(0x53, "NUMLOCK", "Num Lock"),
    k(0x54, "KP_SLASH", "Num /"),
    k(0x55, "KP_ASTERISK", "Num *"),
    k(0x56, "KP_MINUS", "Num -"),
    k(0x57, "KP_PLUS", "Num +"),
    k(0x58, "KP_ENTER", "Num Enter"),
    k(0x59, "KP_1", "Num 1"),
    k(0x5A, "KP_2", "Num 2"),
    k(0x5B, "KP_3", "Num 3"),
    k(0x5C, "KP_4", "Num 4"),
    k(0x5D, "KP_5", "Num 5"),
    k(0x5E, "KP_6", "Num 6"),
    k(0x5F, "KP_7", "Num 7"),
    k(0x60, "KP_8", "Num 8"),
    k(0x61, "KP_9", "Num 9"),
    k(0x62, "KP_0", "Num 0"),
    k(0x63, "KP_PERIOD", "Num ."),
    k(0x64, "NONUS_BACKSLASH", "\\ (non-US)"),
    k(0x65, "APPLICATION", "Menu"),
    k(0x67, "KP_EQUAL", "Num ="),
    k(0x68, "F13", "F13"),
    k(0x69, "F14", "F14"),
    k(0x6A, "F15", "F15"),
    k(0x6B, "F16", "F16"),
    k(0x6C, "F17", "F17"),
    k(0x6D, "F18", "F18"),
    k(0x6E, "F19", "F19"),
    k(0x6F, "F20", "F20"),
    k(0x70, "F21", "F21"),
    k(0x71, "F22", "F22"),
    k(0x72, "F23", "F23"),
    k(0x73, "F24", "F24"),
    k(0xE0, "LCTRL", "Left Ctrl"),
    k(0xE1, "LSHIFT", "Left Shift"),
    k(0xE2, "LALT", "Left Alt/Option"),
    k(0xE3, "LGUI", "Left Win/Cmd"),
    k(0xE4, "RCTRL", "Right Ctrl"),
    k(0xE5, "RSHIFT", "Right Shift"),
    k(0xE6, "RALT", "Right Alt/Option"),
    k(0xE7, "RGUI", "Right Win/Cmd"),
    // Vendor codes (spec §5), valid as single-key assignments.
    k(0xA4, "MEDIA_PLAYER", "Media Player"),
    k(0xA5, "MEDIA_STOP", "Stop"),
    k(0xA6, "MEDIA_PREV", "Previous Track"),
    k(0xA7, "MEDIA_PLAY", "Play/Pause"),
    k(0xA8, "MEDIA_NEXT", "Next Track"),
    k(0xA9, "MEDIA_MUTE", "Mute"),
    k(0xAA, "MEDIA_VOLDOWN", "Volume -"),
    k(0xAB, "MEDIA_VOLUP", "Volume +"),
    k(0xB4, "LAYER1", "Layer 1"),
    k(0xB5, "LAYER2", "Layer 2"),
    k(0xB6, "LAYER3", "Layer 3"),
    k(0xB7, "LAYER4", "Layer 4"),
    k(0xB8, "LAYER5", "Layer 5"),
    k(0xB9, "LAYER_CYCLE", "Layer Cycle"),
    k(0xBA, "KP_00", "Num 00"),
    k(0xBB, "LED_BRIGHTNESS", "LED Brightness"),
    k(0xBC, "LED_MODE", "LED Mode Cycle"),
    k(0xBE, "MOUSE_LEFT", "Left Click"),
    k(0xBF, "MOUSE_RIGHT", "Right Click"),
    k(0xC0, "MOUSE_MIDDLE", "Middle Click"),
    k(0xC3, "MOUSE_BACK", "Backward"),
    k(0xC4, "MOUSE_FORWARD", "Forward"),
    k(0xFA, "LOCK_ALL", "Lock/Unlock All Keys"),
];

/// Extra spellings accepted by [`parse_key_name`] (after normalisation).
const ALIASES: &[(&str, u8)] = &[
    ("CTRL", 0xE0),
    ("CONTROL", 0xE0),
    ("SHIFT", 0xE1),
    ("ALT", 0xE2),
    ("OPT", 0xE2),
    ("OPTION", 0xE2),
    ("LOPT", 0xE2),
    ("ROPT", 0xE6),
    ("GUI", 0xE3),
    ("WIN", 0xE3),
    ("LWIN", 0xE3),
    ("RWIN", 0xE7),
    ("CMD", 0xE3),
    ("LCMD", 0xE3),
    ("RCMD", 0xE7),
    ("META", 0xE3),
    ("SUPER", 0xE3),
    ("RETURN", 0x28),
    ("ESCAPE", 0x29),
    ("BKSP", 0x2A),
    ("DEL", 0x4C),
    ("INS", 0x49),
    ("PGUP", 0x4B),
    ("PGDN", 0x4E),
    ("MENU", 0x65),
    ("-", 0x2D),
    ("=", 0x2E),
    ("[", 0x2F),
    ("]", 0x30),
    ("\\", 0x31),
    (";", 0x33),
    ("'", 0x34),
    ("`", 0x35),
    (",", 0x36),
    (".", 0x37),
    ("/", 0x38),
    ("KP/", 0x54),
    ("KP*", 0x55),
    ("KP-", 0x56),
    ("KP+", 0x57),
    ("KP.", 0x63),
    ("NUM_LOCK", 0x53),
];

/// Codes that mean something else inside a macro event list (spec §5): mouse buttons.
/// `BC..BF` (mouse moves) are deliberately not offered until their encoding is known.
pub const MACRO_MOUSE_CODES: &[KeyName] = &[
    k(0xB7, "BTN_LEFT", "Mouse Left (macro)"),
    k(0xB8, "BTN_RIGHT", "Mouse Right (macro)"),
    k(0xB9, "BTN_MIDDLE", "Mouse Middle (macro)"),
];

/// The 136-entry single-key palette of the Windows UI, in its order.
/// Entry 0 (`0x00`) means "Disable".
pub const SINGLE_KEY_PALETTE: [u8; 136] = [
    0x00, 0xFA, 0xBC, 0xBB, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xA4, 0xAA, 0xAB, 0xA9, 0xA6, 0xA8,
    0xA7, 0xA5, 0xBE, 0xC0, 0xBF, 0xC3, 0xC4, 0x29, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41,
    0x42, 0x43, 0x44, 0x45, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x35, 0x1E, 0x1F, 0x20,
    0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2D, 0x2E, 0x2A, 0x46, 0x47, 0x48, 0x53, 0x54, 0x55,
    0x56, 0x2B, 0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13, 0x2F, 0x30, 0x31, 0x49,
    0x4A, 0x4B, 0x5F, 0x60, 0x61, 0x57, 0x39, 0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F,
    0x33, 0x34, 0x28, 0x4C, 0x4D, 0x4E, 0x5C, 0x5D, 0x5E, 0xE1, 0x1D, 0x1B, 0x06, 0x19, 0x05, 0x11,
    0x10, 0x36, 0x38, 0x37, 0xE5, 0x52, 0x59, 0x5A, 0x5B, 0x58, 0xE0, 0xE3, 0xE2, 0x2C, 0xE6, 0xE7,
    0x65, 0xE4, 0x50, 0x51, 0x4F, 0x62, 0xBA, 0x63,
];

fn normalize(s: &str) -> String {
    s.trim()
        .chars()
        .filter(|c| *c != '_' && *c != ' ')
        .flat_map(char::to_uppercase)
        .collect()
}

fn parse_hex_byte(s: &str) -> Option<u8> {
    let digits = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .filter(|d| !d.is_empty() && d.len() <= 2)?;
    u8::from_str_radix(digits, 16).ok()
}

/// Looks up a key name (case-insensitive; `_` and spaces ignored) or a `0xNN` literal.
pub fn parse_key_name(s: &str) -> Option<u8> {
    if let Some(code) = parse_hex_byte(s.trim()) {
        return Some(code);
    }
    let n = normalize(s);
    KEY_NAMES
        .iter()
        .find(|e| normalize(e.name) == n)
        .map(|e| e.code)
        .or_else(|| {
            ALIASES
                .iter()
                .find(|(a, _)| normalize(a) == n)
                .map(|&(_, c)| c)
        })
}

/// Like [`parse_key_name`], but inside a macro, where `BTN_LEFT`/`BTN_RIGHT`/`BTN_MIDDLE` name the
/// mouse-button codes.
pub fn parse_macro_code(s: &str) -> Option<u8> {
    let n = normalize(s);
    MACRO_MOUSE_CODES
        .iter()
        .find(|e| normalize(e.name) == n)
        .map(|e| e.code)
        .or_else(|| parse_key_name(s))
}

/// The [`KeyName`] entry for a code, if any.
pub fn key_name(code: u8) -> Option<&'static KeyName> {
    KEY_NAMES.iter().find(|e| e.code == code)
}

/// Canonical name of a key-map code, or `0xNN`.
pub fn code_name(code: u8) -> String {
    match key_name(code) {
        Some(e) => e.name.to_owned(),
        None => format!("0x{code:02X}"),
    }
}

/// Human-readable label of a key-map code, or `0xNN`.
pub fn code_label(code: u8) -> String {
    match key_name(code) {
        Some(e) => e.label.to_owned(),
        None if code == 0 => "Disabled".to_owned(),
        None => format!("0x{code:02X}"),
    }
}

/// Canonical name of a code inside a macro event (mouse buttons take precedence).
pub fn macro_code_name(code: u8) -> String {
    match MACRO_MOUSE_CODES.iter().find(|e| e.code == code) {
        Some(e) => e.name.to_owned(),
        None => code_name(code),
    }
}

/// UI function codes of the Windows software (spec §5). These carry the Windows model's
/// numbering so the device-byte table applies unchanged.
pub mod fncode {
    /// Profile cycle / up / down.
    pub const PROFILE_CYCLE: u16 = 0x104;
    pub const PROFILE_UP: u16 = 0x105;
    pub const PROFILE_DOWN: u16 = 0x106;
    /// Launch a program on the host (device byte F7, then a host-key event).
    pub const LAUNCH_PROGRAM: u16 = 0x180;
    /// Single key: the key-map byte is the HID usage itself.
    pub const SINGLE_KEY: u16 = 0x1D1;
    /// Key + modifiers: a macro upload (`F8`; Windows writes `E8`, which this firmware ignores).
    pub const COMBO: u16 = 0x1D2;
    /// Disable: `00`.
    pub const DISABLE: u16 = 0x1D3;
    /// Recorded macro (any code `0x200..=0xFFFF`): `F8` plus a macro upload.
    pub const MACRO: u16 = 0x200;
    /// Windows shortcuts synthesised as macros.
    pub const SHOW_DESKTOP: u16 = 0x1E1;
    pub const ZOOM_IN: u16 = 0x1E3;
    pub const ZOOM_OUT: u16 = 0x1E4;
}

/// Function → device byte table of the Windows software (spec §5), checked first.
pub const FUNCTION_TABLE: &[(u16, u8)] = &[
    (0x105, 0xF7),
    (0x106, 0xF8),
    (0x104, 0xF9),
    (0x121, 0xC7),
    (0x122, 0xC7),
    (0x123, 0xC7),
    (0x141, 0xC4),
    (0x142, 0xC5),
    (0x143, 0xC6),
    (0x161, 0xC4),
    (0x162, 0xC5),
    (0x163, 0xC6),
    (0x180, 0xF7),
    (0xDF, 0xDF),
    (0xDB, 0xDB),
    (0x1A1, 0xCE),
    (0x1A2, 0xCF),
    (0x1A4, 0xD5),
    (0x1A3, 0xD0),
    (0x1A7, 0xD2),
    (0x1A8, 0xD1),
    (0x1A5, 0xD3),
    (0x1A6, 0xD4),
    (0x1C4, 0xA7),
    (0x1C7, 0xA5),
    (0x1C5, 0xA6),
    (0x1C6, 0xA8),
    (0x1C1, 0xAB),
    (0x1C2, 0xAA),
    (0x1C3, 0xA9),
    (0x1E2, 0xDD),
];

/// Device key-map byte for a UI function code, with the precedence the Windows software uses (spec §5):
///
/// 1. [`FUNCTION_TABLE`];
/// 2. `0x1D1` with a non-zero `single_key` → the raw usage (a zero usage falls
///    through and keeps the factory byte);
/// 3. `0x200..=0xFFFF` → `F8`;
/// 4. `0x1D2`, `0x1E1`, `0x1E3`, `0x1E4` → `F8` as well. Windows writes `E8` here, but on
///    hardware an `E8` key types nothing while the same events as an `F8` macro play (hardware
///    session 2026-10-20), so combos and shortcuts are uploaded as ordinary macros;
/// 5. `0x1D3` → `00`;
/// 6. otherwise `None`: keep the factory byte.
pub fn function_device_byte(code: u16, single_key: u8) -> Option<u8> {
    if let Some(&(_, byte)) = FUNCTION_TABLE.iter().find(|(c, _)| *c == code) {
        return Some(byte);
    }
    match code {
        fncode::SINGLE_KEY if single_key != 0 => Some(single_key),
        fncode::MACRO..=0xFFFF
        | fncode::COMBO
        | fncode::SHOW_DESKTOP
        | fncode::ZOOM_IN
        | fncode::ZOOM_OUT => Some(0xF8),
        fncode::DISABLE => Some(0x00),
        _ => None,
    }
}

/// Whether a function code needs a `0x05` macro upload (spec §5).
pub fn function_needs_macro(code: u16) -> bool {
    matches!(
        code,
        fncode::COMBO | fncode::SHOW_DESKTOP | fncode::ZOOM_IN | fncode::ZOOM_OUT
    ) || code >= fncode::MACRO
}

/// Device-handled functions that can be assigned to a key (the Windows "function" lists).
/// Launch program, macros, combos, single keys and disable are separate
/// [`crate::KeyAssignment`] variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    ProfileCycle,
    ProfileUp,
    ProfileDown,
    MouseLeft,
    MouseRight,
    MouseMiddle,
    MouseDoubleClick,
    MouseBack,
    MouseForward,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    PrevTrack,
    NextTrack,
    Stop,
    ShowDesktop,
    Calculator,
    ZoomIn,
    ZoomOut,
}

impl Function {
    pub const ALL: [Self; 24] = [
        Self::ProfileCycle,
        Self::ProfileUp,
        Self::ProfileDown,
        Self::MouseLeft,
        Self::MouseRight,
        Self::MouseMiddle,
        Self::MouseDoubleClick,
        Self::MouseBack,
        Self::MouseForward,
        Self::ScrollUp,
        Self::ScrollDown,
        Self::ScrollLeft,
        Self::ScrollRight,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::Mute,
        Self::PlayPause,
        Self::PrevTrack,
        Self::NextTrack,
        Self::Stop,
        Self::ShowDesktop,
        Self::Calculator,
        Self::ZoomIn,
        Self::ZoomOut,
    ];

    /// UI function code.
    pub const fn code(self) -> u16 {
        match self {
            Self::ProfileCycle => 0x104,
            Self::ProfileUp => 0x105,
            Self::ProfileDown => 0x106,
            Self::MouseLeft => 0x1A1,
            Self::MouseRight => 0x1A2,
            Self::MouseMiddle => 0x1A3,
            Self::MouseDoubleClick => 0x1A4,
            Self::MouseBack => 0x1A5,
            Self::MouseForward => 0x1A6,
            Self::ScrollUp => 0x1A7,
            Self::ScrollDown => 0x1A8,
            Self::ScrollLeft => 0x1A9,
            Self::ScrollRight => 0x1AA,
            Self::VolumeUp => 0x1C1,
            Self::VolumeDown => 0x1C2,
            Self::Mute => 0x1C3,
            Self::PlayPause => 0x1C4,
            Self::PrevTrack => 0x1C5,
            Self::NextTrack => 0x1C6,
            Self::Stop => 0x1C7,
            Self::ShowDesktop => 0x1E1,
            Self::Calculator => 0x1E2,
            Self::ZoomIn => 0x1E3,
            Self::ZoomOut => 0x1E4,
        }
    }

    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.code() == code)
    }

    /// Kebab-case CLI name, e.g. `volume-up`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::ProfileCycle => "profile-cycle",
            Self::ProfileUp => "profile-up",
            Self::ProfileDown => "profile-down",
            Self::MouseLeft => "mouse-left",
            Self::MouseRight => "mouse-right",
            Self::MouseMiddle => "mouse-middle",
            Self::MouseDoubleClick => "mouse-double-click",
            Self::MouseBack => "mouse-back",
            Self::MouseForward => "mouse-forward",
            Self::ScrollUp => "scroll-up",
            Self::ScrollDown => "scroll-down",
            Self::ScrollLeft => "scroll-left",
            Self::ScrollRight => "scroll-right",
            Self::VolumeUp => "volume-up",
            Self::VolumeDown => "volume-down",
            Self::Mute => "mute",
            Self::PlayPause => "play-pause",
            Self::PrevTrack => "prev-track",
            Self::NextTrack => "next-track",
            Self::Stop => "stop",
            Self::ShowDesktop => "show-desktop",
            Self::Calculator => "calculator",
            Self::ZoomIn => "zoom-in",
            Self::ZoomOut => "zoom-out",
        }
    }

    /// Human-readable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ProfileCycle => "Profile Cycle",
            Self::ProfileUp => "Profile Up",
            Self::ProfileDown => "Profile Down",
            Self::MouseLeft => "Left Click",
            Self::MouseRight => "Right Click",
            Self::MouseMiddle => "Scroll Click",
            Self::MouseDoubleClick => "Double Click",
            Self::MouseBack => "Backward",
            Self::MouseForward => "Forward",
            Self::ScrollUp => "Scroll Up",
            Self::ScrollDown => "Scroll Down",
            Self::ScrollLeft => "Scroll Left",
            Self::ScrollRight => "Scroll Right",
            Self::VolumeUp => "Volume Up",
            Self::VolumeDown => "Volume Down",
            Self::Mute => "Mute",
            Self::PlayPause => "Play/Pause",
            Self::PrevTrack => "Previous Track",
            Self::NextTrack => "Next Track",
            Self::Stop => "Stop",
            Self::ShowDesktop => "Show Desktop",
            Self::Calculator => "Calculator",
            Self::ZoomIn => "Zoom In",
            Self::ZoomOut => "Zoom Out",
        }
    }

    /// Key-map byte, or `None` when Windows leaves the factory byte (scroll left/right, which
    /// are missing from the table).
    pub fn device_byte(self) -> Option<u8> {
        function_device_byte(self.code(), 0)
    }

    /// Keys of the chord Windows synthesises for the shortcut functions (spec §5); uploaded as
    /// an `F8` macro, see [`crate::model::combo_events`].
    pub fn synthesized_combo(self) -> Option<&'static [u8]> {
        match self {
            // LGUI + D
            Self::ShowDesktop => Some(&[0xE3, 0x07]),
            // LCTRL + vendor wheel codes C0/C1 [HW-VERIFY]
            Self::ZoomIn => Some(&[0xE0, 0xC0]),
            Self::ZoomOut => Some(&[0xE0, 0xC1]),
            _ => None,
        }
    }
}

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Function {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let n = s.trim().replace('_', "-").to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|f| f.name() == n)
            .ok_or_else(|| format!("unknown function {s:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_table_first() {
        // 0x105 is in the table even though nothing else would match.
        assert_eq!(function_device_byte(0x105, 0x04), Some(0xF7));
        // 0x1E2 (Calculator) is in the table, so it is not a macro like the other shortcuts.
        assert_eq!(function_device_byte(0x1E2, 0), Some(0xDD));
        assert_eq!(function_device_byte(0x180, 0), Some(0xF7));
        assert_eq!(function_device_byte(0x1C7, 0), Some(0xA5));
    }

    #[test]
    fn precedence_rules() {
        assert_eq!(function_device_byte(fncode::SINGLE_KEY, 0x2C), Some(0x2C));
        assert_eq!(function_device_byte(fncode::SINGLE_KEY, 0), None);
        assert_eq!(function_device_byte(0x200, 0), Some(0xF8));
        assert_eq!(function_device_byte(0xFFFF, 0), Some(0xF8));
        // Combos and shortcuts are F8 macros (Windows: E8, which the firmware ignores).
        for code in [0x1D2, 0x1E1, 0x1E3, 0x1E4] {
            assert_eq!(function_device_byte(code, 0x04), Some(0xF8), "{code:#x}");
        }
        assert_eq!(function_device_byte(fncode::DISABLE, 0x04), Some(0x00));
        // Not in the table and no rule: keep the factory byte.
        assert_eq!(function_device_byte(0x1A9, 0), None);
        assert_eq!(function_device_byte(0x1AA, 0), None);
        assert_eq!(function_device_byte(0, 0), None);
        assert_eq!(function_device_byte(0x101, 0), None);
    }

    #[test]
    fn macro_needed() {
        assert!(function_needs_macro(0x1D2));
        assert!(function_needs_macro(0x1E1));
        assert!(function_needs_macro(0x200));
        assert!(!function_needs_macro(0x1E2));
        assert!(!function_needs_macro(0x1D1));
        assert!(!function_needs_macro(0x180));
    }

    #[test]
    fn every_function_round_trips() {
        for f in Function::ALL {
            assert_eq!(Function::from_code(f.code()), Some(f));
            assert_eq!(f.name().parse::<Function>(), Ok(f));
        }
        assert_eq!("VOLUME_UP".parse::<Function>(), Ok(Function::VolumeUp));
        assert_eq!(Function::ProfileDown.device_byte(), Some(0xF8));
        assert_eq!(Function::ScrollLeft.device_byte(), None);
    }

    #[test]
    fn key_names() {
        assert_eq!(parse_key_name("lctrl"), Some(0xE0));
        assert_eq!(parse_key_name("Ctrl"), Some(0xE0));
        assert_eq!(parse_key_name("kp_7"), Some(0x5F));
        assert_eq!(parse_key_name("KP7"), Some(0x5F));
        assert_eq!(parse_key_name("kp-"), Some(0x56));
        assert_eq!(parse_key_name("0x2c"), Some(0x2C));
        assert_eq!(parse_key_name("layer 3"), Some(0xB6));
        assert_eq!(parse_key_name("nope"), None);
        assert_eq!(parse_key_name("0x"), None);
        assert_eq!(parse_macro_code("btn_left"), Some(0xB7));
        assert_eq!(macro_code_name(0xB7), "BTN_LEFT");
        assert_eq!(code_name(0xB7), "LAYER4");
        assert_eq!(code_name(0x01), "0x01");
        assert_eq!(code_label(0), "Disabled");
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<String> = KEY_NAMES.iter().map(|e| normalize(e.name)).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n);
        let mut codes: Vec<u8> = KEY_NAMES.iter().map(|e| e.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n);
    }

    #[test]
    fn palette_entries_are_named() {
        for &code in &SINGLE_KEY_PALETTE[1..] {
            assert!(key_name(code).is_some(), "{code:#04x}");
        }
    }
}
