//! The five factory profile blocks read from a real keypad (`falcon dump N --out pN.bin`,
//! hardware session 2026-10-20). Each file is the raw 264-byte `0x82` reply.

use falcon_core::encode::{self, BLOCK_CONSTANT, offset};
use falcon_core::layout::{BAR_COUNT, FACTORY_KEYMAP, SLOT_COUNT, factory_code};
use falcon_core::model::BRIGHTNESS_OFF;
use falcon_core::protocol::{self, Block, LONG_LEN, cmd};
use falcon_core::{Effect, Lighting, Profile, Rgb};

const DUMPS: [&[u8]; 5] = [
    include_bytes!("data/p1.bin"),
    include_bytes!("data/p2.bin"),
    include_bytes!("data/p3.bin"),
    include_bytes!("data/p4.bin"),
    include_bytes!("data/p5.bin"),
];

fn block(p: usize) -> Block {
    protocol::parse_block(DUMPS[p]).unwrap()
}

#[test]
fn replies_are_aligned() {
    for (p, reply) in DUMPS.iter().enumerate() {
        assert_eq!(reply.len(), LONG_LEN);
        assert_eq!(
            &reply[..5],
            &[0x07, cmd::READ_BLOCK, p as u8 + 1, 0, 0],
            "P{}",
            p + 1
        );
    }
}

#[test]
fn factory_key_map_twice() {
    for p in 0..5 {
        let b = block(p);
        assert_eq!(&b[..SLOT_COUNT], &FACTORY_KEYMAP, "P{}", p + 1);
        // A second copy of the factory map; meaning unknown.
        assert_eq!(
            &b[offset::KEYMAP_COPY..offset::KEYMAP_COPY + SLOT_COUNT],
            &FACTORY_KEYMAP
        );
        let by_k = encode::decode_keymap(&b);
        for (k, &code) in by_k.iter().enumerate() {
            assert_eq!(Some(code), factory_code(k), "P{} key {}", p + 1, k + 1);
        }
    }
}

#[test]
fn factory_lighting() {
    let colors = [Rgb::WHITE, Rgb::BLUE, Rgb::RED, Rgb::GREEN, Rgb::MAGENTA];
    for (p, &color) in colors.iter().enumerate() {
        let b = block(p);
        let l = encode::decode_lighting(&b);
        let defaults = Lighting::default_for_profile(p);
        assert_eq!(l.effect, Effect::Static, "P{}", p + 1);
        assert_eq!(l.color, color, "P{}", p + 1);
        // Profile 2 ships with its LEDs off (byte 0), the others at full brightness.
        let (byte, brightness) = if p == 1 { (0, BRIGHTNESS_OFF) } else { (4, 5) };
        assert_eq!(b[offset::BRIGHTNESS], byte, "P{}", p + 1);
        assert_eq!(l.brightness, brightness);
        // Direction 1 (left → right) except profile 3 (0, right → left), as in the Windows software.
        assert_eq!(l.direction, defaults.direction, "P{}", p + 1);
        assert_eq!(b[offset::DIRECTION], if p == 2 { 0 } else { 1 });
        // The default per-key pattern (arrow cluster) and dark light bars.
        assert_eq!(l.key_colors, defaults.key_colors, "P{}", p + 1);
        assert_eq!(l.bar_colors, [Rgb::BLACK; BAR_COUNT]);
        assert_eq!(&b[offset::CONSTANT..offset::CONSTANT + 3], &BLOCK_CONSTANT);
    }
}

#[test]
fn tail_is_zeros_then_firmware_ram() {
    for p in 0..5 {
        let b = block(p);
        assert!(
            b[offset::CONSTANT + 3..offset::RAM_GARBAGE]
                .iter()
                .all(|&x| x == 0)
        );
        // Pointers into firmware RAM, e.g. 54 09 00 10.
        assert!(
            b[offset::RAM_GARBAGE..]
                .windows(4)
                .any(|w| w == [0x54, 0x09, 0x00, 0x10]),
            "P{}",
            p + 1
        );
    }
}

#[test]
fn decoded_profile_writes_back_unchanged() {
    for p in 0..5 {
        let original = block(p);
        let profile = Profile {
            lighting: encode::decode_lighting(&original),
            ..Profile::default_for(p)
        };
        assert_eq!(profile.validate(), Ok(()));
        let mut patched = original;
        encode::apply_profile_to_block(&profile, &mut patched);
        assert_eq!(patched, original, "P{}", p + 1);
    }
}
