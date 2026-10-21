//! egui keys → HID keyboard usages, for macro recording inside the app window.
//!
//! Limitations (egui exposes no more): the numeric keypad reports the same keys as the digit
//! row, so it records as `1`..`0`; media keys, Fn and keys egui does not know are skipped; the
//! OS may swallow shortcuts such as Cmd+Q or Cmd+Tab before the app sees them.

use eframe::egui::Key;

/// The HID usage for `key` (US layout positions), or `None` if it cannot be recorded.
pub fn hid_usage(key: Key) -> Option<u8> {
    use Key::*;
    let code = match key {
        A => 0x04,
        B => 0x05,
        C => 0x06,
        D => 0x07,
        E => 0x08,
        F => 0x09,
        G => 0x0A,
        H => 0x0B,
        I => 0x0C,
        J => 0x0D,
        K => 0x0E,
        L => 0x0F,
        M => 0x10,
        N => 0x11,
        O => 0x12,
        P => 0x13,
        Q => 0x14,
        R => 0x15,
        S => 0x16,
        T => 0x17,
        U => 0x18,
        V => 0x19,
        W => 0x1A,
        X => 0x1B,
        Y => 0x1C,
        Z => 0x1D,
        Num1 => 0x1E,
        Num2 => 0x1F,
        Num3 => 0x20,
        Num4 => 0x21,
        Num5 => 0x22,
        Num6 => 0x23,
        Num7 => 0x24,
        Num8 => 0x25,
        Num9 => 0x26,
        Num0 => 0x27,
        Enter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus => 0x2D,
        Equals | Plus => 0x2E,
        OpenBracket | OpenCurlyBracket => 0x2F,
        CloseBracket | CloseCurlyBracket => 0x30,
        Backslash | Pipe => 0x31,
        Semicolon | Colon => 0x33,
        Quote => 0x34,
        Backtick => 0x35,
        Comma => 0x36,
        Period => 0x37,
        Slash | Questionmark => 0x38,
        CapsLock => 0x39,
        F1 => 0x3A,
        F2 => 0x3B,
        F3 => 0x3C,
        F4 => 0x3D,
        F5 => 0x3E,
        F6 => 0x3F,
        F7 => 0x40,
        F8 => 0x41,
        F9 => 0x42,
        F10 => 0x43,
        F11 => 0x44,
        F12 => 0x45,
        PrintScreen => 0x46,
        ScrollLock => 0x47,
        Pause => 0x48,
        Insert => 0x49,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        NumLock => 0x53,
        IntlBackslash => 0x64,
        Menu => 0x65,
        F13 => 0x68,
        F14 => 0x69,
        F15 => 0x6A,
        F16 => 0x6B,
        F17 => 0x6C,
        F18 => 0x6D,
        F19 => 0x6E,
        F20 => 0x6F,
        F21 => 0x70,
        F22 => 0x71,
        F23 => 0x72,
        F24 => 0x73,
        ControlLeft => 0xE0,
        ShiftLeft => 0xE1,
        AltLeft => 0xE2,
        SuperLeft => 0xE3,
        ControlRight => 0xE4,
        ShiftRight => 0xE5,
        AltRight => 0xE6,
        SuperRight => 0xE7,
        _ => return None,
    };
    Some(code)
}

/// Delay between two recorded events in the device's 10 ms units, `1..=0x7FFF`.
pub fn delay_units(seconds: f64) -> u16 {
    let units = (seconds * 100.0).round();
    if units.is_nan() || units < 1.0 {
        1
    } else {
        units.min(f64::from(falcon_core::protocol::MAX_DELAY_10MS)) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use falcon_core::codes::code_name;

    #[test]
    fn letters_digits_and_modifiers() {
        assert_eq!(hid_usage(Key::A), Some(0x04));
        assert_eq!(hid_usage(Key::Z), Some(0x1D));
        assert_eq!(hid_usage(Key::Num1), Some(0x1E));
        assert_eq!(hid_usage(Key::Num0), Some(0x27));
        assert_eq!(hid_usage(Key::ShiftLeft), Some(0xE1));
        assert_eq!(hid_usage(Key::SuperRight), Some(0xE7));
        assert_eq!(hid_usage(Key::F12), Some(0x45));
        assert_eq!(hid_usage(Key::F24), Some(0x73));
        assert_eq!(hid_usage(Key::ArrowUp), Some(0x52));
    }

    #[test]
    fn shifted_symbols_record_their_base_key() {
        assert_eq!(hid_usage(Key::Plus), hid_usage(Key::Equals));
        assert_eq!(hid_usage(Key::Colon), hid_usage(Key::Semicolon));
        assert_eq!(hid_usage(Key::Questionmark), hid_usage(Key::Slash));
    }

    #[test]
    fn unmapped_keys() {
        for k in [Key::F25, Key::AudioVolumeUp, Key::Copy, Key::Fn] {
            assert_eq!(hid_usage(k), None, "{k:?}");
        }
    }

    #[test]
    fn every_mapped_code_has_a_name() {
        for k in Key::ALL {
            if let Some(code) = hid_usage(*k) {
                assert!(!code_name(code).starts_with("0x"), "{k:?} → {code:#04x}");
            }
        }
    }

    #[test]
    fn delays_round_and_clamp() {
        assert_eq!(delay_units(0.0), 1);
        assert_eq!(delay_units(0.004), 1);
        assert_eq!(delay_units(0.256), 26);
        assert_eq!(delay_units(1.0), 100);
        assert_eq!(delay_units(1e9), 0x7FFF);
        assert_eq!(delay_units(f64::NAN), 1);
    }
}
