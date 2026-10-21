//! Chooses the transport (USB, simulator, dry run) and prints packet logs.

use std::time::Duration;

use anyhow::{Context, Result};
use falcon_core::protocol::{LONG_LEN, describe_request};
use falcon_core::transport::hid::{CollectionInfo, HidTransport, INPUT_MONITORING_SETTINGS_URL};
use falcon_core::{Falcon, PacketDir, Recorder, SimulatedDevice, Transport, hex};

use crate::Global;

pub type Device = Falcon<Recorder<Box<dyn Transport>>>;

/// What [`open`] found when it opened real hardware.
pub struct HwInfo {
    pub config: CollectionInfo,
    pub event_paths: Vec<String>,
    pub descriptor: Result<Vec<u8>, String>,
}

pub struct Backend {
    pub dev: Device,
    /// `None` for the simulator.
    pub hw: Option<HwInfo>,
}

impl Backend {
    pub fn is_sim(&self) -> bool {
        self.hw.is_none()
    }

    pub fn is_dry_run(&self) -> bool {
        self.dev.transport().is_dry_run()
    }
}

fn simulator(demo_events: bool) -> Box<dyn Transport> {
    let mut sim = SimulatedDevice::new();
    if demo_events {
        sim.push_demo_events();
    }
    Box::new(sim)
}

/// Opens the keypad, the simulator (`--simulate`), or wraps either for `--dry-run`. A dry run
/// without hardware falls back to the simulator for its reads.
pub fn open(g: &Global, demo_events: bool) -> Result<Backend> {
    let (transport, hw): (Box<dyn Transport>, _) = if g.simulate {
        (simulator(demo_events), None)
    } else {
        match HidTransport::open() {
            Ok(t) => {
                for w in t.warnings() {
                    eprintln!("warning: {w}");
                }
                let hw = HwInfo {
                    config: t.config_info().clone(),
                    event_paths: t.event_paths(),
                    descriptor: t.report_descriptor().map_err(|e| e.to_string()),
                };
                (Box::new(t), Some(hw))
            }
            Err(e) if g.dry_run => {
                eprintln!("note: {e}; the dry run reads from the simulator instead");
                (simulator(demo_events), None)
            }
            Err(e @ falcon_core::Error::PermissionDenied(_)) => {
                return Err(e).context(format!(
                    "cannot open the keypad: no Input Monitoring permission for this terminal \
                     (settings: open \"{INPUT_MONITORING_SETTINGS_URL}\")"
                ));
            }
            Err(e) => {
                return Err(e).context(
                    "cannot open the keypad (try `falcon list`, or --simulate / --dry-run)",
                );
            }
        }
    };
    let recorder = if g.dry_run {
        Recorder::dry_run(transport)
    } else {
        Recorder::new(transport)
    };
    Ok(Backend {
        dev: Falcon::new(recorder),
        hw,
    })
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("    {l}\n"))
        .collect::<String>()
}

/// Prints the packet log: always for dry runs, with `-v` otherwise.
pub fn print_log(b: &Backend, verbose: u8) {
    let log = b.dev.transport().log();
    if log.is_empty() || (!b.is_dry_run() && verbose == 0) {
        return;
    }
    let sets = log
        .iter()
        .filter(|r| r.dir == PacketDir::SetFeature)
        .count();
    let suppressed = log.iter().filter(|r| !r.sent).count();
    let source = if b.is_sim() { "simulator" } else { "device" };
    println!();
    if b.is_dry_run() {
        println!(
            "--- dry run: {sets} SetFeature packet(s), {suppressed} not sent; reads answered by the {source} ---"
        );
    } else {
        println!("--- packet log ({source}) ---");
    }
    for r in log {
        let at = r.at.as_millis();
        let line = match r.dir {
            PacketDir::SetFeature => format!(
                "[{at:>5} ms] SET  {}{}",
                describe_request(&r.data),
                if r.sent { "" } else { "   (NOT SENT)" }
            ),
            PacketDir::GetFeature => format!("[{at:>5} ms] GET  reply, {} bytes", r.data.len()),
            PacketDir::Input => format!("[{at:>5} ms] IN   input report"),
        };
        println!("{line}");
        // Long replies are only interesting with -vv; dry runs always show what would be sent.
        let show = match r.dir {
            PacketDir::SetFeature => true,
            _ => verbose >= 2,
        };
        if show {
            print!("{}", indent(&hex::dump(&r.data)));
        }
    }
}

/// Human-readable size of a reply vs. the expected long length.
pub fn reply_note(reply: &[u8], expected_cmd: u8) -> String {
    let mut notes = Vec::new();
    if reply.len() != LONG_LEN && reply.len() != 8 {
        notes.push(format!("{} bytes", reply.len()));
    }
    match reply.first() {
        Some(0x07) => {}
        Some(id) => notes.push(format!("report ID {id:#04x}, expected 0x07")),
        None => notes.push("empty".into()),
    }
    if reply.get(1).is_some_and(|&c| c != expected_cmd) {
        notes.push(format!(
            "byte 1 is {:#04x}, not the command {expected_cmd:#04x}",
            reply[1]
        ));
    }
    if notes.is_empty() {
        "layout as expected".into()
    } else {
        notes.join("; ")
    }
}

/// Default timeout per `listen` poll.
pub const LISTEN_POLL: Duration = Duration::from_millis(200);
