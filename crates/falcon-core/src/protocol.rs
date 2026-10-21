//! Wire protocol: constants, request builders, reply parsers and input-report parsing.
//!
//! Every packet is a HID **feature report** with report ID 7 (spec §2):
//!
//! ```text
//! byte:  0     1     2    3    4    5..7   8 .............. 263
//!       [07] [cmd] [a0] [a1] [a2] [00..]  payload, zero-filled
//! ```
//!
//! No I/O happens here. All values that still need confirmation on hardware are marked
//! **[HW-VERIFY]** and collected in [`HW_VERIFY`], which the CLI's `doctor` prints together with
//! the items the first hardware session (2026-10-20, spec §11) confirmed.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// USB vendor ID (iONE / Itron). Spec §1.
pub const VID: u16 = 0x195D;
/// USB product ID. Spec §1.
pub const PID: u16 = 0x6008;
/// Usage page of the configuration collection. Spec §1: select by usage page/usage, never by
/// interface number.
pub const CONFIG_USAGE_PAGE: u16 = 0xFF01;
/// Usage of the configuration collection. Spec §1.
pub const CONFIG_USAGE: u16 = 0x0001;
/// Lowest vendor-defined usage page. Device events arrive on the vendor collections (confirmed
/// on hardware), so every collection at or above this page is read.
pub const VENDOR_USAGE_PAGE_MIN: u16 = 0xFF00;

/// Report ID of every command and reply. Spec §2.
pub const REPORT_ID: u8 = 0x07;
/// Feature report length, report ID included (spec §2, confirmed on hardware).
pub const LONG_LEN: usize = 264;
/// Short length Windows passes for version, polling rate and key mode (spec §2). Never send it
/// on macOS: see [`SHORT_REPORTS_BLOCKED`].
pub const SHORT_LEN: usize = 8;
/// On macOS `IOHIDDeviceSetReport` rejects an 8-byte feature report (`0xE0005000`) and the keypad
/// then times out on every request until it is replugged (hardware session 2026-10-20). The hid
/// transport refuses any length other than [`LONG_LEN`] where this is true.
pub const SHORT_REPORTS_BLOCKED: bool = cfg!(target_os = "macos");
/// Payload offset inside a long packet.
pub const PAYLOAD_OFFSET: usize = 8;

/// Number of on-board profiles (layers).
pub const PROFILE_COUNT: usize = 5;
/// Bytes of the profile block carried by `0x82`/`0x02` (spec §3).
pub const BLOCK_LEN: usize = 200;
/// One key's macro block (spec §5).
pub const MACRO_BLOCK_LEN: usize = 722;
/// Payload bytes per `0x05` chunk.
pub const MACRO_CHUNK_LEN: usize = 256;
/// Chunks per macro block (`a2` = 1..=3).
pub const MACRO_CHUNKS: usize = 3;
/// Maximum events in a macro (spec §5).
pub const MAX_MACRO_EVENTS: usize = 240;
/// Largest macro delay in 10 ms units (15 bits).
pub const MAX_DELAY_10MS: u16 = 0x7FFF;

/// Command bytes (`buf[1]`). Spec §2.
pub mod cmd {
    /// Firmware version: Set + Get, `reply[2]`, `reply[3]`. The firmware does not implement the
    /// 264-byte form (the Get returns the stale feature buffer with byte 1 cleared); the 8-byte
    /// form Windows uses cannot be sent on macOS.
    pub const VERSION: u8 = 0x80;
    /// Current profile: Set + Get, `reply[2]` = profile 1..=5 (1-based, confirmed on hardware).
    pub const CURRENT_PROFILE: u8 = 0x81;
    /// Select profile (264 bytes, `a0 = profile + 1`).
    pub const SELECT_PROFILE: u8 = 0x01;
    /// Polling rate (8 bytes only, `a0 = 1 << n`). Same byte as [`SELECT_PROFILE`]: the 264-byte
    /// form would be a profile select (rate 3 = "profile 8"), so it is never built. Experimental
    /// **[HW-VERIFY]**.
    pub const POLLING_RATE: u8 = 0x01;
    /// Key mode, probably 6KRO/NKRO (8 bytes only, `a0 = 0/1`). Experimental **[HW-VERIFY]**.
    pub const KEY_MODE: u8 = 0x11;
    /// Read a profile block: Set + Get, data at `reply[8..208]`.
    pub const READ_BLOCK: u8 = 0x82;
    /// Write a profile block, data at `buf[8..208]`.
    pub const WRITE_BLOCK: u8 = 0x02;
    /// Write one 256-byte chunk of a key's macro.
    pub const WRITE_MACRO: u8 = 0x05;
    /// Sent last in a full download, as Windows does. Block writes persist across a power
    /// cycle without it (hardware session), so what it does is open **[HW-VERIFY commit]**.
    pub const COMMIT: u8 = 0x06;
}

