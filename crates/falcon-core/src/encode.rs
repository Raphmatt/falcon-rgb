//! Model → device bytes, and pure command plans (spec §2, §3, §5).
//!
//! Plans are lists of [`Step`]s. The real device, the simulator and `--dry-run` all execute the
//! same plans through [`crate::Falcon::execute`], so what the dry run prints is what the device
//! would receive.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::codes::function_device_byte;
use crate::error::{Result, ValidationError};
use crate::layout::{self, FACTORY_KEYMAP, K_TO_HW, K_TO_LED, KEY_COUNT, LightBar};
use crate::model::{
    Effect, KeyAssignment, Lighting, MacroEvent, Profile, Repeat, Rgb, WaveDirection,
};
use crate::protocol::{
    self, Block, MACRO_BLOCK_LEN, MACRO_CHUNK_LEN, MACRO_CHUNKS, MAX_MACRO_EVENTS, check_profile,
    delay,
};

/// Block offsets (spec §3).
pub mod offset {
    /// Key map, 25 bytes by hw index.
    pub const KEYMAP: usize = 0;
    /// A second copy of the factory key map on hardware; meaning unknown, preserved.
    pub const KEYMAP_COPY: usize = 25;
    /// Red plane, 25 bytes by LED index.
    pub const RED: usize = 50;
    /// Green plane.
    pub const GREEN: usize = 75;
    /// Blue plane.
    pub const BLUE: usize = 100;
    /// Lighting effect.
    pub const EFFECT: usize = 125;
    /// Brightness − 1: 0 = LEDs off, 1..=4 = dim to full.
    pub const BRIGHTNESS: usize = 126;
    /// Wave direction 0..=3 ([`crate::WaveDirection`]).
    pub const DIRECTION: usize = 127;
    /// Effect colour R, G, B.
    pub const COLOR: usize = 128;
    /// Constant `01 00 05`, always written.
    pub const CONSTANT: usize = 131;
    /// From here to the end the hardware returns firmware RAM (pointers such as `54 09 00 10`);
    /// preserved.
    pub const RAM_GARBAGE: usize = 152;
}

/// The constant Windows writes at [`offset::CONSTANT`].
pub const BLOCK_CONSTANT: [u8; 3] = [0x01, 0x00, 0x05];

/// A key's 722-byte macro block (spec §5).
pub type MacroBlock = [u8; MACRO_BLOCK_LEN];

/// Key-map byte for key `k`: the function → device-byte precedence of spec §5, falling back to
/// the factory byte.
pub fn key_device_byte(k: usize, assignment: &KeyAssignment) -> Option<u8> {
    let factory = layout::factory_code(k)?;
    let single = assignment.single_key_code().unwrap_or(0);
    Some(
        assignment
            .function_code()
            .and_then(|code| function_device_byte(code, single))
            .unwrap_or(factory),
    )
}

/// Writes key-map bytes 0..24 like the Windows software: all 25 bytes start from the
/// factory template (so slots 20..24 are reset to `BB 00 BC B9 00` on every full download),
/// then the 20 keys are patched by hw index.
pub fn apply_keymap(keys: &[KeyAssignment; KEY_COUNT], block: &mut Block) {
    block[offset::KEYMAP..offset::KEYMAP + layout::SLOT_COUNT].copy_from_slice(&FACTORY_KEYMAP);
    for (k, assignment) in keys.iter().enumerate() {
        let hw = K_TO_HW[k] as usize;
        block[offset::KEYMAP + hw] = key_device_byte(k, assignment).unwrap_or(FACTORY_KEYMAP[hw]);
    }
}

fn set_led(block: &mut Block, led: u8, c: Rgb) {
    let led = led as usize;
    block[offset::RED + led] = c.r;
    block[offset::GREEN + led] = c.g;
    block[offset::BLUE + led] = c.b;
}

fn get_led(block: &Block, led: u8) -> Rgb {
    let led = led as usize;
    Rgb::new(
        block[offset::RED + led],
        block[offset::GREEN + led],
        block[offset::BLUE + led],
    )
}

