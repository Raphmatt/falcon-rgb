//! `falcon`: command-line tool for the MAX Falcon-20 RGB keypad.

mod backend;
mod commands;
mod parse;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use falcon_core::{Effect, Repeat, Rgb, WaveDirection, store};

/// Configure the MAX Falcon-20 RGB keypad (USB 195D:6008).
///
/// Commands that change the keypad first update the stored config (see `show-config`), then
/// send it. Profiles and keys are numbered from 1.
#[derive(Parser)]
#[command(name = "falcon", version, max_term_width = 100)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args)]
pub struct Global {
    /// Talk to an in-memory simulated keypad instead of USB.
    #[arg(long, global = true)]
    pub simulate: bool,
    /// Print the packets instead of sending them. Read-only queries (0x80/0x81/0x82) still reach
    /// the keypad; the config file is not written.
    #[arg(long, global = true)]
    pub dry_run: bool,
    /// More output: -v packet log, -vv also replies.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Config file [default: platform config dir, `config.sim.json` with --simulate].
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

impl Global {
    pub fn config_path(&self) -> Result<PathBuf> {
        if let Some(p) = &self.config {
            return Ok(p.clone());
        }
        let path = store::default_config_path().context("no config directory on this platform")?;
        Ok(if self.simulate {
            path.with_file_name("config.sim.json")
        } else {
            path
        })
    }
}

/// Profile number 1..=5.
fn profile_arg(s: &str) -> Result<usize, String> {
    match s.parse::<usize>() {
        Ok(n @ 1..=5) => Ok(n - 1),
        _ => Err("expected a profile number 1..=5".into()),
    }
}

/// Key number 1..=20.
fn key_arg(s: &str) -> Result<usize, String> {
    s.parse::<u8>()
        .map_err(|e| e.to_string())
        .and_then(|n| parse::key_number(n).map_err(|e| e.to_string()))
}

fn rgb_arg(s: &str) -> Result<Rgb, String> {
    s.parse()
}

fn effect_arg(s: &str) -> Result<Effect, String> {
    s.parse()
}

fn direction_arg(s: &str) -> Result<WaveDirection, String> {
    s.parse()
}

fn repeat_arg(s: &str) -> Result<Repeat, String> {
    s.parse()
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LenArg {
    /// 264 bytes. On macOS an 8-byte request is rejected by IOKit and left the keypad
    /// unresponsive until replugged (first hardware session), so 8 is never tried implicitly.
    Auto,
    #[value(name = "264")]
    Long,
    /// 8 bytes, as Windows sends it. Refused on macOS (the keypad hangs until replugged).
    #[value(name = "8")]
    Short,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the keypad's HID collections (does not open anything).
    List,
    /// Read-only health check for the first hardware session: collections, report descriptor,
    /// current profile, active block, version (264 bytes only), and the [HW-VERIFY] checklist.
    Doctor {
        /// Skip Windows' 3 s wait before the first query.
        #[arg(long)]
        no_settle: bool,
    },
    /// Firmware version (0x80). The firmware does not implement the 264-byte form ("not
    /// available"); the 8-byte form is refused on macOS.
    Version {
        /// Report length to use.
        #[arg(long, value_enum, default_value = "auto")]
        len: LenArg,
        /// Skip Windows' 3 s wait before the query.
        #[arg(long)]
        no_settle: bool,
    },
    /// Get or set the active profile.
    Profile {
        #[command(subcommand)]
        action: ProfileCmd,
    },
    /// Read a profile block (0x82): hex dump and decoded fields.
    Dump {
        /// Profile 1..=5.
        #[arg(value_parser = profile_arg)]
        profile: usize,
        /// Also save the raw 264-byte reply to this file (.bin).
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Set the lighting effect (lighting-only update; the keypad stores it in flash anyway).
    Effect {
        /// static, breathing, fade-in, fade-out, last-key, wave, random, custom (or 0..10).
        #[arg(value_parser = effect_arg)]
        effect: Effect,
        /// Effect colour RRGGBB.
        #[arg(long, value_parser = rgb_arg)]
        color: Option<Rgb>,
        /// 1..=5: 1 = LEDs off, 2..5 = dim to full.
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        brightness: Option<u8>,
        /// Wave movement: to-left, to-right, down, up (or the raw byte 0..3).
        #[arg(long, value_parser = direction_arg)]
        direction: Option<WaveDirection>,
        /// Profile 1..=5 [default: the keypad's active layer (0x81)].
        #[arg(long, value_parser = profile_arg)]
        profile: Option<usize>,
    },
    /// Set one key's or light bar's colour and switch to the custom effect.
    KeyColor {
        /// Key 1..=20 or bar:lu|ru|ll|lr.
        target: String,
        /// Colour RRGGBB.
        #[arg(value_parser = rgb_arg)]
        color: Rgb,
        /// Profile 1..=5 [default: the keypad's active layer (0x81)].
        #[arg(long, value_parser = profile_arg)]
        profile: Option<usize>,
    },
    /// Assign a key: a key name, 0xNN, default, disable, a function, ctrl+c, launch:PATH.
    Remap {
        /// Key 1..=20 (row-major, top-left = 1).
        #[arg(value_parser = key_arg)]
        key: usize,
        /// What the key should do (see `falcon keys`).
        target: String,
        /// Profile 1..=5 [default: the keypad's active layer (0x81)].
        #[arg(long, value_parser = profile_arg)]
        profile: Option<usize>,
    },
    /// Assign a macro to a key.
    Macro {
        /// Key 1..=20.
        #[arg(value_parser = key_arg)]
        key: usize,
        /// Events, e.g. "down:lctrl tap:c,50ms 1s up:lctrl".
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        events: Option<String>,
        /// JSON file with {"repeat": ..., "events": [...]} as in the config.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// N times (1..65534), held, or toggle [default: 1, or the file's value].
        #[arg(long, value_parser = repeat_arg)]
        repeat: Option<Repeat>,
        /// Profile 1..=5 [default: the keypad's active layer (0x81)].
        #[arg(long, value_parser = profile_arg)]
        profile: Option<usize>,
    },
    /// Full download of the stored profile(s): block, macros, commit (one commit per profile, so
    /// --all sends five; Windows only ever writes one profile per save).
    Apply {
        /// Profile 1..=5 [default: the keypad's active layer (0x81)].
        #[arg(long, value_parser = profile_arg, conflicts_with = "all")]
        profile: Option<usize>,
        /// All five profiles.
        #[arg(long)]
        all: bool,
    },
    /// Print the stored config.
    ShowConfig {
        /// Only this profile.
        #[arg(long, value_parser = profile_arg)]
        profile: Option<usize>,
        /// Raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Write the stored config (defaults if there is none) to the config file or FILE.
    SaveConfig {
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Print key names and functions usable with `remap` and `macro`.
    Keys,
    /// Print events from the keypad (on-device profile/lighting changes, host keys) until Ctrl-C.
    Listen {
        /// Stop after this many seconds.
        #[arg(long)]
        seconds: Option<u64>,
    },
    /// Expert: send raw bytes (report ID first), zero-padded to --len. With --dry-run nothing is
    /// opened or sent. On macOS only --len 264 is sent to the keypad.
    Raw {
        /// Hex bytes, e.g. "07 80".
        bytes: String,
        #[arg(long, default_value_t = 264)]
        len: usize,
        /// Read a reply (GetFeature, same length) afterwards.
        #[arg(long)]
        get: bool,
        /// Required on real hardware.
        #[arg(long)]
        yes_i_know: bool,
    },
    /// Experimental commands whose effect is unverified.
    #[command(subcommand)]
    Experimental(ExperimentalCmd),
}

#[derive(Subcommand)]
enum ProfileCmd {
    /// Print the active profile (0x81).
    Get,
    /// Activate a profile (0x01).
    Set {
        #[arg(value_parser = profile_arg)]
        profile: usize,
    },
}

#[derive(Subcommand)]
enum ExperimentalCmd {
    /// Polling rate 0..=3, sent as 1 << n with the 8-byte 0x01 (the 264-byte form would be a
    /// profile select, so it is never sent). Refused on macOS.
    PollingRate {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=3))]
        rate: u8,
        #[arg(long)]
        yes_i_know: bool,
    },
    /// Key mode 0/1 (8-byte 0x11, probably 6KRO/NKRO). Refused on macOS.
    KeyMode {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=1))]
        mode: u8,
        #[arg(long)]
        yes_i_know: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let g = &cli.global;
    match cli.cmd {
        Cmd::List => commands::list(),
        Cmd::Doctor { no_settle } => commands::doctor(g, no_settle),
        Cmd::Version { len, no_settle } => commands::version(g, len, no_settle),
        Cmd::Profile { action } => match action {
            ProfileCmd::Get => commands::profile_get(g),
            ProfileCmd::Set { profile } => commands::profile_set(g, profile),
        },
        Cmd::Dump { profile, out } => commands::dump(g, profile, out.as_deref()),
        Cmd::Effect {
            effect,
            color,
            brightness,
            direction,
            profile,
        } => commands::effect(g, effect, color, brightness, direction, profile),
        Cmd::KeyColor {
            target,
            color,
            profile,
        } => commands::key_color(g, &target, color, profile),
        Cmd::Remap {
            key,
            target,
            profile,
        } => commands::remap(g, key, &target, profile),
        Cmd::Macro {
            key,
            events,
            file,
            repeat,
            profile,
        } => commands::macro_cmd(g, key, events.as_deref(), file.as_deref(), repeat, profile),
        Cmd::Apply { profile, all } => commands::apply(g, profile, all),
        Cmd::ShowConfig { profile, json } => commands::show_config(g, profile, json),
        Cmd::SaveConfig { out } => commands::save_config(g, out.as_deref()),
        Cmd::Keys => {
            commands::keys();
            Ok(())
        }
        Cmd::Listen { seconds } => commands::listen(g, seconds),
        Cmd::Raw {
            bytes,
            len,
            get,
            yes_i_know,
        } => commands::raw(g, &bytes, len, get, yes_i_know),
        Cmd::Experimental(e) => match e {
            ExperimentalCmd::PollingRate { rate, yes_i_know } => {
                commands::polling_rate(g, rate, yes_i_know)
            }
            ExperimentalCmd::KeyMode { mode, yes_i_know } => {
                commands::key_mode(g, mode, yes_i_know)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn numbers_are_one_based() {
        assert_eq!(profile_arg("1"), Ok(0));
        assert!(profile_arg("0").is_err());
        assert!(profile_arg("6").is_err());
        assert_eq!(key_arg("20"), Ok(19));
        assert!(key_arg("21").is_err());
    }
}