/// Delays copied from the Windows software (spec §2). Kept in one place so they are easy to tune.
pub mod delay {
    use std::time::Duration;

    /// After ordinary commands, and between Set and Get of short queries.
    pub const DEFAULT: Duration = Duration::from_millis(20);
    /// Around `0x82` (between Set and Get, and after the Get) and after `0x02`.
    pub const BLOCK: Duration = Duration::from_millis(200);
    /// Between macro chunks 1→2 and 2→3.
    pub const MACRO_CHUNK: Duration = Duration::from_millis(50);
    /// After the last macro chunk.
    pub const MACRO_LAST: Duration = Duration::from_millis(200);
    /// After `0x06`.
    pub const COMMIT: Duration = Duration::from_millis(200);
    /// Windows sleeps this long before every `0x80`, i.e. after connecting. Applied
    /// by [`crate::Falcon::settle`] (CLI `doctor`/`version`, GUI connect); virtual in the
    /// simulator.
    pub const CONNECT_SETTLE: Duration = Duration::from_secs(3);
    /// Before the `0x82` of a full download.
    pub const PRE_DOWNLOAD: Duration = Duration::from_millis(100);
    /// Lighting-only update: between the `0x82` and the `0x02`.
    pub const LIGHTING_GAP: Duration = Duration::from_millis(100);
    /// Lighting-only update: extra wait after the `0x02`.
    pub const LIGHTING_AFTER: Duration = Duration::from_millis(500);
}

/// The 200-byte profile block of `0x82`/`0x02` (spec §3).
pub type Block = [u8; BLOCK_LEN];
/// A long (264-byte) packet.
pub type LongPacket = [u8; LONG_LEN];
/// A short (8-byte) packet.
pub type ShortPacket = [u8; SHORT_LEN];

/// Report length for the commands Windows sends as 8 bytes (spec §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportLen {
    /// 264 bytes, report ID included.
    Long,
    /// 8 bytes, as Windows sends `0x80`, polling rate and key mode. Refused on macOS
    /// ([`SHORT_REPORTS_BLOCKED`]).
    Short,
}

impl ReportLen {
    pub const fn byte_len(self) -> usize {
        match self {
            Self::Long => LONG_LEN,
            Self::Short => SHORT_LEN,
        }
    }
}

/// Checks a 0-based profile index.
pub fn check_profile(profile: usize) -> Result<()> {
    if profile < PROFILE_COUNT {
        Ok(())
    } else {
        Err(Error::InvalidProfile(profile))
    }
}

/// The 1-based profile number the device expects in `a0`.
fn profile_arg(profile: usize) -> Result<u8> {
    check_profile(profile)?;
    // In range 1..=5 after the check.
    Ok(profile as u8 + 1)
}

fn packet(len: ReportLen, cmd: u8, args: [u8; 3]) -> Vec<u8> {
    let mut buf = vec![0u8; len.byte_len()];
    buf[0] = REPORT_ID;
    buf[1] = cmd;
    buf[2..5].copy_from_slice(&args);
    buf
}

fn long(cmd: u8, args: [u8; 3]) -> LongPacket {
    let mut buf = [0u8; LONG_LEN];
    buf[0] = REPORT_ID;
    buf[1] = cmd;
    buf[2..5].copy_from_slice(&args);
    buf
}

/// `0x80`: firmware version request.
pub fn version_request(len: ReportLen) -> Vec<u8> {
    packet(len, cmd::VERSION, [0; 3])
}

/// `0x81`: current-profile request.
pub fn current_profile_request() -> LongPacket {
    long(cmd::CURRENT_PROFILE, [0; 3])
}

/// `0x01` (264 bytes): make `profile` (0-based) the active layer.
pub fn select_profile_request(profile: usize) -> Result<LongPacket> {
    Ok(long(cmd::SELECT_PROFILE, [profile_arg(profile)?, 0, 0]))
}

fn short(cmd: u8, a0: u8) -> ShortPacket {
    let mut buf = [0u8; SHORT_LEN];
    buf[0] = REPORT_ID;
    buf[1] = cmd;
    buf[2] = a0;
    buf
}

