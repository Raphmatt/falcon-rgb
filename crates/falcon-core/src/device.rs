//! High-level device API: executes [`crate::encode`] plans over any [`Transport`].
//!
//! Every method blocks for the protocol delays (via [`Transport::delay`]; the simulator only
//! advances a virtual clock). A GUI should call these from a worker thread.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::encode::{self, Step, StepKind};
use crate::error::Result;
use crate::model::{Lighting, Profile};
use crate::protocol::{
    self, Block, DeviceEvent, FirmwareVersion, REPORT_ID, ReportLen, delay, parse_input_report,
};
use crate::transport::Transport;

/// A raw input report and what it means, if recognised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputReport {
    pub raw: Vec<u8>,
    pub event: Option<DeviceEvent>,
}

/// The keypad behind some transport.
#[derive(Debug)]
pub struct Falcon<T: Transport> {
    transport: T,
}

impl<T: Transport> Falcon<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_inner(self) -> T {
        self.transport
    }

    /// SetFeature, wait, GetFeature of the same length, wait. Returns the raw reply.
    pub fn query(&mut self, packet: &[u8], wait: Duration) -> Result<Vec<u8>> {
        self.transport.set_feature(packet)?;
        self.transport.delay(wait);
        let reply = self.transport.get_feature(REPORT_ID, packet.len())?;
        self.transport.delay(wait);
        Ok(reply)
    }

    /// Sends one packet and waits.
    pub fn send(&mut self, packet: &[u8], wait: Duration) -> Result<()> {
        self.transport.set_feature(packet)?;
        self.transport.delay(wait);
        Ok(())
    }

    /// Runs a plan. Returns the replies of the [`StepKind::SetThenGet`] steps, in order.
    pub fn execute(&mut self, steps: &[Step]) -> Result<Vec<Vec<u8>>> {
        let mut replies = Vec::new();
        for step in steps {
            match step.kind {
                StepKind::Set => self.send(&step.packet, step.delay_after)?,
                StepKind::SetThenGet => replies.push(self.query(&step.packet, step.delay_after)?),
            }
        }
        Ok(replies)
    }

    /// Waits [`delay::CONNECT_SETTLE`] (3 s), as Windows does before `0x80` right after the device
    /// appears. Call once after opening real hardware; the simulator only advances
    /// its virtual clock.
    pub fn settle(&mut self) {
        self.transport.delay(delay::CONNECT_SETTLE);
    }

    /// Raw `0x80` reply in the given length. [`ReportLen::Short`] fails on macOS
    /// ([`protocol::SHORT_REPORTS_BLOCKED`]).
    pub fn version_reply(&mut self, len: ReportLen) -> Result<Vec<u8>> {
        self.query(&protocol::version_request(len), delay::DEFAULT)
    }

    /// Firmware version (`0x80`); `None` when not available, which is always the case for the
    /// 264-byte request ([`protocol::parse_version`]).
    pub fn firmware_version(&mut self, len: ReportLen) -> Result<Option<FirmwareVersion>> {
        protocol::parse_version(&self.version_reply(len)?)
    }

    /// Raw `0x81` reply.
    pub fn current_profile_reply(&mut self) -> Result<Vec<u8>> {
        self.query(&protocol::current_profile_request(), delay::DEFAULT)
    }

    /// Active profile, 0-based (`0x81`, whose reply is 1-based).
    pub fn current_profile(&mut self) -> Result<usize> {
        protocol::parse_current_profile(&self.current_profile_reply()?)
    }

    /// Makes `profile` (0-based) the active layer (`0x01`).
    pub fn select_profile(&mut self, profile: usize) -> Result<()> {
        self.send(&protocol::select_profile_request(profile)?, delay::DEFAULT)
    }

    /// Raw `0x82` reply for `profile` (0-based), 264 bytes.
    pub fn read_block_reply(&mut self, profile: usize) -> Result<Vec<u8>> {
        let step = encode::plan_read_block(profile)?;
        self.query(&step.packet, step.delay_after)
    }

    /// The 200-byte block of `profile` (0-based).
    pub fn read_block(&mut self, profile: usize) -> Result<Block> {
        protocol::parse_block(&self.read_block_reply(profile)?)
    }

    /// Full download (Windows "Save"): `0x82`, patched `0x02`, macros, `0x06`. Validates the
    /// profile before sending anything.
    pub fn write_profile(&mut self, profile_idx: usize, profile: &Profile) -> Result<()> {
        protocol::check_profile(profile_idx)?;
        profile.validate()?;
        // Windows sleeps 100 ms before the read of a full download.
        self.transport.delay(delay::PRE_DOWNLOAD);
        // read_block refuses a misaligned or short reply, so nothing shifted is written back.
        let current = self.read_block(profile_idx)?;
        let steps = encode::plan_full_download(profile_idx, profile, &current)?;
        self.execute(&steps).map(drop)
    }

    /// Lighting-only update: `0x82`, then `0x02` with the key map unchanged. No `0x06`, as in
    /// Windows; the keypad stores the block in flash anyway (it survives a power cycle), so
    /// every call costs a flash write. Writing a non-active profile previews its lighting until
    /// the next layer switch or replug.
    pub fn apply_lighting(&mut self, profile_idx: usize, lighting: &Lighting) -> Result<()> {
        protocol::check_profile(profile_idx)?;
        lighting.validate()?;
        let current = self.read_block(profile_idx)?;
        // 100 ms between the read and the write.
        self.transport.delay(delay::LIGHTING_GAP);
        let steps = encode::plan_lighting_apply(profile_idx, lighting, &current)?;
        self.execute(&steps).map(drop)
    }

    /// Experimental: polling rate `0..=3` (`1 << rate`, 8-byte `0x01` only) **[HW-VERIFY]**. The
    /// 264-byte form of the same command byte selects profiles, so it is never sent; the 8-byte
    /// form fails on macOS.
    pub fn set_polling_rate(&mut self, rate: u8) -> Result<()> {
        self.send(&protocol::polling_rate_request(rate)?, delay::DEFAULT)
    }

    /// Experimental: key mode 0/1 (8-byte `0x11` only, fails on macOS) **[HW-VERIFY]**.
    pub fn set_key_mode(&mut self, mode: u8) -> Result<()> {
        self.send(&protocol::key_mode_request(mode)?, delay::DEFAULT)
    }

    /// Next input report, waiting at most `timeout`.
    pub fn poll_event(&mut self, timeout: Duration) -> Result<Option<InputReport>> {
        Ok(self.transport.read_event(timeout)?.map(|raw| InputReport {
            event: parse_input_report(&raw),
            raw,
        }))
    }
}

