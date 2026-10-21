//! Approximate on-screen preview of the lighting effects (spec §6). The firmware's real timing
//! and colours are unknown, so this only conveys the idea of each effect.

use std::f32::consts::TAU;

use falcon_core::layout::{BAR_COUNT, COLS, KEY_COUNT, LightBar, ROWS};
use falcon_core::{Effect, Lighting, Rgb, WaveDirection};

/// Colours of every LED at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub keys: [Rgb; KEY_COUNT],
    pub bars: [Rgb; BAR_COUNT],
}

const BREATH_PERIOD: f32 = 4.0;
const FADE_PERIOD: f32 = 3.0;
const FADE_RAMP: f32 = 2.0;
/// Wave: hue change per key column and per second.
const WAVE_SPATIAL: f32 = 0.2;
const WAVE_SPEED: f32 = 0.3;
const RANDOM_RATE: f32 = 1.5;

/// Whether the preview of `effect` changes over time (the view then keeps repainting).
pub fn is_animated(effect: Effect) -> bool {
    matches!(
        effect,
        Effect::Breathing | Effect::FadeIn | Effect::FadeOut | Effect::Wave | Effect::Random
    )
}

/// Brightness 1..=5 as a factor: 1 is off on the keypad (block byte 0), 2..=5 are a quarter
/// to full.
pub fn brightness_factor(brightness: u8) -> f32 {
    f32::from(brightness.clamp(1, 5) - 1) / 4.0
}

pub fn scale(c: Rgb, f: f32) -> Rgb {
    let s = |v: u8| (f32::from(v) * f.clamp(0.0, 1.0)).round() as u8;
    Rgb::new(s(c.r), s(c.g), s(c.b))
}

/// HSV (all in 0..=1) → RGB.
pub fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as u8 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let u = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb::new(u(r), u(g), u(b))
}

/// Horizontal position of a key column or light bar, in key widths (bars sit outside 0..=3).
fn key_x(k: usize) -> f32 {
    (k % COLS) as f32
}

/// Vertical position of a key row, in key heights.
fn key_y(k: usize) -> f32 {
    (k / COLS) as f32
}

fn bar_y(bar: usize) -> f32 {
    match LightBar::ALL[bar] {
        LightBar::LeftUpper | LightBar::RightUpper => 0.5,
        LightBar::LeftLower | LightBar::RightLower => ROWS as f32 - 1.5,
    }
}

fn bar_x(bar: usize) -> f32 {
    match LightBar::ALL[bar] {
        LightBar::LeftUpper | LightBar::LeftLower => -0.8,
        LightBar::RightUpper | LightBar::RightLower => COLS as f32 - 0.2,
    }
}