/// `0x01`, 8 bytes only: polling rate, `a0 = 1 << rate` for `rate` 0..=3. The Hz
/// mapping is unknown **[HW-VERIFY]**. Only the short form exists: 264 bytes would be a profile
/// select with an invalid profile for rate 3.
pub fn polling_rate_request(rate: u8) -> Result<ShortPacket> {
    if rate > 3 {
        return Err(Error::UnexpectedValue {
            what: "polling rate (0..=3)",
            value: rate,
        });
    }
    Ok(short(cmd::POLLING_RATE, 1 << rate))
}

/// `0x11`, 8 bytes only: key mode 0/1 **[HW-VERIFY]**.
pub fn key_mode_request(mode: u8) -> Result<ShortPacket> {
    if mode > 1 {
        return Err(Error::UnexpectedValue {
            what: "key mode (0/1)",
            value: mode,
        });
    }
    Ok(short(cmd::KEY_MODE, mode))
}

/// `0x82`: read the block of `profile` (0-based).
pub fn read_block_request(profile: usize) -> Result<LongPacket> {
    Ok(long(cmd::READ_BLOCK, [profile_arg(profile)?, 0, 0]))
}

/// `0x02`: write `block` to `profile` (0-based).
pub fn write_block_request(profile: usize, block: &Block) -> Result<LongPacket> {
    let mut buf = long(cmd::WRITE_BLOCK, [profile_arg(profile)?, 0, 0]);
    buf[PAYLOAD_OFFSET..PAYLOAD_OFFSET + BLOCK_LEN].copy_from_slice(block);
    Ok(buf)
}

/// `0x05`: chunk `chunk` (1..=3) of the macro for hardware key `hw` in `profile` (0-based).
pub fn write_macro_request(
    profile: usize,
    hw: u8,
    chunk: u8,
    data: &[u8; MACRO_CHUNK_LEN],
) -> Result<LongPacket> {
    if !(1..=MACRO_CHUNKS as u8).contains(&chunk) {
        return Err(Error::UnexpectedValue {
            what: "macro chunk (1..=3)",
            value: chunk,
        });
    }
    let mut buf = long(cmd::WRITE_MACRO, [profile_arg(profile)?, hw, chunk]);
    buf[PAYLOAD_OFFSET..].copy_from_slice(data);
    Ok(buf)
}

/// `0x06`: end of a full download (commit) **[HW-VERIFY commit]**.
pub fn commit_request() -> LongPacket {
    long(cmd::COMMIT, [0; 3])
}

fn need(reply: &[u8], cmd: u8, len: usize) -> Result<()> {
    if reply.len() < len {
        Err(Error::ShortReply {
            cmd,
            expected: len,
            got: reply.len(),
        })
    } else {
        Ok(())
    }
}

/// Firmware version from `0x80`. Windows shows the two bytes as digits (1, 5 → "15").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FirmwareVersion {
    pub major: u8,
    pub minor: u8,
}

impl fmt::Display for FirmwareVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Parses the `0x80` reply (`reply[2]`, `reply[3]`; the report ID is `reply[0]`). `None` means
/// "not available":
///
/// - a 264-byte reply whose `reply[1]` is not `0x80`. The firmware does not implement the long
///   form: the Get returns whatever the feature buffer held, with byte 1 cleared (`07 00 00 …`
///   right after connect, `07 00 01 …` after a `0x81` that answered 1), so those bytes are no
///   version;
/// - both version bytes 0.
pub fn parse_version(reply: &[u8]) -> Result<Option<FirmwareVersion>> {
    need(reply, cmd::VERSION, 4)?;
    if reply.len() >= LONG_LEN && reply[1] != cmd::VERSION {
        return Ok(None);
    }
    Ok((reply[2..4] != [0, 0]).then_some(FirmwareVersion {
        major: reply[2],
        minor: reply[3],
    }))
}

/// Parses the `0x81` reply into a 0-based profile index. `reply[2]` is 1-based on hardware
/// (layer 1 → 1, layer 2 → 2); anything outside 1..=5 is an error rather than a guess.
/// (Windows read it as 0-based with 6 → 0, which was wrong.)
pub fn parse_current_profile(reply: &[u8]) -> Result<usize> {
    need(reply, cmd::CURRENT_PROFILE, 3)?;
    match reply[2] {
        p @ 1..=5 => Ok(usize::from(p) - 1),
        value => Err(Error::UnexpectedValue {
            what: "current profile",
            value,
        }),
    }
}