/// Patches the lighting fields: colour planes (keys and light bars, LED 14 untouched), effect,
/// brightness, direction, effect colour and the `01 00 05` constant.
pub fn apply_lighting(l: &Lighting, block: &mut Block) {
    for (k, &c) in l.key_colors.iter().enumerate() {
        set_led(block, K_TO_LED[k], c);
    }
    for (bar, &c) in LightBar::ALL.iter().zip(l.bar_colors.iter()) {
        set_led(block, bar.led(), c);
    }
    block[offset::EFFECT] = l.effect.to_byte();
    // Windows treats 0 or > 5 as 5; validation rejects them before we get here.
    let brightness = if (1..=5).contains(&l.brightness) {
        l.brightness
    } else {
        5
    };
    block[offset::BRIGHTNESS] = brightness - 1;
    // Validation rejects > 3; clamp as a fallback.
    block[offset::DIRECTION] = l.direction.to_byte().min(3);
    block[offset::COLOR..offset::COLOR + 3].copy_from_slice(&[l.color.r, l.color.g, l.color.b]);
    block[offset::CONSTANT..offset::CONSTANT + 3].copy_from_slice(&BLOCK_CONSTANT);
}

/// Read-modify-write of a whole profile: key map (template + keys) and lighting. Bytes 25..49
/// (key map copy) and 134..199 (zeros, then firmware RAM from 152), and LED 14, keep whatever
/// the device returned.
pub fn apply_profile_to_block(profile: &Profile, block: &mut Block) {
    apply_keymap(&profile.keys, block);
    apply_lighting(&profile.lighting, block);
}

/// Reads the lighting fields back out of a block (inverse of [`apply_lighting`]).
pub fn decode_lighting(block: &Block) -> Lighting {
    let raw_brightness = block[offset::BRIGHTNESS];
    Lighting {
        effect: Effect::from_byte(block[offset::EFFECT]),
        brightness: if raw_brightness <= 4 {
            raw_brightness + 1
        } else {
            5
        },
        direction: WaveDirection::from_byte(block[offset::DIRECTION]),
        color: Rgb::new(
            block[offset::COLOR],
            block[offset::COLOR + 1],
            block[offset::COLOR + 2],
        ),
        key_colors: std::array::from_fn(|k| get_led(block, K_TO_LED[k])),
        bar_colors: std::array::from_fn(|i| get_led(block, LightBar::ALL[i].led())),
    }
}

/// Key-map byte of each key k (decoded from hw order).
pub fn decode_keymap(block: &Block) -> [u8; KEY_COUNT] {
    std::array::from_fn(|k| block[offset::KEYMAP + K_TO_HW[k] as usize])
}

/// Encodes a macro block: repeat (big-endian), then up to 240 events of 3 bytes, zero-padded.
pub fn encode_macro(repeat: Repeat, events: &[MacroEvent]) -> Result<MacroBlock, ValidationError> {
    if events.len() > MAX_MACRO_EVENTS {
        return Err(ValidationError::TooManyEvents(events.len()));
    }
    let mut block = [0u8; MACRO_BLOCK_LEN];
    block[..2].copy_from_slice(&repeat.to_u16().to_be_bytes());
    for (i, e) in events.iter().enumerate() {
        block[2 + i * 3..5 + i * 3].copy_from_slice(&e.encode());
    }
    Ok(block)
}

/// The macro block a key needs, if any (combo, macro, Windows shortcut). Validates first.
pub fn macro_block(assignment: &KeyAssignment) -> Result<Option<MacroBlock>, ValidationError> {
    assignment.validate()?;
    assignment
        .macro_events()
        .map(|(repeat, events)| encode_macro(repeat, &events))
        .transpose()
}

/// Splits a macro block into the three 256-byte `0x05` payloads; chunk 3 is zero-padded.
pub fn macro_chunks(block: &MacroBlock) -> [[u8; MACRO_CHUNK_LEN]; MACRO_CHUNKS] {
    std::array::from_fn(|i| {
        let mut chunk = [0u8; MACRO_CHUNK_LEN];
        let start = i * MACRO_CHUNK_LEN;
        let end = (start + MACRO_CHUNK_LEN).min(MACRO_BLOCK_LEN);
        chunk[..end - start].copy_from_slice(&block[start..end]);
        chunk
    })
}