/// Cheap deterministic hash → 0..1, for the random effect.
fn noise(a: u32, b: u32) -> f32 {
    let mut x = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

fn random_led(index: u32, t: f32) -> Rgb {
    let phase = t * RANDOM_RATE + index as f32 * 0.37;
    let slot = phase.floor() as u32;
    let level = 1.0 - phase.fract();
    hsv(noise(index, slot), 1.0, level)
}

/// The preview at time `t` seconds. `last_key` is the key treated as "last pressed" by the
/// last-key effect (the GUI uses the selected key).
pub fn render(l: &Lighting, t: f32, last_key: Option<usize>) -> Frame {
    let uniform = |c: Rgb| Frame {
        keys: [c; KEY_COUNT],
        bars: [c; BAR_COUNT],
    };
    let frame = match l.effect {
        Effect::Static | Effect::Other(_) => uniform(l.color),
        Effect::Breathing => {
            let level = 0.5 - 0.5 * (TAU * t / BREATH_PERIOD).cos();
            uniform(scale(l.color, level))
        }
        Effect::FadeIn => {
            let level = (t.rem_euclid(FADE_PERIOD) / FADE_RAMP).min(1.0);
            uniform(scale(l.color, level))
        }
        Effect::FadeOut => {
            let level = 1.0 - (t.rem_euclid(FADE_PERIOD) / FADE_RAMP).min(1.0);
            uniform(scale(l.color, level))
        }
        Effect::LastKey => {
            let mut f = uniform(Rgb::BLACK);
            if let Some(k) = last_key.filter(|&k| k < KEY_COUNT) {
                f.keys[k] = l.color;
            }
            f
        }
        Effect::Wave => {
            // The pattern moves towards the direction's arrow: (dx, dy) per second.
            let (dx, dy) = match l.direction {
                WaveDirection::RightToLeft => (-1.0, 0.0),
                WaveDirection::TopToBottom => (0.0, 1.0),
                WaveDirection::BottomToTop => (0.0, -1.0),
                WaveDirection::LeftToRight | WaveDirection::Other(_) => (1.0, 0.0),
            };
            let hue = |x: f32, y: f32| (dx * x + dy * y) * WAVE_SPATIAL - t * WAVE_SPEED;
            Frame {
                keys: std::array::from_fn(|k| hsv(hue(key_x(k), key_y(k)), 1.0, 1.0)),
                bars: std::array::from_fn(|b| hsv(hue(bar_x(b), bar_y(b)), 1.0, 1.0)),
            }
        }
        Effect::Random => Frame {
            keys: std::array::from_fn(|k| random_led(k as u32, t)),
            bars: std::array::from_fn(|b| random_led(100 + b as u32, t)),
        },
        Effect::Custom => Frame {
            keys: l.key_colors,
            bars: l.bar_colors,
        },
    };
    let f = brightness_factor(l.brightness);
    Frame {
        keys: frame.keys.map(|c| scale(c, f)),
        bars: frame.bars.map(|c| scale(c, f)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lighting(effect: Effect) -> Lighting {
        Lighting {
            effect,
            color: Rgb::new(200, 100, 50),
            ..Lighting::default()
        }
    }

    fn close(a: Rgb, b: Rgb) -> bool {
        let d = |x: u8, y: u8| x.abs_diff(y) <= 3;
        d(a.r, b.r) && d(a.g, b.g) && d(a.b, b.b)
    }

    #[test]
    fn static_and_brightness() {
        let mut l = lighting(Effect::Static);
        let f = render(&l, 1.0, None);
        assert_eq!(f.keys, [Rgb::new(200, 100, 50); KEY_COUNT]);
        assert_eq!(f.bars[0], Rgb::new(200, 100, 50));
        l.brightness = 2;
        assert_eq!(render(&l, 0.0, None).keys[7], Rgb::new(50, 25, 13));
        // 1 is off on the keypad.
        l.brightness = 1;
        assert_eq!(render(&l, 0.0, None).keys[7], Rgb::BLACK);
        // Out-of-range brightness is clamped, never panics.
        l.brightness = 0;
        assert_eq!(render(&l, 0.0, None).keys[7], Rgb::BLACK);
    }

    #[test]
    fn breathing_goes_dark_and_full() {
        let l = lighting(Effect::Breathing);
        assert_eq!(render(&l, 0.0, None).keys[0], Rgb::BLACK);
        assert_eq!(render(&l, BREATH_PERIOD / 2.0, None).keys[0], l.color);
    }

    #[test]
    fn fades_are_opposite() {
        let fin = render(&lighting(Effect::FadeIn), 1.0, None).keys[0];
        let fout = render(&lighting(Effect::FadeOut), 1.0, None).keys[0];
        assert_eq!(fin, scale(Rgb::new(200, 100, 50), 0.5));
        assert_eq!(fout, fin);
        assert_eq!(
            render(&lighting(Effect::FadeIn), 0.0, None).keys[0],
            Rgb::BLACK
        );
        assert_eq!(
            render(&lighting(Effect::FadeOut), 0.0, None).keys[0],
            Rgb::new(200, 100, 50)
        );
    }

    #[test]
    fn last_key_lights_only_that_key() {
        let f = render(&lighting(Effect::LastKey), 0.0, Some(9));
        assert_eq!(f.keys[9], Rgb::new(200, 100, 50));
        assert_eq!(f.keys.iter().filter(|&&c| c != Rgb::BLACK).count(), 1);
        assert!(f.bars.iter().all(|&c| c == Rgb::BLACK));
        let none = render(&lighting(Effect::LastKey), 0.0, None);
        assert!(none.keys.iter().all(|&c| c == Rgb::BLACK));
    }

    #[test]
    fn wave_moves_in_its_direction() {
        let mut l = lighting(Effect::Wave);
        let dt = WAVE_SPATIAL / WAVE_SPEED; // time for the pattern to move one column
        l.direction = WaveDirection::RightToLeft;
        // Moving left: what column 1 shows now, column 0 shows one step later.
        assert!(close(
            render(&l, dt, None).keys[0],
            render(&l, 0.0, None).keys[1]
        ));
        l.direction = WaveDirection::LeftToRight;
        assert!(close(
            render(&l, dt, None).keys[1],
            render(&l, 0.0, None).keys[0]
        ));
        // Rows share the colour of their column.
        let f = render(&l, 0.3, None);
        assert_eq!(f.keys[1], f.keys[5]);
        // Down: what row 0 shows now, row 1 (k + 4) shows one step later.
        l.direction = WaveDirection::TopToBottom;
        assert!(close(
            render(&l, dt, None).keys[4],
            render(&l, 0.0, None).keys[0]
        ));
        l.direction = WaveDirection::BottomToTop;
        assert!(close(
            render(&l, dt, None).keys[0],
            render(&l, 0.0, None).keys[4]
        ));
        // Columns share the colour of their row.
        let f = render(&l, 0.3, None);
        assert_eq!(f.keys[4], f.keys[7]);
    }

    #[test]
    fn custom_uses_per_key_colours() {
        let mut l = lighting(Effect::Custom);
        l.key_colors[3] = Rgb::RED;
        l.bar_colors[2] = Rgb::GREEN;
        let f = render(&l, 5.0, None);
        assert_eq!(f.keys, l.key_colors);
        assert_eq!(f.bars[2], Rgb::GREEN);
    }

    #[test]
    fn random_is_deterministic_and_varied() {
        let l = lighting(Effect::Random);
        assert_eq!(render(&l, 1.2, None), render(&l, 1.2, None));
        let f = render(&l, 1.2, None);
        assert!(f.keys.iter().any(|&c| c != f.keys[0]));
    }

    #[test]
    fn hsv_primaries() {
        assert_eq!(hsv(0.0, 1.0, 1.0), Rgb::RED);
        assert_eq!(hsv(1.0 / 3.0, 1.0, 1.0), Rgb::GREEN);
        assert_eq!(hsv(2.0 / 3.0, 1.0, 1.0), Rgb::BLUE);
        assert_eq!(hsv(-1.0, 1.0, 1.0), Rgb::RED);
        assert_eq!(hsv(0.5, 0.0, 1.0), Rgb::WHITE);
    }

    #[test]
    fn animation_flags() {
        assert!(is_animated(Effect::Wave));
        assert!(!is_animated(Effect::Static));
        assert!(!is_animated(Effect::Custom));
    }
}