/// Extracts the profile block from a `0x82` reply (`reply[8..208]`).
///
/// Every read-modify-write starts here, so the reply must look aligned: at least 208 bytes and
/// the report ID at `[0]`. A reply with the ID stripped would shift every preserved byte by one
/// on write-back (hardware replies are `07 82 0N 00 …`, as expected). `reply[1] == 0x82` is not
/// required (Windows never checks it); `doctor` reports it.
pub fn parse_block(reply: &[u8]) -> Result<Block> {
    need(reply, cmd::READ_BLOCK, PAYLOAD_OFFSET + BLOCK_LEN)?;
    if reply[0] != REPORT_ID {
        return Err(Error::MalformedReply {
            cmd: cmd::READ_BLOCK,
            reason: format!(
                "reply[0] is {:#04x}, expected the report ID 0x07; refusing to write it back",
                reply[0]
            ),
        });
    }
    let mut block = [0u8; BLOCK_LEN];
    block.copy_from_slice(&reply[PAYLOAD_OFFSET..PAYLOAD_OFFSET + BLOCK_LEN]);
    Ok(block)
}

/// Something the keypad reported (spec §7), filtered like the Windows software, plus
/// the command echoes and mouse reports seen on hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceEvent {
    /// `04 01 01 m`: lighting effect changed (raw effect byte, ≤ 9) **[HW-VERIFY event-filters]**.
    EffectChanged { effect: u8 },
    /// `04 02 0C d`: wave direction changed (raw byte, 0/1 only) **[HW-VERIFY event-filters]**.
    DirectionChanged { direction: u8 },
    /// `04 01 07 s`: brightness changed, `brightness = s + 1` (1..=5)
    /// **[HW-VERIFY event-filters]**.
    BrightnessChanged { brightness: u8 },
    /// `04 01 00 p 00 00`: profile switched; 0-based. The raw byte is 1-based on hardware
    /// (`04 01 00 01` after Layer 1, `04 01 00 02` after Layer 2). Windows computes `p − 1`, then
    /// maps 6 → 0 and drops anything above 4, so raw 1..=5 → 0..=4, raw 7 → 0, raw 6 → ignored.
    ProfileChanged { profile: usize },
    /// `04 1F c 00 v`: the keypad echoes every feature command `c` with a value `v`, e.g.
    /// `04 1F 81 00 02` after `0x81` on layer 2, `04 1F 01 00 00` after `0x01`. Informational.
    CommandAck { cmd: u8, value: u8 },
    /// Report ID 1 from the mouse collection, which shares the event path on macOS (a key mapped
    /// to `E8` sends `01 00 00 00 00 00 00 00`). Ignored.
    MouseReport,
    /// `xx 02 19 5D 60 08 kk` with `kk` a valid hw index (0..=19): a key whose device byte reports
    /// to the host was pressed (launch program). See [`crate::layout::hw_to_k`].
    HostKey { hw: u8 },
    /// `xx 02 19 5D 60 08 kk` with `kk` outside 0..=19. Windows looks such codes up in the device
    /// column of the function table; for profile functions it re-reads `0x81`. Consumers should
    /// re-read the active profile.
    HostCode { code: u8 },
}

/// Windows always parses `InputReportByteLength` zero-padded bytes; hidapi returns the real
/// length. Short reports are zero-padded to this length before parsing.
pub const INPUT_PAD_LEN: usize = 64;

/// Parses an input report (`buf[0]` = report ID) with the Windows software's pattern order
/// and value filters, after zero-padding to [`INPUT_PAD_LEN`]. Returns `None`
/// for anything Windows ignores; never panics.
pub fn parse_input_report(buf: &[u8]) -> Option<DeviceEvent> {
    let mut padded = vec![0u8; buf.len().max(INPUT_PAD_LEN)];
    padded[..buf.len()].copy_from_slice(buf);
    let b = &padded;
    if b[1..6] == [0x02, 0x19, 0x5D, 0x60, 0x08] {
        let kk = b[6];
        return Some(if usize::from(kk) < crate::layout::KEY_COUNT {
            DeviceEvent::HostKey { hw: kk }
        } else {
            DeviceEvent::HostCode { code: kk }
        });
    }
    if b[0] == 0x01 {
        return Some(DeviceEvent::MouseReport);
    }
    if b[0] != 0x04 {
        return None;
    }
    let v = b[3];
    match (b[1], b[2]) {
        (0x1F, command) => Some(DeviceEvent::CommandAck {
            cmd: command,
            value: b[4],
        }),
        // Mode ≤ 9.
        (0x01, 0x01) => (v <= 9).then_some(DeviceEvent::EffectChanged { effect: v }),
        // Direction ≤ 1.
        (0x02, 0x0C) => (v <= 1).then_some(DeviceEvent::DirectionChanged { direction: v }),
        // Index ≤ 4, stored + 1.
        (0x01, 0x07) => (v <= 4).then_some(DeviceEvent::BrightnessChanged { brightness: v + 1 }),
        // b[3] − 1, then 6 → 0; anything above 4 is dropped.
        (0x01, 0x00) if b[5] == 0 => {
            let p = match v.wrapping_sub(1) {
                6 => 0,
                p => p,
            };
            (usize::from(p) < PROFILE_COUNT).then_some(DeviceEvent::ProfileChanged {
                profile: usize::from(p),
            })
        }
        _ => None,
    }
}