/// Whether the device is only written to, or written and then read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// SetFeature, then wait `delay_after`.
    Set,
    /// SetFeature, wait `delay_after`, GetFeature (same length), wait `delay_after` again.
    SetThenGet,
}

/// One packet of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// Complete report including the report ID at `[0]`.
    pub packet: Vec<u8>,
    pub kind: StepKind,
    pub delay_after: Duration,
    /// What the step does, for logs and dry runs.
    pub label: String,
}

impl Step {
    pub fn set(
        packet: impl Into<Vec<u8>>,
        delay_after: Duration,
        label: impl Into<String>,
    ) -> Self {
        Self {
            packet: packet.into(),
            kind: StepKind::Set,
            delay_after,
            label: label.into(),
        }
    }

    pub fn query(packet: impl Into<Vec<u8>>, label: impl Into<String>) -> Self {
        let packet = packet.into();
        let delay_after = protocol::query_delay(packet.get(1).copied().unwrap_or(0));
        Self {
            packet,
            kind: StepKind::SetThenGet,
            delay_after,
            label: label.into(),
        }
    }
}

/// `0x82`: read the block of `profile` (Set, 200 ms, Get, 200 ms).
pub fn plan_read_block(profile: usize) -> Result<Step> {
    Ok(Step::query(
        protocol::read_block_request(profile)?,
        format!("read block of profile {}", profile + 1),
    ))
}

fn write_block_step(profile: usize, block: &Block) -> Result<Step> {
    Ok(Step::set(
        protocol::write_block_request(profile, block)?,
        delay::BLOCK,
        format!("write block of profile {}", profile + 1),
    ))
}

/// Full download of `profile` (Windows "Save", spec §2): `0x02` with the patched block, three
/// `0x05` chunks per key that needs a macro, then `0x06`. `current` is the block read with
/// `0x82` just before.
pub fn plan_full_download(
    profile_idx: usize,
    profile: &Profile,
    current: &Block,
) -> Result<Vec<Step>> {
    check_profile(profile_idx)?;
    profile.validate()?;
    let mut block = *current;
    apply_profile_to_block(profile, &mut block);
    let mut steps = vec![write_block_step(profile_idx, &block)?];
    for (k, assignment) in profile.keys.iter().enumerate() {
        steps.extend(macro_steps(profile_idx, k, assignment)?);
    }
    steps.push(commit_step());
    Ok(steps)
}

/// The three `0x05` chunks for key `k` if `assignment` needs a macro upload, else nothing.
pub fn macro_steps(profile_idx: usize, k: usize, assignment: &KeyAssignment) -> Result<Vec<Step>> {
    let Some(mblock) = macro_block(assignment).map_err(|e| e.in_key(k))? else {
        return Ok(Vec::new());
    };
    let hw = K_TO_HW[k];
    let mut steps = Vec::with_capacity(MACRO_CHUNKS);
    for (i, chunk) in macro_chunks(&mblock).iter().enumerate() {
        let n = i as u8 + 1;
        let wait = if i + 1 == MACRO_CHUNKS {
            delay::MACRO_LAST
        } else {
            delay::MACRO_CHUNK
        };
        steps.push(Step::set(
            protocol::write_macro_request(profile_idx, hw, n, chunk)?,
            wait,
            format!("macro key {} (hw {hw}) chunk {n}/3", k + 1),
        ));
    }
    Ok(steps)
}

fn commit_step() -> Step {
    Step::set(protocol::commit_request(), delay::COMMIT, "commit")
}

/// Restore of a backed-up layer (spec §9): `0x02` with the block exactly as backed up, the
/// backup's macros, then `0x06`. `profile_idx` may differ from the layer it was taken from.
pub fn plan_restore(profile_idx: usize, layer: &crate::backup::LayerBackup) -> Result<Vec<Step>> {
    check_profile(profile_idx)?;
    let mut steps = vec![write_block_step(profile_idx, &layer.block)?];
    for m in &layer.macros {
        let k =
            m.key
                .checked_sub(1)
                .filter(|&k| k < KEY_COUNT)
                .ok_or(crate::error::Error::Backup(format!(
                    "macro key {} out of range",
                    m.key
                )))?;
        steps.extend(macro_steps(profile_idx, k, &m.assignment)?);
    }
    steps.push(commit_step());
    Ok(steps)
}