#[cfg(feature = "hid")]
impl Falcon<crate::transport::hid::HidTransport> {
    /// Opens the first keypad found via hidapi.
    pub fn open_hid() -> Result<Self> {
        Ok(Self::new(crate::transport::hid::HidTransport::open()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes::Function;
    use crate::error::Error;
    use crate::layout::{FACTORY_KEYMAP, K_TO_HW};
    use crate::model::{Effect, KeyAssignment, Macro, MacroEvent, Repeat, Rgb};
    use crate::protocol::cmd;
    use crate::transport::{PacketDir, SimulatedDevice};

    fn sim() -> Falcon<SimulatedDevice> {
        Falcon::new(SimulatedDevice::new())
    }

    #[test]
    fn queries() {
        let mut f = sim();
        assert_eq!(
            f.firmware_version(ReportLen::Short).unwrap(),
            Some(FirmwareVersion { major: 1, minor: 0 })
        );
        // Like the hardware, the simulator answers the 264-byte form with its stale buffer.
        assert_eq!(f.firmware_version(ReportLen::Long).unwrap(), None);
        f.current_profile().unwrap();
        let stale = f.version_reply(ReportLen::Long).unwrap();
        assert_eq!(&stale[..3], &[0x07, 0x00, 0x01]);
        assert_eq!(protocol::parse_version(&stale).unwrap(), None);
        f.select_profile(3).unwrap();
        assert_eq!(f.current_profile().unwrap(), 3);
        // 0x81 is 1-based on the wire.
        assert_eq!(f.current_profile_reply().unwrap()[2], 4);
        assert_eq!(f.read_block(3).unwrap(), SimulatedDevice::initial_block(3));
    }

    #[test]
    fn full_download_end_state() {
        let mut f = sim();
        let before = SimulatedDevice::initial_block(1);
        let mut p = Profile::default_for(1);
        p.keys[0] = KeyAssignment::Key(0x04);
        p.keys[1] = KeyAssignment::Macro(Macro {
            repeat: Repeat::Times(2),
            events: vec![MacroEvent::down(0x05, 0), MacroEvent::up(0x05, 3)],
        });
        p.keys[19] = KeyAssignment::Function(Function::ShowDesktop);
        p.lighting.effect = Effect::Custom;
        p.lighting.key_colors[0] = Rgb::new(1, 2, 3);
        f.write_profile(1, &p).unwrap();

        let dev = f.transport();
        let block = dev.block(1);
        let mut keymap = FACTORY_KEYMAP;
        keymap[0] = 0x04;
        keymap[5] = 0xF8;
        keymap[19] = 0xF8;
        assert_eq!(&block[..25], &keymap);
        assert_eq!(block[125], 7);
        assert_eq!((block[50], block[75], block[100]), (1, 2, 3));
        // Unknown bytes survived the round trip.
        assert_eq!(&block[25..50], &before[25..50]);
        assert_eq!(&block[134..], &before[134..]);
        // Other profiles untouched.
        assert_eq!(dev.block(0), &SimulatedDevice::initial_block(0));

        let m = dev.macro_block(1, K_TO_HW[1]).unwrap();
        assert_eq!(&m[..8], &[0x00, 0x02, 0x80, 0x01, 0x05, 0x00, 0x03, 0x05]);
        assert!(dev.macro_block(1, 19).is_some());
        assert_eq!(dev.commit_count(), 1);

        // Packet sequence: 0x82 set+get, 0x02, 2 × 3 × 0x05, 0x06.
        let cmds: Vec<(PacketDir, u8)> = dev
            .log()
            .iter()
            .map(|r| (r.dir, r.data.get(1).copied().unwrap_or(0)))
            .collect();
        let mut expected = vec![
            (PacketDir::SetFeature, cmd::READ_BLOCK),
            (PacketDir::GetFeature, cmd::READ_BLOCK),
            (PacketDir::SetFeature, cmd::WRITE_BLOCK),
        ];
        expected.extend([(PacketDir::SetFeature, cmd::WRITE_MACRO); 6]);
        expected.push((PacketDir::SetFeature, cmd::COMMIT));
        assert_eq!(cmds, expected);
        // Virtual send time of the commit: 100 (pre-download) + 200 + 200 (read) + 200 (write)
        // + 2 × (50 + 50 + 200).
        assert_eq!(dev.log().last().unwrap().at, Duration::from_millis(1300));
    }

    /// Simulator whose `0x82` slots 20..24 and key 1 differ from the template.
    fn sim_with_custom_keymap() -> Falcon<SimulatedDevice> {
        let mut f = sim();
        let mut block = SimulatedDevice::initial_block(0);
        block[0] = 0x04;
        block[20..25].copy_from_slice(&[1, 2, 3, 4, 5]);
        f.transport_mut()
            .set_feature(&protocol::write_block_request(0, &block).unwrap())
            .unwrap();
        f.transport_mut().clear_log();
        f
    }

    #[test]
    fn full_download_resets_template_slots() {
        let mut f = sim_with_custom_keymap();
        f.write_profile(0, &Profile::default_for(0)).unwrap();
        assert_eq!(&f.transport().block(0)[..25], &FACTORY_KEYMAP);
    }

    #[test]
    fn lighting_apply_keeps_read_back_keymap() {
        let mut f = sim_with_custom_keymap();
        f.apply_lighting(0, &Lighting::default_for_profile(0))
            .unwrap();
        let block = f.transport().block(0);
        assert_eq!(block[0], 0x04);
        assert_eq!(&block[20..25], &[1, 2, 3, 4, 5]);
        // 0x82 at 0, Get at 200, 100 ms gap → 0x02 at 500.
        assert_eq!(f.transport().log()[2].at, Duration::from_millis(500));
    }

    /// Strips the report ID from every GetFeature reply, like a misbehaving backend would.
    struct StripId(SimulatedDevice);

    impl Transport for StripId {
        fn set_feature(&mut self, data: &[u8]) -> Result<()> {
            self.0.set_feature(data)
        }
        fn get_feature(&mut self, id: u8, len: usize) -> Result<Vec<u8>> {
            let mut r = self.0.get_feature(id, len)?;
            r.remove(0);
            Ok(r)
        }
        fn read_event(&mut self, t: Duration) -> Result<Option<Vec<u8>>> {
            self.0.read_event(t)
        }
        fn delay(&mut self, d: Duration) {
            self.0.delay(d);
        }
    }

    #[test]
    fn misaligned_reply_aborts_before_writing() {
        let mut f = Falcon::new(StripId(SimulatedDevice::new()));
        assert!(matches!(
            f.write_profile(0, &Profile::default_for(0)),
            Err(Error::MalformedReply { .. })
        ));
        assert!(matches!(
            f.apply_lighting(0, &Lighting::default()),
            Err(Error::MalformedReply { .. })
        ));
        let writes = f
            .transport()
            .0
            .log()
            .iter()
            .filter(|r| r.dir == PacketDir::SetFeature && r.data[1] != cmd::READ_BLOCK)
            .count();
        assert_eq!(writes, 0);
    }

    #[test]
    fn settle_is_virtual_in_the_simulator() {
        let mut f = sim();
        f.settle();
        f.firmware_version(ReportLen::Short).unwrap();
        assert_eq!(f.transport().log()[0].at, delay::CONNECT_SETTLE);
    }

    #[test]
    fn lighting_apply_keeps_keymap_and_skips_commit() {
        let mut f = sim();
        let mut l = Lighting::default_for_profile(0);
        l.effect = Effect::Breathing;
        l.brightness = 2;
        f.apply_lighting(0, &l).unwrap();
        let dev = f.transport();
        assert_eq!(&dev.block(0)[..25], &FACTORY_KEYMAP);
        assert_eq!(&dev.block(0)[125..127], &[1, 1]);
        assert_eq!(dev.commit_count(), 0);
    }

    #[test]
    fn invalid_input_sends_nothing() {
        let mut f = sim();
        let mut p = Profile::default_for(0);
        p.lighting.brightness = 9;
        assert!(f.write_profile(0, &p).is_err());
        assert!(f.write_profile(5, &Profile::default()).is_err());
        assert!(f.set_key_mode(2).is_err());
        assert!(f.transport().log().is_empty());
    }

    #[test]
    fn experimental_commands() {
        let mut f = sim();
        f.set_polling_rate(2).unwrap();
        f.set_key_mode(1).unwrap();
        assert!(f.transport().log().iter().all(|r| r.data.len() == 8));
        assert_eq!(f.transport().polling_rate(), Some(4));
        assert_eq!(f.transport().key_mode(), Some(1));
        assert_eq!(f.transport().active_profile(), 0);
    }

    #[test]
    fn events() {
        let mut f = sim();
        f.transport_mut().push_demo_events();
        let mut parsed = Vec::new();
        while let Some(r) = f.poll_event(Duration::from_millis(1)).unwrap() {
            parsed.push(r.event);
        }
        assert_eq!(
            parsed,
            vec![
                Some(DeviceEvent::EffectChanged { effect: 5 }),
                Some(DeviceEvent::DirectionChanged { direction: 1 }),
                Some(DeviceEvent::BrightnessChanged { brightness: 3 }),
                Some(DeviceEvent::ProfileChanged { profile: 1 }),
                Some(DeviceEvent::HostKey { hw: 6 }),
                Some(DeviceEvent::CommandAck {
                    cmd: cmd::CURRENT_PROFILE,
                    value: 2
                }),
                Some(DeviceEvent::MouseReport),
                None,
            ]
        );
    }
}