/// One checklist item of the hardware sessions (spec §10, §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HwVerify {
    /// Short stable identifier, used by `doctor` to attach probe results.
    pub id: &'static str,
    /// What to check, or the confirmed fact.
    pub question: &'static str,
    /// `Some(finding)` once confirmed on hardware; `None` while open.
    pub confirmed: Option<&'static str>,
}

const fn open_item(id: &'static str, question: &'static str) -> HwVerify {
    HwVerify {
        id,
        question,
        confirmed: None,
    }
}

const fn confirmed_item(
    id: &'static str,
    question: &'static str,
    finding: &'static str,
) -> HwVerify {
    HwVerify {
        id,
        question,
        confirmed: Some(finding),
    }
}

/// Every **[HW-VERIFY]** item of the spec: open ones first, then those confirmed on 2026-10-20.
pub const HW_VERIFY: &[HwVerify] = &[
    open_item("effects-8-10", "Effects 8..10 exist in firmware (§6)"),
    open_item(
        "polling-rate",
        "8-byte 0x01 sets the polling rate without switching profiles (§2; not on macOS)",
    ),
    open_item(
        "key-mode",
        "0x11 key mode meaning, 6KRO/NKRO? (§2; not on macOS)",
    ),
    open_item(
        "macro-mouse",
        "Macro mouse-move magnitude encoding for codes BC..BF (§5)",
    ),
    open_item(
        "event-filters",
        "Effect/brightness/direction events 04 01 01 / 04 01 07 / 04 02 0C (no on-keypad keys found yet) (§7)",
    ),
    open_item(
        "commit",
        "What 0x06 does: block writes persist without it (§2, §11)",
    ),
    confirmed_item(
        "config-collection",
        "Config traffic goes to usage page 0xFF01 / usage 1 (§1)",
        "yes",
    ),
    confirmed_item(
        "feature-len",
        "Feature report 7 is 264 bytes including the report ID (§2)",
        "yes; 8-byte reports break the keypad on macOS",
    ),
    confirmed_item(
        "reply-layout",
        "Get replies start with 07 <cmd> like the request (§2)",
        "0x82 replies are 07 82 0N 00 …",
    ),
    confirmed_item(
        "version-len",
        "0x80 version form (§2)",
        "264 bytes is not implemented (stale buffer, byte 1 cleared); 8 bytes cannot be sent on macOS",
    ),
    confirmed_item(
        "profile-base",
        "0x81 reply[2] numbering (§2)",
        "1-based: layer 1 → 1, layer 2 → 2",
    ),
    confirmed_item(
        "block-layout",
        "0x82 block: key map at 0..24, constant 01 00 05 at 131..133 (§3)",
        "factory map at 0..24 and again at 25..49; 152..199 hold firmware RAM",
    ),
    confirmed_item(
        "brightness",
        "Block byte 126 = brightness − 1 (§3)",
        "0 = LEDs off, 1..4 = dim to full",
    ),
    confirmed_item(
        "direction",
        "Block byte 127 = wave direction (§3)",
        "0 right→left, 1 left→right, 2 top→bottom, 3 bottom→top",
    ),
    confirmed_item(
        "led-map",
        "LED index map row*5+col and light bars at LEDs 9/4/24/19 (§4)",
        "yes; colour planes R/G/B at 50/75/100",
    ),
    confirmed_item(
        "event-collection",
        "Report 4 events arrive on a vendor collection (§1, §7)",
        "yes, together with 04 1F command echoes and report-1 mouse reports",
    ),
    confirmed_item(
        "combo-encoding",
        "Combos and shortcuts play on the keypad (§5)",
        "only as F8 macros with the full chord; E8 types nothing",
    ),
];

