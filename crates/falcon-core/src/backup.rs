//! Whole-keypad backups (spec §9): the raw block of every layer (key map and lighting, exactly
//! as `0x82` returns it) plus the macros the app knows for keys whose device byte is `F8`.
//!
//! Macro contents live in a separate store on the keypad that cannot be read back, so they come
//! from the [`Config`], not from the device. A restore writes each block back unchanged with
//! `0x02`, re-uploads those macros and commits ([`crate::encode::plan_restore`]).
//!
//! The file is versioned JSON, the same for the CLI, the desktop app and the browser. Old
//! single-layer `falcon dump --out pN.bin` files (raw 264-byte `0x82` replies) load as
//! one-layer backups.

use serde::{Deserialize, Serialize};

use crate::codes::Function;
use crate::encode::{self, BLOCK_CONSTANT, decode_lighting, key_device_byte, offset};
use crate::error::{Error, Result};
use crate::hex;
use crate::layout::{self, K_TO_HW, KEY_COUNT};
use crate::model::{BRIGHTNESS_OFF, Config, KeyAssignment, Profile};
use crate::protocol::{self, Block, PROFILE_COUNT, cmd};

/// `format` of every backup file.
pub const FORMAT: &str = "falcon-rgb-backup";
/// Current file version; newer files are refused.
pub const VERSION: u32 = 1;
/// Device byte of a key that plays a macro.
const MACRO_BYTE: u8 = 0xF8;

/// A backup of some or all layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Backup {
    pub format: String,
    pub version: u32,
    /// `VID:PID` of the keypad it came from.
    pub device: String,
    /// Seconds since the Unix epoch, if the creator knew the time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_unix: Option<u64>,
    /// Active layer when the backup was made, 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_layer: Option<usize>,
    pub layers: Vec<LayerBackup>,
}

/// One layer: its raw block and the macros for its `F8` keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerBackup {
    /// 1-based.
    pub layer: usize,
    /// The 200-byte block as hex (`"53 5F 5C …"`).
    #[serde(with = "block_hex")]
    pub block: Block,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<MacroBackup>,
}

/// A macro (or combo) for key `key` (1-based, the app's key numbering), taken from the app
/// config at backup time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroBackup {
    pub key: usize,
    pub assignment: KeyAssignment,
}

mod block_hex {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    use crate::protocol::{BLOCK_LEN, Block};

    pub fn serialize<S: Serializer>(b: &Block, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&crate::hex::to_hex(b))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Block, D::Error> {
        let text = String::deserialize(d)?;
        let bytes = crate::hex::parse(&text).map_err(D::Error::custom)?;
        bytes.try_into().map_err(|v: Vec<u8>| {
            D::Error::custom(format!("block has {} bytes, expected {BLOCK_LEN}", v.len()))
        })
    }
}

fn bad(msg: impl Into<String>) -> Error {
    Error::Backup(msg.into())
}

impl Backup {
    /// A backup of `blocks` (index = 0-based layer), taking macros for `F8` keys from `config`.
    pub fn from_blocks(
        blocks: &[(usize, Block)],
        active: Option<usize>,
        config: &Config,
        created_unix: Option<u64>,
    ) -> Self {
        let layers = blocks
            .iter()
            .map(|&(p, block)| LayerBackup {
                layer: p + 1,
                block,
                macros: config
                    .profiles
                    .get(p)
                    .map(|profile| macros_for(&block, profile))
                    .unwrap_or_default(),
            })
            .collect();
        Self {
            format: FORMAT.into(),
            version: VERSION,
            device: format!("{:04X}:{:04X}", protocol::VID, protocol::PID),
            created_unix,
            active_layer: active.map(|p| p + 1),
            layers,
        }
    }

    /// One layer from a raw `0x82` reply (`falcon dump --out pN.bin`); the layer comes from the
    /// reply header `07 82 0N`.
    pub fn from_dump(reply: &[u8]) -> Result<Self> {
        let block = protocol::parse_block(reply)?;
        if reply[1] != cmd::READ_BLOCK {
            return Err(bad(format!(
                "not a 0x82 dump (byte 1 is {:#04x})",
                reply[1]
            )));
        }
        let layer = usize::from(reply[2]);
        if !(1..=PROFILE_COUNT).contains(&layer) {
            return Err(bad(format!("dump names layer {layer}, expected 1..=5")));
        }
        let mut b = Self::from_blocks(&[(layer - 1, block)], None, &Config::default(), None);
        // A raw dump carries no macros, and the default config must not invent any.
        b.layers[0].macros.clear();
        b.validate()?;
        Ok(b)
    }

