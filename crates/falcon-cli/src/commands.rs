//! Implementation of the subcommands.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use falcon_core::codes::{self, Function, KEY_NAMES, MACRO_MOUSE_CODES};
use falcon_core::descriptor::{self, ReportKind};
use falcon_core::encode::{self, BLOCK_CONSTANT, offset};
use falcon_core::layout::{self, COLS, FACTORY_KEYMAP, LightBar};
use falcon_core::model::BRIGHTNESS_OFF;
use falcon_core::protocol::{
    self, Block, FirmwareVersion, HW_VERIFY, LONG_LEN, REPORT_ID, ReportLen, SHORT_REPORTS_BLOCKED,
    cmd,
};
use falcon_core::transport::hid::{CollectionInfo, list_collections};
use falcon_core::{
    Config, DeviceEvent, Effect, KeyAssignment, Macro, Repeat, Rgb, Transport, WaveDirection, hex,
    store,
};

use crate::backend::{self, Backend, print_log, reply_note};
use crate::{Global, LenArg, parse};

fn report_len(l: LenArg) -> ReportLen {
    match l {
        LenArg::Short => ReportLen::Short,
        LenArg::Auto | LenArg::Long => ReportLen::Long,
    }
}

/// Refuses commands that need a non-264-byte feature report on macOS, where the first one makes
/// the keypad stop answering until it is replugged. The simulator is fine.
fn refuse_short_on_macos(g: &Global, what: &str) -> Result<()> {
    if SHORT_REPORTS_BLOCKED && !g.simulate {
        bail!(
            "{what} would send a feature report that is not 264 bytes long. macOS rejects those \
             (IOKit 0xE0005000) and the keypad then stops answering until it is replugged, so \
             nothing was sent. Use --simulate to see the packets."
        );
    }
    Ok(())
}

fn version_text(v: Option<FirmwareVersion>) -> String {
    v.map_or_else(
        || "not available (the keypad does not answer 0x80 with 264 bytes)".into(),
        |v| v.to_string(),
    )
}

fn brightness_text(brightness: u8) -> String {
    if brightness == BRIGHTNESS_OFF {
        format!("{brightness} (off)")
    } else {
        brightness.to_string()
    }
}

fn require_ack(g: &Global, yes: bool, what: &str) -> Result<()> {
    if !g.simulate && !g.dry_run && !yes {
        bail!("{what} on real hardware needs --yes-i-know (or use --simulate / --dry-run)");
    }
    Ok(())
}

/// Prefix for result lines, so a dry run never reads as if the keypad changed.
fn tag(b: &Backend) -> &'static str {
    if b.is_dry_run() { "[dry run] " } else { "" }
}

fn load(g: &Global) -> Result<(PathBuf, Config)> {
    let path = g.config_path()?;
    let cfg = store::load_config(&path)
        .with_context(|| format!("cannot read config {}", path.display()))?;
    Ok((path, cfg))
}

fn save(g: &Global, path: &Path, cfg: &Config) -> Result<()> {
    if g.dry_run {
        println!("(dry run: config {} not written)", path.display());
        return Ok(());
    }
    store::save_config(path, cfg).with_context(|| format!("cannot write {}", path.display()))?;
    if g.verbose > 0 {
        println!("saved {}", path.display());
    }
    Ok(())
}

/// `--profile` if given, else the keypad's active layer (`0x81`, verified on hardware). The
/// simulator starts on layer 1 every run, so with it (or if the query fails) the stored active
/// profile (set by `profile set`) is used.
fn resolve_profile(b: &mut Backend, arg: Option<usize>, cfg: &Config) -> usize {
    if let Some(p) = arg {
        return p;
    }
    if !b.is_sim() {
        match b.dev.current_profile() {
            Ok(p) => {
                eprintln!(
                    "target: the keypad's active layer {} (use --profile N to choose another)",
                    p + 1
                );
                return p;
            }
            Err(e) => eprintln!("cannot read the active layer ({e})"),
        }
    }
    eprintln!(
        "target: stored active profile {} (use --profile N to choose another)",
        cfg.active_profile + 1
    );
    cfg.active_profile
}