/// Whether `p` is byte-identical to a read-only query the builders produce: `0x80` (8 or 264
/// bytes), `0x81`, or `0x82` for profiles 1..=5. A dry run lets only these reach the device.
pub fn is_standard_query(p: &[u8]) -> bool {
    p == version_request(ReportLen::Short).as_slice()
        || p == version_request(ReportLen::Long).as_slice()
        || p == current_profile_request().as_slice()
        || (0..PROFILE_COUNT).any(|i| read_block_request(i).is_ok_and(|r| p == r.as_slice()))
}

/// One-line description of a request packet, for logs and dry runs.
pub fn describe_request(p: &[u8]) -> String {
    if p.len() < 5 || p[0] != REPORT_ID {
        return format!("non-standard packet ({} bytes)", p.len());
    }
    let (a0, a1, a2) = (p[2], p[3], p[4]);
    let key = |hw: u8| {
        crate::layout::hw_to_k(hw).map_or_else(|| format!("hw {hw}"), |k| format!("key {}", k + 1))
    };
    match p[1] {
        cmd::VERSION => format!("0x80 firmware version ({} bytes)", p.len()),
        cmd::CURRENT_PROFILE => "0x81 current profile".into(),
        cmd::SELECT_PROFILE if p.len() == LONG_LEN => format!("0x01 select profile {a0}"),
        cmd::POLLING_RATE => format!("0x01 polling rate {a0:#04x} (experimental)"),
        cmd::KEY_MODE => format!("0x11 key mode {a0} (experimental)"),
        cmd::READ_BLOCK => format!("0x82 read block of profile {a0}"),
        cmd::WRITE_BLOCK => format!("0x02 write block of profile {a0}"),
        cmd::WRITE_MACRO => format!(
            "0x05 macro for {} (hw {a1}) of profile {a0}, chunk {a2}/3",
            key(a1)
        ),
        cmd::COMMIT => "0x06 commit".into(),
        other => format!("{other:#04x} unknown command"),
    }
}

