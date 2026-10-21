//! In-memory keypad that understands the spec's commands, for tests, `--simulate` and dry runs
//! without hardware.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use super::{PacketDir, PacketRecord, Transport};
use crate::encode::{self, MacroBlock};
use crate::error::{Error, Result};
use crate::layout::FACTORY_KEYMAP;
use crate::model::Lighting;
use crate::protocol::{
    BLOCK_LEN, Block, FirmwareVersion, LONG_LEN, MACRO_BLOCK_LEN, MACRO_CHUNK_LEN, PAYLOAD_OFFSET,
    PROFILE_COUNT, REPORT_ID, SHORT_LEN, cmd,
};

/// A simulated MAX Falcon-20 RGB.
///
/// It holds five profile blocks (factory key map, Windows default lighting, and a recognisable
/// filler `0xC0 | (i & 0x0F)` in the unknown bytes so tests can see that they survive), answers
/// `0x80` (264 bytes: the stale buffer with byte 1 cleared, like the hardware), `0x81` (1-based)
/// and `0x82`, applies `0x01`,
/// `0x02`, `0x05`, `0x06`, `0x11`, and logs every packet with a virtual timestamp (the sum of
/// requested delays; it never sleeps). It does not emit the `04 1F` command echoes.
#[derive(Debug, Clone)]
pub struct SimulatedDevice {
    blocks: [Block; PROFILE_COUNT],
    active: usize,
    firmware: FirmwareVersion,
    macros: BTreeMap<(usize, u8), MacroBlock>,
    staging: BTreeMap<(usize, u8), [u8; MACRO_CHUNK_LEN * 3]>,
    commits: usize,
    polling_rate: Option<u8>,
    key_mode: Option<u8>,
    pending: Option<Vec<u8>>,
    /// The last GetFeature reply: the firmware's feature buffer.
    last_reply: Vec<u8>,
    events: VecDeque<Vec<u8>>,
    log: Vec<PacketRecord>,
    clock: Duration,
}

impl Default for SimulatedDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatedDevice {
    pub fn new() -> Self {
        Self {
            blocks: std::array::from_fn(Self::initial_block),
            active: 0,
            firmware: FirmwareVersion { major: 1, minor: 0 },
            macros: BTreeMap::new(),
            staging: BTreeMap::new(),
            commits: 0,
            polling_rate: None,
            key_mode: None,
            pending: None,
            last_reply: Vec::new(),
            events: VecDeque::new(),
            log: Vec::new(),
            clock: Duration::ZERO,
        }
    }

    /// The block a fresh simulator holds for `profile`.
    pub fn initial_block(profile: usize) -> Block {
        let mut block: Block = std::array::from_fn(|i| 0xC0 | (i as u8 & 0x0F));
        block[..FACTORY_KEYMAP.len()].copy_from_slice(&FACTORY_KEYMAP);
        encode::apply_lighting(&Lighting::default_for_profile(profile), &mut block);
        block
    }

    /// Profile block `profile` (0-based). Panics if out of range (test helper).
    pub fn block(&self, profile: usize) -> &Block {
        &self.blocks[profile]
    }

    /// Active profile, 0-based.
    pub fn active_profile(&self) -> usize {
        self.active
    }

    /// Fully received macro of (`profile`, `hw`).
    pub fn macro_block(&self, profile: usize, hw: u8) -> Option<&MacroBlock> {
        self.macros.get(&(profile, hw))
    }

    /// Number of `0x06` commits received.
    pub fn commit_count(&self) -> usize {
        self.commits
    }

    pub fn polling_rate(&self) -> Option<u8> {
        self.polling_rate
    }

    pub fn key_mode(&self) -> Option<u8> {
        self.key_mode
    }

    pub fn set_firmware(&mut self, v: FirmwareVersion) {
        self.firmware = v;
    }

    /// Every packet seen so far.
    pub fn log(&self) -> &[PacketRecord] {
        &self.log
    }

    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    /// Queues an input report for [`Transport::read_event`].
    pub fn push_event(&mut self, report: impl Into<Vec<u8>>) {
        self.events.push_back(report.into());
    }

    /// Queues one example of every event of spec §7, a command echo, a mouse report and an
    /// unknown report.
    pub fn push_demo_events(&mut self) {
        for report in [
            &[0x04, 0x01, 0x01, 0x05][..],
            &[0x04, 0x02, 0x0C, 0x01],
            &[0x04, 0x01, 0x07, 0x02],
            &[0x04, 0x01, 0x00, 0x02, 0x00, 0x00],
            &[0x05, 0x02, 0x19, 0x5D, 0x60, 0x08, 0x06],
            &[0x04, 0x1F, 0x81, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00],
            &[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            &[0x04, 0x09, 0x09, 0x09],
        ] {
            self.push_event(report);
        }
    }

    fn record(&mut self, dir: PacketDir, data: &[u8]) {
        self.log.push(PacketRecord {
            at: self.clock,
            dir,
            data: data.to_vec(),
            sent: true,
        });
    }

    fn profile_arg(a0: u8) -> Result<usize> {
        match a0 {
            1..=5 => Ok(a0 as usize - 1),
            _ => Err(Error::Sim(format!("profile argument {a0} not in 1..=5"))),
        }
    }

    fn reply(data: &[u8], body: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; data.len()];
        r[..2].copy_from_slice(&data[..2]);
        let n = body.len().min(r.len().saturating_sub(2));
        r[2..2 + n].copy_from_slice(&body[..n]);
        r
    }