/// Windows' 3 s wait before `0x80` after connecting (virtual in the simulator).
fn settle(b: &mut Backend, skip: bool) {
    if skip {
        return;
    }
    if !b.is_sim() {
        eprintln!("waiting 3 s before the first query, as Windows does (--no-settle skips)");
    }
    b.dev.settle();
}

fn collection_line(c: &CollectionInfo) -> String {
    format!(
        "{} if{:<2} usage {:04X}:{:04X} {:<17} {}",
        if c.is_config() { "*" } else { " " },
        c.interface,
        c.usage_page,
        c.usage,
        c.usage_name(),
        c.path
    )
}

pub fn list() -> Result<()> {
    let cols = list_collections()?;
    let Some(first) = cols.first() else {
        println!("No MAX Falcon-20 RGB (195D:6008) found.");
        println!("Check the USB cable. On Linux, hidraw access needs a udev rule.");
        return Ok(());
    };
    println!(
        "{} collection(s) of 195D:6008: {} / {}, release {:X}.{:02X}",
        cols.len(),
        first.manufacturer.as_deref().unwrap_or("?"),
        first.product.as_deref().unwrap_or("?"),
        first.release >> 8,
        first.release & 0xFF
    );
    for c in &cols {
        println!("  {}", collection_line(c));
    }
    if cols.iter().any(CollectionInfo::is_config) {
        println!("* = config collection (usage page FF01, usage 1): all commands go here");
    } else {
        println!("No config collection (usage page FF01 / usage 1) found: commands cannot work.");
    }
    Ok(())
}

fn print_block(block: &Block) {
    let keymap = encode::decode_keymap(block);
    println!("Key map by key (row-major; * = differs from factory):");
    for row in keymap.chunks(COLS).enumerate() {
        let (r, codes) = row;
        let cells: Vec<String> = codes
            .iter()
            .enumerate()
            .map(|(c, &code)| {
                let k = r * COLS + c;
                let mark = if layout::factory_code(k) == Some(code) {
                    ' '
                } else {
                    '*'
                };
                format!("{:>2} {:<12}{mark}", k + 1, codes::code_label(code))
            })
            .collect();
        println!("  {}", cells.join(" "));
    }
    let extra: Vec<String> = block[20..25].iter().map(|b| format!("{b:02X}")).collect();
    println!(
        "  extra slots 20..24: {} (factory {})",
        extra.join(" "),
        hex::to_hex(&FACTORY_KEYMAP[20..])
    );
    let l = encode::decode_lighting(block);
    println!(
        "Lighting: effect {} (byte {}), brightness {} (byte {}), direction {} (byte {}), colour {}",
        l.effect,
        block[offset::EFFECT],
        brightness_text(l.brightness),
        block[offset::BRIGHTNESS],
        l.direction,
        block[offset::DIRECTION],
        l.color
    );
    let constant = &block[offset::CONSTANT..offset::CONSTANT + 3];
    println!(
        "  bytes 131..133: {}{}",
        hex::to_hex(constant),
        if constant == BLOCK_CONSTANT {
            ""
        } else {
            "  (expected 01 00 05)"
        }
    );
    println!("Key colours:");
    for row in l.key_colors.chunks(COLS) {
        let cells: Vec<String> = row.iter().map(Rgb::to_string).collect();
        println!("  {}", cells.join(" "));
    }
    let bars: Vec<String> = LightBar::ALL
        .iter()
        .map(|b| format!("{b} {}", l.bar_colors[b.slot()]))
        .collect();
    println!("Light bars: {}", bars.join(", "));
}