/// Time the Windows software waits between Set and Get for a query command.
pub fn query_delay(command: u8) -> Duration {
    if command == cmd::READ_BLOCK {
        delay::BLOCK
    } else {
        delay::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_profile_index_2() {
        let p = select_profile_request(2).unwrap();
        assert_eq!(&p[..4], &[0x07, 0x01, 0x03, 0x00]);
        assert!(p[4..].iter().all(|&b| b == 0));
        assert_eq!(p.len(), 264);
    }

    #[test]
    fn profile_out_of_range_is_rejected() {
        assert!(matches!(
            select_profile_request(5),
            Err(Error::InvalidProfile(5))
        ));
        assert!(read_block_request(7).is_err());
    }

    #[test]
    fn version_request_lengths() {
        let short = version_request(ReportLen::Short);
        assert_eq!(short, vec![0x07, 0x80, 0, 0, 0, 0, 0, 0]);
        let long = version_request(ReportLen::Long);
        assert_eq!(long.len(), 264);
        assert_eq!(&long[..2], &[0x07, 0x80]);
    }

    #[test]
    fn simple_requests() {
        assert_eq!(&current_profile_request()[..3], &[0x07, 0x81, 0x00]);
        assert_eq!(&read_block_request(0).unwrap()[..3], &[0x07, 0x82, 0x01]);
        assert_eq!(&read_block_request(4).unwrap()[..3], &[0x07, 0x82, 0x05]);
        assert_eq!(&commit_request()[..3], &[0x07, 0x06, 0x00]);
        assert_eq!(
            key_mode_request(1).unwrap(),
            [0x07, 0x11, 0x01, 0, 0, 0, 0, 0]
        );
        assert!(key_mode_request(2).is_err());
    }

    #[test]
    fn polling_rate_is_a_bit_mask() {
        let rates: Vec<u8> = (0..4)
            .map(|n| polling_rate_request(n).unwrap()[2])
            .collect();
        assert_eq!(rates, vec![1, 2, 4, 8]);
        assert_eq!(polling_rate_request(3).unwrap(), [7, 1, 8, 0, 0, 0, 0, 0]);
        assert!(polling_rate_request(4).is_err());
    }

    #[test]
    fn write_block_places_payload_at_8() {
        let mut block = [0u8; BLOCK_LEN];
        block[0] = 0xAA;
        block[199] = 0xBB;
        let p = write_block_request(1, &block).unwrap();
        assert_eq!(&p[..3], &[0x07, 0x02, 0x02]);
        assert_eq!(p[8], 0xAA);
        assert_eq!(p[207], 0xBB);
        assert!(p[208..].iter().all(|&b| b == 0));
    }

    #[test]
    fn write_macro_header() {
        let data = [0x5Au8; MACRO_CHUNK_LEN];
        let p = write_macro_request(4, 19, 3, &data).unwrap();
        assert_eq!(&p[..8], &[0x07, 0x05, 0x05, 19, 3, 0, 0, 0]);
        assert!(p[8..].iter().all(|&b| b == 0x5A));
        assert!(write_macro_request(0, 0, 0, &data).is_err());
        assert!(write_macro_request(0, 0, 4, &data).is_err());
    }

    #[test]
    fn parse_replies() {
        assert_eq!(
            parse_version(&[7, 0x80, 1, 5, 0, 0, 0, 0]).unwrap(),
            Some(FirmwareVersion { major: 1, minor: 5 })
        );
        // What the 264-byte request answers on hardware: the stale feature buffer, byte 1
        // cleared. Right after connect …
        let mut stale = [0u8; LONG_LEN];
        stale[0] = REPORT_ID;
        assert_eq!(parse_version(&stale).unwrap(), None);
        // … and after a 0x81 that answered `07 81 01`: not firmware 1.0.
        stale[2] = 1;
        assert_eq!(parse_version(&stale).unwrap(), None);
        // A long reply that does echo 0x80 would be a version.
        stale[1] = cmd::VERSION;
        assert_eq!(
            parse_version(&stale).unwrap(),
            Some(FirmwareVersion { major: 1, minor: 0 })
        );
        assert!(matches!(
            parse_version(&[7, 0x80, 1]),
            Err(Error::ShortReply { .. })
        ));
        // 0x81 is 1-based on hardware.
        assert_eq!(parse_current_profile(&[7, 0x81, 1]).unwrap(), 0);
        assert_eq!(parse_current_profile(&[7, 0x81, 2]).unwrap(), 1);
        assert_eq!(parse_current_profile(&[7, 0x81, 5]).unwrap(), 4);
        for bad in [0, 6, 7, 0xFF] {
            assert!(matches!(
                parse_current_profile(&[7, 0x81, bad]),
                Err(Error::UnexpectedValue { value, .. }) if value == bad
            ));
        }
        assert!(parse_block(&[0u8; 207]).is_err());
        let mut reply = [0u8; LONG_LEN];
        reply[0] = REPORT_ID;
        reply[8] = 0x53;
        reply[207] = 0x99;
        let block = parse_block(&reply).unwrap();
        assert_eq!((block[0], block[199]), (0x53, 0x99));
    }

    #[test]
    fn parse_every_event_row() {
        use DeviceEvent::*;
        let cases: &[(&[u8], Option<DeviceEvent>)] = &[
            (&[4, 1, 1, 5], Some(EffectChanged { effect: 5 })),
            (&[4, 1, 1, 9], Some(EffectChanged { effect: 9 })),
            (&[4, 1, 1, 10], None),
            (&[4, 2, 0x0C, 1], Some(DirectionChanged { direction: 1 })),
            (&[4, 2, 0x0C, 2], None),
            (&[4, 1, 7, 0], Some(BrightnessChanged { brightness: 1 })),
            (&[4, 1, 7, 4], Some(BrightnessChanged { brightness: 5 })),
            (&[4, 1, 7, 5], None),
            // Profile: raw − 1, then 6 → 0, then > 4 dropped.
            (&[4, 1, 0, 0, 0, 0], None),
            (&[4, 1, 0, 1, 0, 0], Some(ProfileChanged { profile: 0 })),
            (&[4, 1, 0, 3, 0, 0], Some(ProfileChanged { profile: 2 })),
            (&[4, 1, 0, 5, 0, 0], Some(ProfileChanged { profile: 4 })),
            (&[4, 1, 0, 6, 0, 0], None),
            (&[4, 1, 0, 7, 0, 0], Some(ProfileChanged { profile: 0 })),
            (&[4, 1, 0, 8, 0, 0], None),
            (&[4, 1, 0, 3, 0, 1], None),
            // Short reports are zero-padded like the Windows buffer.
            (&[4, 1, 0, 3], Some(ProfileChanged { profile: 2 })),
            (&[4, 1, 0, 3, 0], Some(ProfileChanged { profile: 2 })),
            (&[4, 1, 1], Some(EffectChanged { effect: 0 })),
            // Host key: report ID not checked; kk must be a hw index.
            (
                &[0x09, 2, 0x19, 0x5D, 0x60, 0x08, 17],
                Some(HostKey { hw: 17 }),
            ),
            (&[0x04, 2, 0x19, 0x5D, 0x60, 0x08], Some(HostKey { hw: 0 })),
            (
                &[0x05, 2, 0x19, 0x5D, 0x60, 0x08, 20],
                Some(HostCode { code: 20 }),
            ),
            (
                &[0x05, 2, 0x19, 0x5D, 0x60, 0x08, 0xF9],
                Some(HostCode { code: 0xF9 }),
            ),
            // Command echoes, as seen on hardware.
            (
                &[4, 0x1F, 0x81, 0, 2, 0, 0, 0, 0],
                Some(CommandAck {
                    cmd: 0x81,
                    value: 2,
                }),
            ),
            (
                &[4, 0x1F, 0x01, 0, 0, 0, 0, 0, 0],
                Some(CommandAck {
                    cmd: 0x01,
                    value: 0,
                }),
            ),
            // Mouse collection report (key mapped to E8).
            (&[1, 0, 0, 0, 0, 0, 0, 0], Some(MouseReport)),
            (&[], None),
            (&[4], None),
            (&[2, 1, 1, 1], None),
        ];
        for (raw, expected) in cases {
            assert_eq!(parse_input_report(raw), *expected, "{raw:02X?}");
        }
        let long = [4u8, 1, 0, 2, 0, 0, 0xAA].repeat(20);
        assert_eq!(
            parse_input_report(&long),
            Some(ProfileChanged { profile: 1 })
        );
    }

    #[test]
    fn profile_events_from_hardware() {
        // Captured with `falcon listen` after pressing Layer 1 and Layer 2.
        let layer1 = [0x04, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
        let layer2 = [0x04, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(
            parse_input_report(&layer1),
            Some(DeviceEvent::ProfileChanged { profile: 0 })
        );
        assert_eq!(
            parse_input_report(&layer2),
            Some(DeviceEvent::ProfileChanged { profile: 1 })
        );
    }

    #[test]
    fn parse_block_rejects_misaligned_replies() {
        let mut reply = [0u8; LONG_LEN];
        reply[0] = REPORT_ID;
        reply[1] = 0x99; // not required to echo 0x82
        assert!(parse_block(&reply).is_ok());
        // Report ID stripped: everything would shift by one.
        assert!(matches!(
            parse_block(&reply[1..]),
            Err(Error::MalformedReply { .. })
        ));
        reply[0] = 0x82;
        assert!(matches!(
            parse_block(&reply),
            Err(Error::MalformedReply { .. })
        ));
        reply[0] = REPORT_ID;
        assert!(matches!(
            parse_block(&reply[..207]),
            Err(Error::ShortReply { .. })
        ));
    }

    #[test]
    fn standard_queries() {
        assert!(is_standard_query(&version_request(ReportLen::Short)));
        assert!(is_standard_query(&version_request(ReportLen::Long)));
        assert!(is_standard_query(&current_profile_request()));
        for p in 0..5 {
            assert!(is_standard_query(&read_block_request(p).unwrap()));
        }
        let mut bad = read_block_request(0).unwrap();
        bad[2] = 0xFF;
        assert!(!is_standard_query(&bad));
        let mut extra = current_profile_request();
        extra[200] = 1;
        assert!(!is_standard_query(&extra));
        assert!(!is_standard_query(&[0x05, 0x80]));
        assert!(!is_standard_query(&[0x07, 0x80, 0, 0]));
        assert!(!is_standard_query(&commit_request()));
        assert!(!is_standard_query(&select_profile_request(0).unwrap()));
    }

    #[test]
    fn descriptions() {
        assert_eq!(
            describe_request(&select_profile_request(2).unwrap()),
            "0x01 select profile 3"
        );
        assert_eq!(
            describe_request(&polling_rate_request(0).unwrap()),
            "0x01 polling rate 0x01 (experimental)"
        );
        let data = [0u8; MACRO_CHUNK_LEN];
        assert_eq!(
            describe_request(&write_macro_request(0, 5, 2, &data).unwrap()),
            "0x05 macro for key 2 (hw 5) of profile 1, chunk 2/3"
        );
        assert_eq!(describe_request(&[1, 2]), "non-standard packet (2 bytes)");
    }

    #[test]
    fn checklist_ids_are_unique() {
        let mut ids: Vec<_> = HW_VERIFY.iter().map(|h| h.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), HW_VERIFY.len());
        let open: Vec<_> = HW_VERIFY
            .iter()
            .filter(|h| h.confirmed.is_none())
            .map(|h| h.id)
            .collect();
        assert_eq!(
            open,
            [
                "effects-8-10",
                "polling-rate",
                "key-mode",
                "macro-mouse",
                "event-filters",
                "commit"
            ]
        );
    }
}