    fn handle(&mut self, data: &[u8]) -> Result<()> {
        if data.first() != Some(&REPORT_ID) {
            return Err(Error::Sim(format!(
                "report ID {:#04x}, expected 0x07",
                data.first().copied().unwrap_or(0)
            )));
        }
        if data.len() != LONG_LEN && data.len() != SHORT_LEN {
            return Err(Error::Sim(format!(
                "length {} is neither 264 nor 8",
                data.len()
            )));
        }
        let (command, a0, a1, a2) = (data[1], data[2], data[3], data[4]);
        let long = data.len() == LONG_LEN;
        match command {
            // The real keypad does not implement the 264-byte form: the Get returns the previous
            // reply with byte 1 cleared. Only 8 bytes carry a version.
            cmd::VERSION if long => {
                let mut stale = self.last_reply.clone();
                stale.resize(LONG_LEN, 0);
                stale[1] = 0;
                self.pending = Some(stale);
            }
            cmd::VERSION => {
                self.pending = Some(Self::reply(
                    data,
                    &[self.firmware.major, self.firmware.minor],
                ));
            }
            // 1-based, as on hardware.
            cmd::CURRENT_PROFILE => {
                self.pending = Some(Self::reply(data, &[self.active as u8 + 1]));
            }
            cmd::SELECT_PROFILE if long => self.active = Self::profile_arg(a0)?,
            cmd::POLLING_RATE if !long => self.polling_rate = Some(a0),
            cmd::KEY_MODE if !long => self.key_mode = Some(a0),
            cmd::READ_BLOCK if long => {
                let p = Self::profile_arg(a0)?;
                let mut r = Self::reply(data, &[a0]);
                r[PAYLOAD_OFFSET..PAYLOAD_OFFSET + BLOCK_LEN].copy_from_slice(&self.blocks[p]);
                self.pending = Some(r);
            }
            cmd::WRITE_BLOCK if long => {
                let p = Self::profile_arg(a0)?;
                self.blocks[p].copy_from_slice(&data[PAYLOAD_OFFSET..PAYLOAD_OFFSET + BLOCK_LEN]);
            }
            cmd::WRITE_MACRO if long => {
                let p = Self::profile_arg(a0)?;
                if a1 >= 20 || !(1..=3).contains(&a2) {
                    return Err(Error::Sim(format!("bad macro address hw {a1} chunk {a2}")));
                }
                let stage = self
                    .staging
                    .entry((p, a1))
                    .or_insert([0; MACRO_CHUNK_LEN * 3]);
                let start = (a2 as usize - 1) * MACRO_CHUNK_LEN;
                stage[start..start + MACRO_CHUNK_LEN].copy_from_slice(&data[PAYLOAD_OFFSET..]);
                if a2 == 3 {
                    let mut block = [0u8; MACRO_BLOCK_LEN];
                    block.copy_from_slice(&stage[..MACRO_BLOCK_LEN]);
                    self.macros.insert((p, a1), block);
                    self.staging.remove(&(p, a1));
                }
            }
            cmd::COMMIT if long => self.commits += 1,
            other => {
                return Err(Error::Sim(format!(
                    "command {other:#04x} with length {} is not simulated",
                    data.len()
                )));
            }
        }
        Ok(())
    }
}

impl Transport for SimulatedDevice {
    fn set_feature(&mut self, data: &[u8]) -> Result<()> {
        self.record(PacketDir::SetFeature, data);
        self.handle(data)
    }

    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        let mut reply = self.pending.take().unwrap_or_default();
        reply.resize(len, 0);
        if let Some(first) = reply.first_mut() {
            *first = report_id;
        }
        self.record(PacketDir::GetFeature, &reply);
        self.last_reply.clone_from(&reply);
        Ok(reply)
    }

    fn read_event(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        match self.events.pop_front() {
            Some(ev) => {
                self.record(PacketDir::Input, &ev);
                Ok(Some(ev))
            }
            None => {
                self.clock += timeout;
                Ok(None)
            }
        }
    }

    fn delay(&mut self, d: Duration) {
        self.clock += d;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol;

    #[test]
    fn initial_block_is_plausible() {
        let b = SimulatedDevice::initial_block(1);
        assert_eq!(&b[..25], &FACTORY_KEYMAP);
        assert_eq!(&b[125..134], &[0, 4, 1, 0, 0, 0xFF, 1, 0, 5]);
        assert_eq!(b[25], 0xC0 | 9);
    }

    #[test]
    fn rejects_bad_packets() {
        let mut d = SimulatedDevice::new();
        assert!(d.set_feature(&[0x06, 0x80, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(d.set_feature(&[0x07, 0x80, 0]).is_err());
        assert!(
            d.set_feature(&protocol::select_profile_request(0).unwrap()[..8])
                .is_ok()
        );
        assert_eq!(d.polling_rate(), Some(1), "8-byte 0x01 is the polling rate");
        let mut bad = protocol::read_block_request(0).unwrap();
        bad[2] = 9;
        assert!(d.set_feature(&bad).is_err());
    }

    #[test]
    fn get_without_request_is_zeroed() {
        let mut d = SimulatedDevice::new();
        let r = d.get_feature(7, 8).unwrap();
        assert_eq!(r, vec![7, 0, 0, 0, 0, 0, 0, 0]);
    }
}
