//! Text syntax for key assignments, macro events and colour targets.

use anyhow::{Context, Result, anyhow, bail};
use falcon_core::codes::{parse_key_name, parse_macro_code};
use falcon_core::layout::{KEY_COUNT, LightBar};
use falcon_core::protocol::MAX_DELAY_10MS;
use falcon_core::{Function, KeyAssignment, MacroEvent};

/// `remap` target:
///
/// - `default`, `disable`
/// - a key name or `0xNN` (`kp_7`, `a`, `layer2`, `media_play`, `0x2c`)
/// - a function (`volume-up`, `show-desktop`, also as `fn:volume-up`)
/// - a combo joined with `+` (`ctrl+c`, `lctrl+lshift+esc`)
/// - `launch:/Applications/Calculator.app`
pub fn assignment(s: &str) -> Result<KeyAssignment> {
    let t = s.trim();
    match t.to_ascii_lowercase().as_str() {
        "default" => return Ok(KeyAssignment::Default),
        "disable" | "disabled" | "none" => return Ok(KeyAssignment::Disabled),
        _ => {}
    }
    if let Some(path) = t.strip_prefix("launch:") {
        return Ok(KeyAssignment::LaunchProgram(path.trim().to_owned()));
    }
    if let Some(name) = t.strip_prefix("fn:") {
        return name
            .parse::<Function>()
            .map(KeyAssignment::Function)
            .map_err(|e| anyhow!(e));
    }
    if let Some(code) = parse_key_name(t) {
        return Ok(KeyAssignment::Key(code));
    }
    if let Ok(f) = t.parse::<Function>() {
        return Ok(KeyAssignment::Function(f));
    }
    if t.contains('+') {
        let codes = t
            .split('+')
            .map(|part| {
                parse_key_name(part).ok_or_else(|| anyhow!("unknown key {part:?} in {t:?}"))
            })
            .collect::<Result<Vec<u8>>>()?;
        return Ok(KeyAssignment::Combo(codes));
    }
    bail!("unknown key, function or combo {t:?} (try `falcon keys`)")
}

/// A delay like `100ms`, `1.5s` or `20` (milliseconds), in 10 ms units (at least 1).
pub fn delay_10ms(s: &str) -> Result<u16> {
    let t = s.trim().to_ascii_lowercase();
    let ms: f64 = if let Some(v) = t.strip_suffix("ms") {
        v.parse()?
    } else if let Some(v) = t.strip_suffix('s') {
        v.parse::<f64>()? * 1000.0
    } else {
        t.parse()?
    };
    if !ms.is_finite() || ms < 0.0 {
        bail!("invalid delay {s:?}");
    }
    let units = (ms / 10.0).round().max(1.0);
    if units > f64::from(MAX_DELAY_10MS) {
        bail!("delay {s:?} exceeds 327.67 s");
    }
    Ok(units as u16)
}

fn is_delay(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_digit()) && delay_10ms(token).is_ok()
}

/// Macro events, whitespace- or `;`-separated:
///
/// - `down:KEY[,DELAY]`, `up:KEY[,DELAY]`, `tap:KEY[,DELAY]` (down, then up 10 ms later)
/// - a bare `DELAY` (`100ms`, `1s`) adds a pause before the next event
///
/// Each event waits its delay *after the previous event*; the default is 10 ms. Keys use the
/// key names, plus `btn_left`/`btn_right`/`btn_middle` for mouse buttons inside macros.
pub fn events(s: &str) -> Result<Vec<MacroEvent>> {
    let mut out = Vec::new();
    let mut pending: u32 = 0;
    for token in s
        .split(|c: char| c.is_whitespace() || c == ';')
        .filter(|t| !t.is_empty())
    {
        if is_delay(token) {
            pending += u32::from(delay_10ms(token)?);
            continue;
        }
        let (action, rest) = token
            .split_once(':')
            .with_context(|| format!("expected ACTION:KEY or a delay, got {token:?}"))?;
        let (key, explicit) = match rest.split_once(',') {
            Some((k, d)) => (k, Some(delay_10ms(d)?)),
            None => (rest, None),
        };
        let code = parse_macro_code(key).with_context(|| format!("unknown key {key:?}"))?;
        let delay = match (explicit, pending) {
            (None, 0) => 1,
            (e, p) => u32::from(e.unwrap_or(0)) + p,
        };
        let delay = u16::try_from(delay)
            .ok()
            .filter(|&d| d <= MAX_DELAY_10MS)
            .with_context(|| format!("accumulated delay before {token:?} exceeds 327.67 s"))?;
        pending = 0;
        match action.to_ascii_lowercase().as_str() {
            "down" | "d" => out.push(MacroEvent::down(code, delay)),
            "up" | "u" => out.push(MacroEvent::up(code, delay)),
            "tap" | "t" => {
                out.push(MacroEvent::down(code, delay));
                out.push(MacroEvent::up(code, 1));
            }
            other => bail!("unknown action {other:?} (down, up or tap)"),
        }
    }
    if out.is_empty() {
        bail!("no events in {s:?}");
    }
    Ok(out)
}