    /// A backup file: JSON, or a raw dump.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let trimmed = data
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .map(|i| &data[i..]);
        if trimmed.is_some_and(|d| d.first() == Some(&b'{')) {
            let b: Self = serde_json::from_slice(data)?;
            b.validate()?;
            Ok(b)
        } else {
            Self::from_dump(data)
        }
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Refuses anything that is not a plausible backup of this keypad: wrong format or version,
    /// layers outside 1..=5 or repeated, blocks without the `01 00 05` constant, macros for keys
    /// that are not macro keys in the block, invalid macros.
    pub fn validate(&self) -> Result<()> {
        if self.format != FORMAT {
            return Err(bad(format!(
                "format is {:?}, expected {FORMAT:?}",
                self.format
            )));
        }
        if self.version > VERSION {
            return Err(bad(format!(
                "file version {} is newer than this app ({VERSION})",
                self.version
            )));
        }
        if self.layers.is_empty() {
            return Err(bad("no layers in the backup"));
        }
        let mut seen = [false; PROFILE_COUNT];
        for l in &self.layers {
            if !(1..=PROFILE_COUNT).contains(&l.layer) {
                return Err(bad(format!("layer {} out of range 1..=5", l.layer)));
            }
            if std::mem::replace(&mut seen[l.layer - 1], true) {
                return Err(bad(format!("layer {} appears twice", l.layer)));
            }
            l.validate()?;
        }
        Ok(())
    }

    /// The layer with this 1-based number.
    pub fn layer(&self, layer: usize) -> Option<&LayerBackup> {
        self.layers.iter().find(|l| l.layer == layer)
    }
}

impl LayerBackup {
    /// See [`Backup::validate`].
    pub fn validate(&self) -> Result<()> {
        let c = &self.block[offset::CONSTANT..offset::CONSTANT + 3];
        if c != BLOCK_CONSTANT {
            return Err(bad(format!(
                "layer {}: bytes 131..133 are {}, expected 01 00 05 (not a keypad block)",
                self.layer,
                hex::to_hex(c)
            )));
        }
        for m in &self.macros {
            let k = m
                .key
                .checked_sub(1)
                .filter(|&k| k < KEY_COUNT)
                .ok_or_else(|| {
                    bad(format!(
                        "layer {}: macro key {} out of range",
                        self.layer, m.key
                    ))
                })?;
            if self.block[usize::from(K_TO_HW[k])] != MACRO_BYTE {
                return Err(bad(format!(
                    "layer {}: key {} has a macro but is not a macro key in the block",
                    self.layer, m.key
                )));
            }
            if !matches!(encode::macro_block(&m.assignment), Ok(Some(_))) {
                return Err(bad(format!(
                    "layer {}: key {}: not a valid macro",
                    self.layer, m.key
                )));
            }
        }
        Ok(())
    }