/// Lighting-only update (Windows message `0x668`): one `0x02` with the key map as read back, no
/// macros, no `0x06`. On hardware this write is stored in flash anyway (it survives a power
/// cycle), and writing a profile other than the active one previews its lighting until the next
/// layer switch or replug.
pub fn plan_lighting_apply(
    profile_idx: usize,
    lighting: &Lighting,
    current: &Block,
) -> Result<Vec<Step>> {
    check_profile(profile_idx)?;
    lighting.validate()?;
    let mut block = *current;
    apply_lighting(lighting, &mut block);
    let mut step = write_block_step(profile_idx, &block)?;
    // Windows sleeps another 500 ms after the lighting-only write.
    step.delay_after += delay::LIGHTING_AFTER;
    Ok(vec![step])
}

/// Light-bar colour, for callers that index by [`LightBar`].
pub fn bar_color(l: &Lighting, bar: LightBar) -> Rgb {
    l.bar_colors[bar.slot()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes::Function;
    use crate::model::Macro;
    use crate::protocol::cmd;

    fn patterned_block() -> Block {
        std::array::from_fn(|i| (i as u8).wrapping_mul(7) ^ 0x5A)
    }

    #[test]
    fn patching_preserves_unknown_bytes() {
        let original = patterned_block();
        let mut block = original;
        let mut profile = Profile::default_for(1);
        profile.keys[0] = KeyAssignment::Key(0x04);
        apply_profile_to_block(&profile, &mut block);
        let untouched = (25..50).chain(134..200).chain([50 + 14, 75 + 14, 100 + 14]);
        for i in untouched {
            assert_eq!(block[i], original[i], "byte {i} changed");
        }
        assert_eq!(block[0], 0x04);
        // Slots 20..24 are reset to the template, whatever the device returned.
        assert_eq!(&block[20..25], &FACTORY_KEYMAP[20..]);
        assert_eq!(&block[131..134], &[1, 0, 5]);
        assert_eq!(&block[128..131], &[0, 0, 0xFF]);
    }

    #[test]
    fn keymap_bytes() {
        let mut block = [0xEEu8; 200];
        let mut keys: [KeyAssignment; KEY_COUNT] = Default::default();
        keys[1] = KeyAssignment::Disabled; // hw 5
        keys[4] = KeyAssignment::Combo(vec![0xE0, 0x06]); // hw 1
        keys[5] = KeyAssignment::Macro(Macro {
            repeat: Repeat::Times(1),
            events: vec![MacroEvent::down(4, 1)],
        }); // hw 6
        keys[6] = KeyAssignment::Function(Function::VolumeUp); // hw 11
        keys[7] = KeyAssignment::LaunchProgram("x".into()); // hw 16
        keys[8] = KeyAssignment::Function(Function::ShowDesktop); // hw 2
        keys[9] = KeyAssignment::Function(Function::ScrollLeft); // hw 7: factory
        apply_keymap(&keys, &mut block);
        let mut expected = FACTORY_KEYMAP;
        expected[5] = 0x00;
        // Combos and shortcuts are F8 macros, not Windows' E8.
        expected[1] = 0xF8;
        expected[6] = 0xF8;
        expected[11] = 0xAB;
        expected[16] = 0xF7;
        expected[2] = 0xF8;
        // Slots 20..24 come from the template, not the read-back 0xEE.
        assert_eq!(&block[..25], &expected);
        assert!(block[25..].iter().all(|&b| b == 0xEE));
        assert_eq!(decode_keymap(&block)[5], 0xF8);
    }

    #[test]
    fn lighting_layout() {
        let mut l = Lighting::default_for_profile(0);
        l.effect = Effect::Wave;
        l.brightness = 3;
        l.direction = WaveDirection::TopToBottom;
        l.color = Rgb::new(0x11, 0x22, 0x33);
        l.key_colors = [Rgb::BLACK; KEY_COUNT];
        l.key_colors[19] = Rgb::new(0xA1, 0xA2, 0xA3); // LED 23
        l.bar_colors[LightBar::RightUpper.slot()] = Rgb::new(0xB1, 0xB2, 0xB3); // LED 4
        let mut block = [0u8; 200];
        apply_lighting(&l, &mut block);
        assert_eq!(
            (block[50 + 23], block[75 + 23], block[100 + 23]),
            (0xA1, 0xA2, 0xA3)
        );
        assert_eq!(
            (block[50 + 4], block[75 + 4], block[100 + 4]),
            (0xB1, 0xB2, 0xB3)
        );
        assert_eq!(&block[125..134], &[5, 2, 2, 0x11, 0x22, 0x33, 1, 0, 5]);
        assert_eq!(decode_lighting(&block), l);
        assert_eq!(
            bar_color(&l, LightBar::RightUpper),
            Rgb::new(0xB1, 0xB2, 0xB3)
        );
    }

    #[test]
    fn win_d_macro_bytes() {
        let block = macro_block(&KeyAssignment::Function(Function::ShowDesktop))
            .unwrap()
            .unwrap();
        // Repeat 00 01, then the full chord with delay 1, like a recorded macro.
        assert_eq!(
            &block[..14],
            &[
                0x00, 0x01, 0x80, 0x01, 0xE3, 0x80, 0x01, 0x07, 0x00, 0x01, 0x07, 0x00, 0x01, 0xE3
            ]
        );
        assert!(block[14..].iter().all(|&b| b == 0));
        let zoom = macro_block(&KeyAssignment::Function(Function::ZoomOut))
            .unwrap()
            .unwrap();
        assert_eq!(&zoom[2..8], &[0x80, 0x01, 0xE0, 0x80, 0x01, 0xC1]);
    }

    #[test]
    fn recorded_macro_delay_zero_is_sent_as_one() {
        let m = KeyAssignment::Macro(Macro {
            repeat: Repeat::Times(3),
            events: vec![MacroEvent::down(0x04, 0), MacroEvent::up(0x04, 0)],
        });
        let block = macro_block(&m).unwrap().unwrap();
        assert_eq!(
            &block[..8],
            &[0x00, 0x03, 0x80, 0x01, 0x04, 0x00, 0x01, 0x04]
        );
    }

    #[test]
    fn combo_bytes() {
        // Shift+B, the chord that typed "B" on hardware as an F8 macro.
        let block = macro_block(&KeyAssignment::Combo(vec![0xE1, 0x05]))
            .unwrap()
            .unwrap();
        assert_eq!(
            &block[..14],
            &[
                0x00, 0x01, 0x80, 0x01, 0xE1, 0x80, 0x01, 0x05, 0x00, 0x01, 0x05, 0x00, 0x01, 0xE1
            ]
        );
        assert!(block[14..].iter().all(|&b| b == 0));
        // Three keys: all presses, then the releases in reverse.
        let three = macro_block(&KeyAssignment::Combo(vec![0xE0, 0xE1, 0x29]))
            .unwrap()
            .unwrap();
        assert_eq!(
            &three[..20],
            &[
                0x00, 0x01, 0x80, 0x01, 0xE0, 0x80, 0x01, 0xE1, 0x80, 0x01, 0x29, 0x00, 0x01, 0x29,
                0x00, 0x01, 0xE1, 0x00, 0x01, 0xE0
            ]
        );
        assert_eq!(macro_block(&KeyAssignment::Combo(vec![0x06])), Ok(None));
    }

    #[test]
    fn one_key_combo_and_zero_key() {
        let mut block = [0u8; 200];
        let mut keys: [KeyAssignment; KEY_COUNT] = Default::default();
        keys[0] = KeyAssignment::Combo(vec![0x2C]);
        apply_keymap(&keys, &mut block);
        assert_eq!(block[0], 0x2C);
        // Key(0) (rejected by validation) would keep the factory byte, as Windows does.
        assert_eq!(key_device_byte(0, &KeyAssignment::Key(0)), Some(0x53));
    }

    #[test]
    fn direction_clamped_as_fallback() {
        let l = Lighting {
            direction: WaveDirection::Other(9),
            ..Default::default()
        };
        let mut block = [0u8; 200];
        apply_lighting(&l, &mut block);
        assert_eq!(block[offset::DIRECTION], 3);
    }

    #[test]
    fn macro_repeat_and_limits() {
        let m = KeyAssignment::Macro(Macro {
            repeat: Repeat::WhileHeld,
            events: vec![MacroEvent::down(4, 0x0123)],
        });
        let block = macro_block(&m).unwrap().unwrap();
        assert_eq!(&block[..5], &[0xFF, 0xFF, 0x81, 0x23, 0x04]);
        let toggle = encode_macro(Repeat::Toggle, &[]).unwrap();
        assert_eq!(&toggle[..2], &[0, 0]);
        let full = encode_macro(Repeat::Times(2), &[MacroEvent::up(9, 1); 240]).unwrap();
        assert_eq!(&full[719..722], &[0x00, 0x01, 9]);
        assert_eq!(
            encode_macro(Repeat::Times(1), &[MacroEvent::up(9, 1); 241]),
            Err(ValidationError::TooManyEvents(241))
        );
        assert_eq!(macro_block(&KeyAssignment::Key(4)), Ok(None));
        assert_eq!(
            macro_block(&KeyAssignment::Function(Function::Calculator)),
            Ok(None)
        );
    }

    #[test]
    fn chunking() {
        let block: MacroBlock = std::array::from_fn(|i| (i % 251) as u8 + 1);
        let chunks = macro_chunks(&block);
        assert_eq!(&chunks[0][..], &block[..256]);
        assert_eq!(&chunks[1][..], &block[256..512]);
        assert_eq!(&chunks[2][..210], &block[512..722]);
        assert!(chunks[2][210..].iter().all(|&b| b == 0));
    }

    #[test]
    fn full_download_plan() {
        let mut p = Profile::default_for(2);
        p.keys[19] = KeyAssignment::Combo(vec![0xE0, 0x06]); // hw 19
        p.keys[0] = KeyAssignment::Function(Function::ZoomIn); // hw 0
        let steps = plan_full_download(2, &p, &patterned_block()).unwrap();
        let headers: Vec<[u8; 5]> = steps
            .iter()
            .map(|s| s.packet[..5].try_into().unwrap())
            .collect();
        assert_eq!(
            headers,
            vec![
                [7, cmd::WRITE_BLOCK, 3, 0, 0],
                [7, cmd::WRITE_MACRO, 3, 0, 1],
                [7, cmd::WRITE_MACRO, 3, 0, 2],
                [7, cmd::WRITE_MACRO, 3, 0, 3],
                [7, cmd::WRITE_MACRO, 3, 19, 1],
                [7, cmd::WRITE_MACRO, 3, 19, 2],
                [7, cmd::WRITE_MACRO, 3, 19, 3],
                [7, cmd::COMMIT, 0, 0, 0],
            ]
        );
        let delays: Vec<u64> = steps
            .iter()
            .map(|s| s.delay_after.as_millis() as u64)
            .collect();
        assert_eq!(delays, vec![200, 50, 50, 200, 50, 50, 200, 200]);
        assert!(
            steps
                .iter()
                .all(|s| s.kind == StepKind::Set && s.packet.len() == 264)
        );
        assert!(plan_full_download(5, &p, &patterned_block()).is_err());
    }

    #[test]
    fn invalid_profile_is_not_planned() {
        let mut p = Profile::default_for(0);
        p.keys[2] = KeyAssignment::Macro(Macro::default());
        let err = plan_full_download(0, &p, &[0; 200]).unwrap_err();
        assert!(err.to_string().contains("key 3"), "{err}");
    }

    #[test]
    fn lighting_plan_keeps_keymap() {
        let current = patterned_block();
        let steps = plan_lighting_apply(0, &Lighting::default(), &current).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(&steps[0].packet[..3], &[7, cmd::WRITE_BLOCK, 1]);
        assert_eq!(&steps[0].packet[8..8 + 50], &current[..50]);
        assert_eq!(steps[0].delay_after, delay::BLOCK + delay::LIGHTING_AFTER);
        assert_eq!(plan_read_block(4).unwrap().kind, StepKind::SetThenGet);
        assert_eq!(plan_read_block(4).unwrap().delay_after, delay::BLOCK);
    }
}