pub fn doctor(g: &Global, no_settle: bool) -> Result<()> {
    let mut probes: BTreeMap<&str, String> = BTreeMap::new();
    if !g.simulate {
        println!("== HID collections ==");
        list()?;
        println!();
    }
    let mut b = backend::open(g, false)?;
    println!("== Transport ==");
    match &b.hw {
        Some(hw) => {
            println!("config collection: {}", collection_line(&hw.config));
            println!("event readers:     {}", hw.event_paths.join(", "));
            probes.insert(
                "config-collection",
                format!("found: if{} {}", hw.config.interface, hw.config.path),
            );
            probes.insert(
                "event-collection",
                format!(
                    "reading {}; run `falcon listen` and press Layer/LED keys",
                    hw.event_paths.join(", ")
                ),
            );
            match &hw.descriptor {
                Ok(desc) => {
                    println!("report descriptor ({} bytes):", desc.len());
                    print!("{}", hex::dump(desc));
                    let reports = descriptor::parse_reports(desc);
                    for r in &reports {
                        println!(
                            "  {:?} report {:#04x}: {} bytes incl. ID (usage page {:04X})",
                            r.kind,
                            r.report_id,
                            r.byte_len(),
                            r.usage_page
                        );
                    }
                    let feature = reports
                        .iter()
                        .find(|r| r.kind == ReportKind::Feature && r.report_id == REPORT_ID);
                    probes.insert(
                        "feature-len",
                        feature.map_or("no feature report 7 in the descriptor".into(), |r| {
                            format!("descriptor says {} bytes", r.byte_len())
                        }),
                    );
                }
                Err(e) => println!("report descriptor: unavailable ({e})"),
            }
        }
        None => {
            println!("simulated keypad");
            probes.insert("config-collection", "simulator".into());
        }
    }

    println!("\n== Probes (read-only) ==");
    settle(&mut b, no_settle);
    // Only the 264-byte form: an 8-byte request is rejected by IOKit and the keypad stopped
    // answering afterwards (first hardware session).
    let version = match b.dev.version_reply(ReportLen::Long) {
        Ok(reply) => {
            let parsed =
                protocol::parse_version(&reply).map_or_else(|e| e.to_string(), version_text);
            println!(
                "0x80 version, 264-byte request: {parsed}  [{}]  reply {}",
                reply_note(&reply, cmd::VERSION),
                hex::to_hex(&reply[..reply.len().min(8)])
            );
            parsed
        }
        Err(e) => {
            println!("0x80 version, 264-byte request: FAILED: {e}");
            "failed".into()
        }
    };
    probes.insert("version-len", format!("264: {version}"));

    let mut active = 0;
    match b.dev.current_profile_reply() {
        Ok(reply) => {
            let raw = reply
                .get(2)
                .map_or_else(|| "missing".to_owned(), u8::to_string);
            let parsed = protocol::parse_current_profile(&reply);
            if let Ok(p) = parsed {
                active = p;
            }
            let shown = parsed.map_or_else(|e| e.to_string(), |p| format!("profile {}", p + 1));
            println!(
                "0x81 current profile: reply[2] = {raw} → {shown}  [{}]",
                reply_note(&reply, cmd::CURRENT_PROFILE)
            );
            probes.insert("profile-base", format!("reply[2] = {raw} → {shown}"));
        }
        Err(e) => println!("0x81 current profile: FAILED: {e}"),
    }

    match b.dev.read_block_reply(active) {
        Ok(reply) => {
            println!(
                "\n0x82 block of profile {} ({} bytes, {}):",
                active + 1,
                reply.len(),
                reply_note(&reply, cmd::READ_BLOCK)
            );
            print!("{}", hex::dump(&reply));
            probes.insert("reply-layout", reply_note(&reply, cmd::READ_BLOCK));
            match protocol::parse_block(&reply) {
                Ok(block) => {
                    println!();
                    print_block(&block);
                    let keymap_ok = block[..20] == FACTORY_KEYMAP[..20];
                    let const_ok = block[offset::CONSTANT..offset::CONSTANT + 3] == BLOCK_CONSTANT;
                    probes.insert(
                        "block-layout",
                        format!(
                            "key map {} factory, bytes 131..133 {}",
                            if keymap_ok { "matches" } else { "differs from" },
                            hex::to_hex(&block[offset::CONSTANT..offset::CONSTANT + 3])
                        ) + if const_ok { "" } else { " (expected 01 00 05)" },
                    );
                    probes.insert(
                        "brightness",
                        format!(
                            "byte 126 = {}{}",
                            block[offset::BRIGHTNESS],
                            if block[offset::BRIGHTNESS] == 0 {
                                " (off)"
                            } else {
                                ""
                            }
                        ),
                    );
                    probes.insert(
                        "direction",
                        format!(
                            "byte 127 = {} ({})",
                            block[offset::DIRECTION],
                            encode::decode_lighting(&block).direction
                        ),
                    );
                }
                Err(e) => println!("cannot parse block: {e}"),
            }
        }
        Err(e) => println!("0x82 read block: FAILED: {e}"),
    }

    println!("\n== [HW-VERIFY] checklist (spec §10, §11) ==");
    for item in HW_VERIFY {
        let mark = if item.confirmed.is_some() { "x" } else { " " };
        println!("[{mark}] {:<18} {}", item.id, item.question);
        if let Some(finding) = item.confirmed {
            println!("    {:<18} confirmed: {finding}", "");
        }
        if let Some(p) = probes.get(item.id) {
            println!("    {:<18} probe: {p}", "");
        }
    }
    println!(
        "\nNext: `falcon dump 1..5 --out pN.bin`, `falcon effect static --color ff0000`, \
         `falcon key-color 1 ff0000`, `falcon listen`, then `falcon remap` + power cycle."
    );
    print_log(&b, g.verbose);
    Ok(())
}

