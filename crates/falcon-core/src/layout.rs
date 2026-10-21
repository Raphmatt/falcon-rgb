//! Key and LED indexing (spec §4).
//!
//! Three index spaces exist:
//!
//! - **k** = `row * 4 + col` (row 0..4 top to bottom, col 0..3 left to right): the UI order, used
//!   by [`crate::Profile::keys`] and [`crate::Lighting::key_colors`].
//! - **hw** = `col * 5 + row`: key-map position, macro `a1`, host-key events.
//! - **LED** = `row * 5 + col`: colour planes. Column 4 of the LED grid holds the light bars.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Keys on the pad.
pub const KEY_COUNT: usize = 20;
/// Rows of keys.
pub const ROWS: usize = 5;
/// Columns of keys.
pub const COLS: usize = 4;
/// Entries in the key map and in each colour plane.
pub const SLOT_COUNT: usize = 25;
/// Light-bar segments.
pub const BAR_COUNT: usize = 4;

/// k → hw (`col * 5 + row`), the order the Windows software uses.
pub const K_TO_HW: [u8; KEY_COUNT] = [
    0, 5, 10, 15, 1, 6, 11, 16, 2, 7, 12, 17, 3, 8, 13, 18, 4, 9, 14, 19,
];

/// k → LED index (`row * 5 + col`), confirmed on hardware.
pub const K_TO_LED: [u8; KEY_COUNT] = [
    0, 1, 2, 3, 5, 6, 7, 8, 10, 11, 12, 13, 15, 16, 17, 18, 20, 21, 22, 23,
];

/// Light-bar slot (see [`LightBar`]) → LED index. LED 14 is never written.
pub const BAR_TO_LED: [u8; BAR_COUNT] = [9, 4, 24, 19];

/// Factory key map, block bytes 0..24 by **hw** index. Slots 20..24 are
/// extra (probably Fn-layer defaults) and are never patched.
pub const FACTORY_KEYMAP: [u8; SLOT_COUNT] = [
    0x53, 0x5F, 0x5C, 0x59, 0x62, // col 0: NumLock, KP7, KP4, KP1, KP0
    0xB4, 0x60, 0x5D, 0x5A, 0xBA, // col 1: Layer1, KP8, KP5, KP2, "00"
    0xB5, 0x61, 0x5E, 0x5B, 0x63, // col 2: Layer2, KP9, KP6, KP3, KP.
    0x54, 0x55, 0x56, 0x57, 0x58, // col 3: KP/, KP*, KP-, KP+, KPEnter
    0xBB, 0x00, 0xBC, 0xB9, 0x00, // extra: LED brightness, -, LED mode, Layer cycle, -
];

/// Printed legends by k (from `key-layout.json`).
pub const KEY_LEGENDS: [&str; KEY_COUNT] = [
    "NumLock",
    "Layer 1",
    "Layer 2",
    "Num /",
    "Num 7",
    "Num 8",
    "Num 9",
    "Num *",
    "Num 4",
    "Num 5",
    "Num 6",
    "Num -",
    "Num 1",
    "Num 2",
    "Num 3",
    "Num +",
    "Num 0",
    "Num 00",
    "Num .",
    "Num Enter",
];

/// (row, col) of key `k`, or `None` if out of range.
pub const fn row_col(k: usize) -> Option<(usize, usize)> {
    if k < KEY_COUNT {
        Some((k / COLS, k % COLS))
    } else {
        None
    }
}

/// Hardware index of key `k`.
pub fn k_to_hw(k: usize) -> Option<u8> {
    K_TO_HW.get(k).copied()
}

/// Key `k` for a hardware index (e.g. from [`crate::DeviceEvent::HostKey`]).
pub fn hw_to_k(hw: u8) -> Option<usize> {
    K_TO_HW.iter().position(|&h| h == hw)
}

/// LED index of key `k`.
pub fn k_to_led(k: usize) -> Option<u8> {
    K_TO_LED.get(k).copied()
}

/// Factory key-map byte of key `k`.
pub fn factory_code(k: usize) -> Option<u8> {
    k_to_hw(k).map(|hw| FACTORY_KEYMAP[hw as usize])
}

/// One light-bar segment; the discriminant is the slot in [`crate::Lighting::bar_colors`]
/// (colour entries 20..23 of the Windows model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightBar {
    LeftUpper = 0,
    RightUpper = 1,
    LeftLower = 2,
    RightLower = 3,
}

impl LightBar {
    pub const ALL: [Self; BAR_COUNT] = [
        Self::LeftUpper,
        Self::RightUpper,
        Self::LeftLower,
        Self::RightLower,
    ];

    /// Slot in [`crate::Lighting::bar_colors`].
    pub const fn slot(self) -> usize {
        self as usize
    }

    /// LED index in the colour planes.
    pub const fn led(self) -> u8 {
        BAR_TO_LED[self as usize]
    }

    /// Short name: `lu`, `ru`, `ll`, `lr`.
    pub const fn short_name(self) -> &'static str {
        match self {
            Self::LeftUpper => "lu",
            Self::RightUpper => "ru",
            Self::LeftLower => "ll",
            Self::RightLower => "lr",
        }
    }
}

impl fmt::Display for LightBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.short_name())
    }
}

impl FromStr for LightBar {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|b| b.short_name().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown light bar {s:?} (expected lu, ru, ll or lr)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hw_is_col_major() {
        for k in 0..KEY_COUNT {
            let (row, col) = row_col(k).unwrap();
            assert_eq!(k_to_hw(k).unwrap() as usize, col * 5 + row);
            assert_eq!(k_to_led(k).unwrap() as usize, row * 5 + col);
        }
        assert_eq!(row_col(20), None);
        assert_eq!(k_to_hw(20), None);
    }

    #[test]
    fn hw_round_trip_is_a_permutation() {
        for k in 0..KEY_COUNT {
            assert_eq!(hw_to_k(k_to_hw(k).unwrap()), Some(k));
        }
        let mut hws = K_TO_HW;
        hws.sort_unstable();
        assert_eq!(hws.to_vec(), (0..20).collect::<Vec<u8>>());
        assert_eq!(hw_to_k(20), None);
    }

    #[test]
    fn leds_are_disjoint_and_skip_14() {
        let mut leds: Vec<u8> = K_TO_LED.iter().chain(BAR_TO_LED.iter()).copied().collect();
        leds.sort_unstable();
        let expected: Vec<u8> = (0..25).filter(|&l| l != 14).collect();
        assert_eq!(leds, expected);
    }

    #[test]
    fn factory_map_matches_spec_table() {
        // Spec §4, row-major by k.
        let by_k = [
            0x53, 0xB4, 0xB5, 0x54, 0x5F, 0x60, 0x61, 0x55, 0x5C, 0x5D, 0x5E, 0x56, 0x59, 0x5A,
            0x5B, 0x57, 0x62, 0xBA, 0x63, 0x58,
        ];
        for (k, &code) in by_k.iter().enumerate() {
            assert_eq!(factory_code(k), Some(code), "k = {k}");
        }
        assert_eq!(&FACTORY_KEYMAP[20..], &[0xBB, 0x00, 0xBC, 0xB9, 0x00]);
    }

    #[test]
    fn light_bars() {
        assert_eq!(LightBar::LeftUpper.led(), 9);
        assert_eq!(LightBar::RightUpper.led(), 4);
        assert_eq!(LightBar::LeftLower.led(), 24);
        assert_eq!(LightBar::RightLower.led(), 19);
        assert_eq!("LR".parse::<LightBar>(), Ok(LightBar::RightLower));
        assert!("xx".parse::<LightBar>().is_err());
    }
}