    /// One line for confirmations: effect, colour, brightness, changed keys and macro notes.
    pub fn summary(&self) -> String {
        let lighting = decode_lighting(&self.block);
        let keymap = encode::decode_keymap(&self.block);
        let changed = (0..KEY_COUNT)
            .filter(|&k| Some(keymap[k]) != layout::factory_code(k))
            .count();
        let brightness = if lighting.brightness == BRIGHTNESS_OFF {
            "off".to_owned()
        } else {
            lighting.brightness.to_string()
        };
        let mut s = format!(
            "layer {}: {}, colour {}, brightness {brightness}, {changed} key(s) differ from factory",
            self.layer, lighting.effect, lighting.color
        );
        let list = |keys: Vec<usize>| {
            keys.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !self.macros.is_empty() {
            let keys = self.macros.iter().map(|m| m.key).collect();
            s += &format!(", macros for key {} (from the app config)", list(keys));
        }
        let missing = self.macro_keys_without_macro();
        if !missing.is_empty() {
            s += &format!(
                ", macro key {} without macro (keeps whatever macro the keypad has stored)",
                list(missing)
            );
        }
        s
    }

    /// Macro keys (1-based) of the block that have no macro in the backup: they will play
    /// whatever macro the keypad currently stores for them.
    pub fn macro_keys_without_macro(&self) -> Vec<usize> {
        (0..KEY_COUNT)
            .filter(|&k| self.block[usize::from(K_TO_HW[k])] == MACRO_BYTE)
            .filter(|&k| !self.macros.iter().any(|m| m.key == k + 1))
            .map(|k| k + 1)
            .collect()
    }

    /// Updates `profile` to what this layer holds: the lighting, and every key whose device
    /// byte differs from what `profile` would send. Keys that already match keep their richer
    /// config value (names, macros, functions). Returns keys (1-based) that could not be
    /// represented and were left as configured (macro keys without a macro in the backup).
    pub fn apply_to_profile(&self, profile: &mut Profile) -> Vec<usize> {
        let lighting = decode_lighting(&self.block);
        if lighting.validate().is_ok() {
            profile.lighting = lighting;
        }
        let mut unknown = Vec::new();
        for (k, &hw) in K_TO_HW.iter().enumerate() {
            let byte = self.block[usize::from(hw)];
            if key_device_byte(k, &profile.keys[k]) == Some(byte) {
                continue;
            }
            if let Some(m) = self.macros.iter().find(|m| m.key == k + 1) {
                profile.keys[k] = m.assignment.clone();
            } else if byte == MACRO_BYTE {
                unknown.push(k + 1);
            } else if let Some(a) = decode_key_byte(k, byte) {
                profile.keys[k] = a;
            } else {
                unknown.push(k + 1);
            }
        }
        unknown
    }
}

/// The config assignment that produces device byte `byte` on key `k`, for the bytes that map
/// back unambiguously.
fn decode_key_byte(k: usize, byte: u8) -> Option<KeyAssignment> {
    if Some(byte) == layout::factory_code(k) {
        return Some(KeyAssignment::Default);
    }
    if byte == 0 {
        return Some(KeyAssignment::Disabled);
    }
    if let Some(f) = Function::ALL
        .into_iter()
        .find(|f| f.device_byte() == Some(byte))
    {
        return Some(KeyAssignment::Function(f));
    }
    let a = KeyAssignment::Key(byte);
    (a.validate().is_ok() && key_device_byte(k, &a) == Some(byte)).then_some(a)
}

/// Macros from `profile` for the keys that are macro keys (`F8`) in `block`.
fn macros_for(block: &Block, profile: &Profile) -> Vec<MacroBackup> {
    (0..KEY_COUNT)
        .filter(|&k| block[usize::from(K_TO_HW[k])] == MACRO_BYTE)
        .filter(|&k| matches!(encode::macro_block(&profile.keys[k]), Ok(Some(_))))
        .map(|k| MacroBackup {
            key: k + 1,
            assignment: profile.keys[k].clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Macro, MacroEvent, Repeat};
    use crate::transport::SimulatedDevice;

    const P1: &[u8] = include_bytes!("../tests/data/p1.bin");

    fn macro_assignment() -> KeyAssignment {
        KeyAssignment::Macro(Macro {
            repeat: Repeat::Times(1),
            events: vec![MacroEvent::down(0x0B, 1), MacroEvent::up(0x0B, 1)],
        })
    }

    #[test]
    fn dump_files_load_as_one_layer() {
        let b = Backup::from_bytes(P1).unwrap();
        assert_eq!(b.layers.len(), 1);
        assert_eq!(b.layers[0].layer, 1);
        assert_eq!(&b.layers[0].block[..], &P1[8..208]);
        assert!(b.layers[0].macros.is_empty());

        let mut not_82 = P1.to_vec();
        not_82[1] = 0x81;
        assert!(Backup::from_bytes(&not_82).is_err());
        let mut bad_layer = P1.to_vec();
        bad_layer[2] = 9;
        assert!(Backup::from_bytes(&bad_layer).is_err());
        assert!(Backup::from_bytes(&P1[..100]).is_err());
    }

    #[test]
    fn json_round_trip_with_macros() {
        let mut config = Config::default();
        let mut block = SimulatedDevice::initial_block(2);
        block[usize::from(K_TO_HW[5])] = MACRO_BYTE;
        config.profiles[2].keys[5] = macro_assignment();
        // A macro in the config for a key that is not a macro key on the keypad is not taken.
        config.profiles[2].keys[6] = macro_assignment();
        let b = Backup::from_blocks(&[(2, block)], Some(2), &config, Some(1_700_000_000));
        assert_eq!(b.active_layer, Some(3));
        assert_eq!(b.layers[0].macros.len(), 1);
        assert_eq!(b.layers[0].macros[0].key, 6);

        let json = b.to_json().unwrap();
        assert!(json.contains("\"format\": \"falcon-rgb-backup\""));
        let back = Backup::from_bytes(json.as_bytes()).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn validation() {
        let good = Backup::from_bytes(P1).unwrap();
        let mut b = good.clone();
        b.layers[0].block[offset::CONSTANT] = 0;
        assert!(b.validate().is_err(), "constant");
        let mut b = good.clone();
        b.layers.push(b.layers[0].clone());
        assert!(b.validate().is_err(), "duplicate layer");
        let mut b = good.clone();
        b.layers[0].layer = 6;
        assert!(b.validate().is_err(), "layer range");
        let mut b = good.clone();
        b.version = VERSION + 1;
        assert!(b.validate().is_err(), "version");
        let mut b = good.clone();
        b.layers[0].macros.push(MacroBackup {
            key: 1,
            assignment: macro_assignment(),
        });
        assert!(b.validate().is_err(), "macro on a non-macro key");
        let mut b = good;
        b.layers.clear();
        assert!(b.validate().is_err(), "empty");
        assert!(Backup::from_bytes(b"{\"format\": \"other\"}").is_err());
    }

    #[test]
    fn apply_to_profile_decodes_keys_and_lighting() {
        let mut block = SimulatedDevice::initial_block(0);
        block[usize::from(K_TO_HW[0])] = 0x04; // key 1 → a
        block[usize::from(K_TO_HW[1])] = 0x00; // key 2 → disabled
        block[usize::from(K_TO_HW[2])] = 0xA7; // key 3 → play/pause
        block[usize::from(K_TO_HW[3])] = MACRO_BYTE; // key 4 → macro, unknown
        block[offset::EFFECT] = 0;
        block[offset::COLOR] = 0x12;
        let layer = LayerBackup {
            layer: 1,
            block,
            macros: vec![],
        };
        let mut p = Profile::default_for(0);
        let unknown = layer.apply_to_profile(&mut p);
        assert_eq!(unknown, vec![4]);
        assert_eq!(p.keys[0], KeyAssignment::Key(0x04));
        assert_eq!(p.keys[1], KeyAssignment::Disabled);
        assert_eq!(p.keys[2], KeyAssignment::Function(Function::PlayPause));
        assert_eq!(p.keys[5], KeyAssignment::Default);
        assert_eq!(p.lighting.color.r, 0x12);
        assert_eq!(layer.macro_keys_without_macro(), vec![4]);
    }
}

#[cfg(test)]
mod real_keymap {
    use super::*;
    use crate::hex;

    /// Layer 1 as read from a real keypad (browser log, 2026-10-21): media keys and disabled
    /// keys set with the app; bytes 0..24 of the block, by hw index.
    const LAYER1_KEYMAP: &str =
        "00 A6 00 00 00 B4 A7 00 00 00 B5 A8 BB 00 00 00 AB AA 00 00 BB 00 BC B9 00";

    #[test]
    fn every_key_of_a_real_layer_is_recognised() {
        let mut block = crate::transport::SimulatedDevice::initial_block(0);
        block[..25].copy_from_slice(&hex::parse(LAYER1_KEYMAP).unwrap());
        let layer = LayerBackup {
            layer: 1,
            block,
            macros: vec![],
        };
        let mut p = Profile::default_for(0);
        assert_eq!(layer.apply_to_profile(&mut p), Vec::<usize>::new());
        let by_hw = |hw: u8| &p.keys[layout::hw_to_k(hw).unwrap()];
        assert_eq!(by_hw(0), &KeyAssignment::Disabled);
        assert_eq!(by_hw(1), &KeyAssignment::Function(Function::PrevTrack));
        assert_eq!(by_hw(5), &KeyAssignment::Default, "Layer 1 key is factory");
        assert_eq!(by_hw(6), &KeyAssignment::Function(Function::PlayPause));
        assert_eq!(by_hw(11), &KeyAssignment::Function(Function::NextTrack));
        assert_eq!(by_hw(16), &KeyAssignment::Function(Function::VolumeUp));
        assert_eq!(by_hw(17), &KeyAssignment::Function(Function::VolumeDown));
        // Writing the adopted profile back produces exactly the keypad's key map.
        let mut out = block;
        encode::apply_keymap(&p.keys, &mut out);
        assert_eq!(&out[..25], &block[..25]);
        assert_eq!(by_hw(12), &KeyAssignment::Key(0xBB), "LED Brightness key");
    }
}