/// A key number 1..=20 (as printed in `show-config`) → k.
pub fn key_number(n: u8) -> Result<usize> {
    let k = usize::from(n).wrapping_sub(1);
    if k < KEY_COUNT {
        Ok(k)
    } else {
        bail!("key {n} out of range 1..=20")
    }
}

/// Colour target of `key-color`: a key 1..=20 or `bar:lu|ru|ll|lr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTarget {
    Key(usize),
    Bar(LightBar),
}

pub fn color_target(s: &str) -> Result<ColorTarget> {
    let t = s.trim();
    if let Some(bar) = t.strip_prefix("bar:") {
        return bar.parse().map(ColorTarget::Bar).map_err(|e| anyhow!(e));
    }
    let n: u8 = t
        .parse()
        .with_context(|| format!("expected a key 1..=20 or bar:lu|ru|ll|lr, got {t:?}"))?;
    key_number(n).map(ColorTarget::Key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignments() {
        assert_eq!(assignment("default").unwrap(), KeyAssignment::Default);
        assert_eq!(assignment("Disable").unwrap(), KeyAssignment::Disabled);
        assert_eq!(assignment("kp_7").unwrap(), KeyAssignment::Key(0x5F));
        assert_eq!(assignment("0x2c").unwrap(), KeyAssignment::Key(0x2C));
        assert_eq!(assignment("kp+").unwrap(), KeyAssignment::Key(0x57));
        assert_eq!(
            assignment("volume-up").unwrap(),
            KeyAssignment::Function(Function::VolumeUp)
        );
        assert_eq!(
            assignment("fn:show_desktop").unwrap(),
            KeyAssignment::Function(Function::ShowDesktop)
        );
        assert_eq!(
            assignment("ctrl+shift+esc").unwrap(),
            KeyAssignment::Combo(vec![0xE0, 0xE1, 0x29])
        );
        assert_eq!(
            assignment("launch:/Applications/Safari.app").unwrap(),
            KeyAssignment::LaunchProgram("/Applications/Safari.app".into())
        );
        assert!(assignment("ctrl+nope").is_err());
        assert!(assignment("bogus").is_err());
    }

    #[test]
    fn delays() {
        assert_eq!(delay_10ms("100ms").unwrap(), 10);
        assert_eq!(delay_10ms("1.5s").unwrap(), 150);
        assert_eq!(delay_10ms("20").unwrap(), 2);
        assert_eq!(delay_10ms("0").unwrap(), 1);
        assert_eq!(delay_10ms("327670ms").unwrap(), 0x7FFF);
        assert!(delay_10ms("400s").is_err());
        assert!(delay_10ms("-5").is_err());
    }

    #[test]
    fn event_text() {
        let ev = events("down:LCTRL tap:c,50ms 1s up:lctrl").unwrap();
        assert_eq!(
            ev,
            vec![
                MacroEvent::down(0xE0, 1),
                MacroEvent::down(0x06, 5),
                MacroEvent::up(0x06, 1),
                MacroEvent::up(0xE0, 100),
            ]
        );
        assert_eq!(
            events("down:btn_left;up:btn_left").unwrap(),
            vec![MacroEvent::down(0xB7, 1), MacroEvent::up(0xB7, 1)]
        );
        // Bare `1` is a key name, not a delay, when it carries an action.
        assert_eq!(events("tap:1").unwrap()[0].code, 0x1E);
        assert!(events("").is_err());
        assert!(events("press:a").is_err());
        assert!(events("down:nokey").is_err());
        assert!(events("300s 300s down:a").is_err());
    }

    #[test]
    fn targets() {
        assert_eq!(color_target("1").unwrap(), ColorTarget::Key(0));
        assert_eq!(color_target("20").unwrap(), ColorTarget::Key(19));
        assert_eq!(
            color_target("bar:ll").unwrap(),
            ColorTarget::Bar(LightBar::LeftLower)
        );
        assert!(color_target("21").is_err());
        assert!(color_target("0").is_err());
        assert!(color_target("bar:xx").is_err());
    }
}