pub fn version(g: &Global, len: LenArg, no_settle: bool) -> Result<()> {
    if len == LenArg::Short {
        refuse_short_on_macos(g, "`version --len 8`")?;
    }
    let mut b = backend::open(g, false)?;
    settle(&mut b, no_settle);
    let tries: &[ReportLen] = match len {
        LenArg::Auto => &[ReportLen::Long],
        other => &[report_len(other)][..],
    };
    let mut last_err = None;
    for &l in tries {
        match b.dev.firmware_version(l) {
            Ok(v) => {
                println!(
                    "firmware {} ({}-byte request)",
                    version_text(v),
                    l.byte_len()
                );
                last_err = None;
                break;
            }
            Err(e) => {
                eprintln!("{}-byte request failed: {e}", l.byte_len());
                last_err = Some(e);
            }
        }
    }
    print_log(&b, g.verbose);
    match last_err {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

pub fn profile_get(g: &Global) -> Result<()> {
    let mut b = backend::open(g, false)?;
    let reply = b.dev.current_profile_reply()?;
    let p = protocol::parse_current_profile(&reply)?;
    println!("active profile: {}", p + 1);
    if g.verbose > 0 {
        println!("raw reply[2] = {} (1-based)", reply[2]);
    }
    print_log(&b, g.verbose);
    Ok(())
}

pub fn profile_set(g: &Global, profile: usize) -> Result<()> {
    let (path, mut cfg) = load(g)?;
    let mut b = backend::open(g, false)?;
    b.dev.select_profile(profile)?;
    cfg.active_profile = profile;
    save(g, &path, &cfg)?;
    println!("{}profile {} selected", tag(&b), profile + 1);
    print_log(&b, g.verbose);
    Ok(())
}

pub fn dump(g: &Global, profile: usize, out: Option<&Path>) -> Result<()> {
    let mut b = backend::open(g, false)?;
    let reply = b.dev.read_block_reply(profile)?;
    println!(
        "0x82 reply for profile {} ({} bytes, {}):",
        profile + 1,
        reply.len(),
        reply_note(&reply, cmd::READ_BLOCK)
    );
    print!("{}", hex::dump(&reply));
    let block = protocol::parse_block(&reply)?;
    println!();
    print_block(&block);
    if let Some(path) = out {
        std::fs::write(path, &reply).with_context(|| format!("cannot write {}", path.display()))?;
        println!("\nsaved {} bytes to {}", reply.len(), path.display());
    }
    print_log(&b, g.verbose);
    Ok(())
}

pub fn effect(
    g: &Global,
    effect: Effect,
    color: Option<Rgb>,
    brightness: Option<u8>,
    direction: Option<WaveDirection>,
    profile: Option<usize>,
) -> Result<()> {
    let (path, mut cfg) = load(g)?;
    let mut b = backend::open(g, false)?;
    let p = resolve_profile(&mut b, profile, &cfg);
    let l = &mut cfg.profiles[p].lighting;
    l.effect = effect;
    if let Some(c) = color {
        l.color = c;
    }
    if let Some(v) = brightness {
        l.brightness = v;
    }
    if let Some(d) = direction {
        l.direction = d;
    }
    l.validate()?;
    let l = l.clone();
    save(g, &path, &cfg)?;
    b.dev.apply_lighting(p, &l)?;
    println!(
        "{}profile {}: effect {}, colour {}, brightness {}, direction {}",
        tag(&b),
        p + 1,
        l.effect,
        l.color,
        brightness_text(l.brightness),
        l.direction
    );
    print_log(&b, g.verbose);
    Ok(())
}

pub fn key_color(g: &Global, target: &str, color: Rgb, profile: Option<usize>) -> Result<()> {
    let target = parse::color_target(target)?;
    let (path, mut cfg) = load(g)?;
    let mut b = backend::open(g, false)?;
    let p = resolve_profile(&mut b, profile, &cfg);
    let l = &mut cfg.profiles[p].lighting;
    let what = match target {
        parse::ColorTarget::Key(k) => {
            l.key_colors[k] = color;
            format!("key {}", k + 1)
        }
        parse::ColorTarget::Bar(bar) => {
            l.bar_colors[bar.slot()] = color;
            format!("light bar {bar}")
        }
    };
    l.effect = Effect::Custom;
    let l = l.clone();
    save(g, &path, &cfg)?;
    b.dev.apply_lighting(p, &l)?;
    println!(
        "{}profile {}: {what} = {color}, effect custom",
        tag(&b),
        p + 1
    );
    print_log(&b, g.verbose);
    Ok(())
}

fn assign(g: &Global, k: usize, a: KeyAssignment, profile: Option<usize>) -> Result<()> {
    a.validate()
        .map_err(|e| anyhow::anyhow!("key {}: {e}", k + 1))?;
    let (path, mut cfg) = load(g)?;
    let mut b = backend::open(g, false)?;
    let p = resolve_profile(&mut b, profile, &cfg);
    let label = a.label();
    let host = a.is_host_action();
    cfg.profiles[p].keys[k] = a;
    save(g, &path, &cfg)?;
    b.dev.write_profile(p, &cfg.profiles[p])?;
    println!(
        "{}profile {} key {} ({}): {label}",
        tag(&b),
        p + 1,
        k + 1,
        layout::KEY_LEGENDS[k]
    );
    if host {
        println!("note: host actions only run while the Falcon RGB app is running");
    }
    print_log(&b, g.verbose);
    Ok(())
}

pub fn remap(g: &Global, k: usize, target: &str, profile: Option<usize>) -> Result<()> {
    assign(g, k, parse::assignment(target)?, profile)
}

pub fn macro_cmd(
    g: &Global,
    k: usize,
    events: Option<&str>,
    file: Option<&Path>,
    repeat: Option<Repeat>,
    profile: Option<usize>,
) -> Result<()> {
    let mut m = match (events, file) {
        (Some(text), _) => Macro {
            repeat: Repeat::Times(1),
            events: parse::events(text)?,
        },
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            serde_json::from_str(&text)
                .with_context(|| format!("invalid macro JSON in {}", path.display()))?
        }
        (None, None) => bail!("give --events or --file"),
    };
    if let Some(r) = repeat {
        m.repeat = r;
    }
    if g.verbose > 0 {
        for (i, e) in m.events.iter().enumerate() {
            println!(
                "  {:>3}: {:<4} {:<12} after {} ms",
                i + 1,
                if e.down { "down" } else { "up" },
                codes::macro_code_name(e.code),
                u32::from(e.delay_10ms) * 10
            );
        }
    }
    assign(g, k, KeyAssignment::Macro(m), profile)
}

pub fn apply(g: &Global, profile: Option<usize>, all: bool) -> Result<()> {
    let (_, cfg) = load(g)?;
    cfg.validate()?;
    let mut b = backend::open(g, false)?;
    let profiles: Vec<usize> = if all {
        (0..cfg.profiles.len()).collect()
    } else {
        vec![resolve_profile(&mut b, profile, &cfg)]
    };
    for p in profiles {
        b.dev.write_profile(p, &cfg.profiles[p])?;
        println!(
            "{}profile {} written ({})",
            tag(&b),
            p + 1,
            cfg.profiles[p].name
        );
    }
    print_log(&b, g.verbose);
    Ok(())
}

pub fn show_config(g: &Global, profile: Option<usize>, json: bool) -> Result<()> {
    let (path, cfg) = load(g)?;
    if json {
        let text = match profile {
            Some(p) => serde_json::to_string_pretty(&cfg.profiles[p])?,
            None => serde_json::to_string_pretty(&cfg)?,
        };
        println!("{text}");
        return Ok(());
    }
    println!(
        "{}{}",
        path.display(),
        if path.exists() {
            ""
        } else {
            " (not created yet; defaults)"
        }
    );
    let range: Vec<usize> = profile.map_or_else(|| (0..cfg.profiles.len()).collect(), |p| vec![p]);
    for p in range {
        let pr = &cfg.profiles[p];
        let l = &pr.lighting;
        println!(
            "\nProfile {}{}: {:?}",
            p + 1,
            if cfg.active_profile == p {
                " (active)"
            } else {
                ""
            },
            pr.name
        );
        println!(
            "  lighting: {}, colour {}, brightness {}, direction {}",
            l.effect,
            l.color,
            brightness_text(l.brightness),
            l.direction
        );
        let custom: Vec<String> = pr
            .keys
            .iter()
            .enumerate()
            .filter(|(_, a)| **a != KeyAssignment::Default)
            .map(|(k, a)| {
                format!(
                    "    key {:>2} ({:<9}): {}",
                    k + 1,
                    layout::KEY_LEGENDS[k],
                    a.label()
                )
            })
            .collect();
        if custom.is_empty() {
            println!("  keys: all default");
        } else {
            println!("  keys:");
            for line in custom {
                println!("{line}");
            }
        }
        if !pr.linked_apps.is_empty() {
            println!("  linked apps: {}", pr.linked_apps.join(", "));
        }
    }
    Ok(())
}

pub fn save_config(g: &Global, out: Option<&Path>) -> Result<()> {
    let (path, cfg) = load(g)?;
    cfg.validate()?;
    let target = out.unwrap_or(&path);
    save(g, target, &cfg)?;
    if !g.dry_run {
        println!("wrote {}", target.display());
    }
    Ok(())
}

pub fn keys() {
    println!("Keys (remap target or macro key; case-insensitive, `_` optional, or 0xNN):");
    let names: Vec<&str> = KEY_NAMES.iter().map(|k| k.name).collect();
    for row in names.chunks(8) {
        println!("  {}", row.join(" "));
    }
    println!("\nMacro-only codes: {}", {
        let v: Vec<&str> = MACRO_MOUSE_CODES.iter().map(|k| k.name).collect();
        v.join(" ")
    });
    println!("\nFunctions (remap target, optionally prefixed with fn:):");
    let fns: Vec<&str> = Function::ALL.iter().map(|f| f.name()).collect();
    for row in fns.chunks(6) {
        println!("  {}", row.join(" "));
    }
    println!("\nOther remap targets: default, disable, ctrl+c (combo), launch:/path/to/App.app");
    println!(
        "Macro events: down:KEY[,DELAY] up:KEY[,DELAY] tap:KEY[,DELAY] and bare DELAY (100ms, 1.5s)"
    );
    println!("Effects: static breathing fade-in fade-out last-key wave random custom (or 0..10)");
    println!("Wave directions: to-left to-right down up (or 0..3); brightness 1 = off .. 5 = full");
}

fn describe_event(ev: &DeviceEvent, cfg: &Config, profile: usize) -> String {
    match *ev {
        DeviceEvent::EffectChanged { effect } => {
            format!("effect changed to {}", Effect::from_byte(effect))
        }
        DeviceEvent::DirectionChanged { direction } => {
            format!("wave direction changed to {direction}")
        }
        DeviceEvent::BrightnessChanged { brightness } => {
            format!("brightness changed to {brightness}")
        }
        DeviceEvent::ProfileChanged { profile } => format!("profile switched to {}", profile + 1),
        DeviceEvent::HostKey { hw } => match layout::hw_to_k(hw) {
            Some(k) => format!(
                "host key: key {} ({}) → {}",
                k + 1,
                layout::KEY_LEGENDS[k],
                cfg.profiles[profile].keys[k].label()
            ),
            None => format!("host key with unknown hw index {hw}"),
        },
        DeviceEvent::HostCode { code } => {
            format!("host code {code:#04x} (not a key index; re-reading the active profile)")
        }
        DeviceEvent::CommandAck { cmd, value } => format!("ack {cmd:#04x} → {value}"),
        DeviceEvent::MouseReport => "mouse report (ignored)".into(),
    }
}

pub fn listen(g: &Global, seconds: Option<u64>) -> Result<()> {
    let (_, cfg) = load(g)?;
    let mut b = backend::open(g, true)?;
    let mut profile = b.dev.current_profile().unwrap_or(cfg.active_profile);
    match &b.hw {
        Some(hw) => println!(
            "listening on {} (Ctrl-C to stop)",
            hw.event_paths.join(", ")
        ),
        None => println!("listening to the simulator's demo events"),
    }
    let start = Instant::now();
    let limit = seconds.map(Duration::from_secs);
    loop {
        match b.dev.poll_event(backend::LISTEN_POLL)? {
            Some(r) => {
                let what = match &r.event {
                    Some(ev) => {
                        if let DeviceEvent::ProfileChanged { profile: p } = ev {
                            profile = *p;
                        }
                        let mut text = describe_event(ev, &cfg, profile);
                        // Windows re-reads 0x81 for profile-type host codes.
                        if matches!(ev, DeviceEvent::HostCode { .. }) {
                            match b.dev.current_profile() {
                                Ok(p) => {
                                    profile = p;
                                    text += &format!(": profile {}", p + 1);
                                }
                                Err(e) => text += &format!(": {e}"),
                            }
                        }
                        text
                    }
                    None => "unrecognised".into(),
                };
                println!(
                    "[{:>7.3} s] {:<24} {what}",
                    start.elapsed().as_secs_f64(),
                    hex::to_hex(&r.raw)
                );
            }
            None if b.is_sim() => break,
            None => {}
        }
        if limit.is_some_and(|l| start.elapsed() >= l) {
            break;
        }
    }
    print_log(&b, g.verbose);
    Ok(())
}

pub fn raw(g: &Global, bytes: &str, len: usize, get: bool, yes: bool) -> Result<()> {
    let mut data = hex::parse(bytes).map_err(anyhow::Error::msg)?;
    if data.is_empty() || data.len() > len {
        bail!("need 1..={len} bytes, got {}", data.len());
    }
    data.resize(len, 0);
    println!("SET  {}", protocol::describe_request(&data));
    print!("{}", hex::dump(&data));
    // A dry run of `raw` never opens anything: arbitrary bytes are not "read-only".
    if g.dry_run {
        println!(
            "(dry run: not sent{})",
            if get { ", no reply read" } else { "" }
        );
        return Ok(());
    }
    require_ack(g, yes, "`raw`")?;
    if len != LONG_LEN {
        refuse_short_on_macos(g, &format!("`raw --len {len}`"))?;
    }
    let mut b = backend::open(g, false)?;
    let t = b.dev.transport_mut();
    t.set_feature(&data)?;
    let wait = protocol::query_delay(data.get(1).copied().unwrap_or(0));
    t.delay(wait);
    if get {
        let reply = t.get_feature(data[0], len)?;
        t.delay(wait);
        println!("GET  {} bytes", reply.len());
        print!("{}", hex::dump(&reply));
    }
    print_log(&b, g.verbose);
    Ok(())
}

pub fn polling_rate(g: &Global, rate: u8, yes: bool) -> Result<()> {
    refuse_short_on_macos(g, "the experimental polling-rate command")?;
    require_ack(g, yes, "the experimental polling-rate command")?;
    let mut b = backend::open(g, false)?;
    b.dev.set_polling_rate(rate)?;
    println!(
        "{}polling rate {rate} (byte {:#04x}, 8-byte 0x01; experimental)",
        tag(&b),
        1u8 << rate
    );
    print_log(&b, g.verbose);
    Ok(())
}

pub fn key_mode(g: &Global, mode: u8, yes: bool) -> Result<()> {
    refuse_short_on_macos(g, "the experimental key-mode command")?;
    require_ack(g, yes, "the experimental key-mode command")?;
    let mut b = backend::open(g, false)?;
    b.dev.set_key_mode(mode)?;
    println!("{}key mode {mode} (8-byte 0x11; experimental)", tag(&b));
    print_log(&b, g.verbose);
    Ok(())
}
