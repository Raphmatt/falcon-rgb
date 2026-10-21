//! How bytes reach the keypad: the [`Transport`] trait and its implementations.

#[cfg(feature = "hid")]
pub mod hid;
mod sim;

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::protocol::{self, cmd};

pub use sim::SimulatedDevice;

/// Feature-report I/O with the configuration collection, plus input-report reads.
///
/// Buffers always include the report ID at `[0]` (hidapi convention). `Send`, so a GUI can move
/// a [`crate::Falcon`] into a worker thread.
pub trait Transport: Send {
    /// SetFeature with `data[0]` = report ID.
    fn set_feature(&mut self, data: &[u8]) -> Result<()>;

    /// GetFeature of `report_id` with a buffer of `len` bytes. Returns the bytes received,
    /// report ID at `[0]`; may be shorter than `len`, so callers check lengths.
    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>>;

    /// Next input report from the vendor collections, waiting at most `timeout`.
    fn read_event(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>>;

    /// Waits between packets. Real devices sleep; the simulator advances a virtual clock.
    fn delay(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn set_feature(&mut self, data: &[u8]) -> Result<()> {
        (**self).set_feature(data)
    }

    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        (**self).get_feature(report_id, len)
    }

    fn read_event(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        (**self).read_event(timeout)
    }

    fn delay(&mut self, d: Duration) {
        (**self).delay(d);
    }
}

/// Direction of a logged packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketDir {
    SetFeature,
    GetFeature,
    Input,
}

/// One logged packet, for tests, `--simulate -v` and `--dry-run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketRecord {
    /// Time since the transport was created (virtual for the simulator: the sum of delays).
    pub at: Duration,
    pub dir: PacketDir,
    pub data: Vec<u8>,
    /// False when a dry-run [`Recorder`] suppressed the packet.
    pub sent: bool,
}

/// Command bytes that only read state. A dry-run [`Recorder`] is stricter: it only forwards
/// packets that are byte-identical to the standard queries ([`protocol::is_standard_query`]).
pub fn is_read_only(command: u8) -> bool {
    matches!(
        command,
        cmd::VERSION | cmd::CURRENT_PROFILE | cmd::READ_BLOCK
    )
}

/// Wraps a transport and logs every packet.
///
/// In dry-run mode it also suppresses every SetFeature that could change the device. Queries
/// (`0x80`, `0x81`, `0x82`) and GetFeature pass through, so a dry run against real hardware
/// plans from the real profile block.
#[derive(Debug)]
pub struct Recorder<T> {
    inner: T,
    log: Vec<PacketRecord>,
    clock: Duration,
    dry_run: bool,
}

impl<T: Transport> Recorder<T> {
    /// Logs and forwards everything.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            log: Vec::new(),
            clock: Duration::ZERO,
            dry_run: false,
        }
    }

    /// Logs everything, forwards only standard read-only queries
    /// ([`protocol::is_standard_query`]: exact `0x80`, `0x81`, `0x82` packets for profiles 1..=5).
    /// GetFeature is forwarded.
    pub fn dry_run(inner: T) -> Self {
        Self {
            dry_run: true,
            ..Self::new(inner)
        }
    }

    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    pub fn log(&self) -> &[PacketRecord] {
        &self.log
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    fn record(&mut self, dir: PacketDir, data: &[u8], sent: bool) {
        self.log.push(PacketRecord {
            at: self.clock,
            dir,
            data: data.to_vec(),
            sent,
        });
    }
}

impl<T: Transport> Transport for Recorder<T> {
    fn set_feature(&mut self, data: &[u8]) -> Result<()> {
        let pass = !self.dry_run || protocol::is_standard_query(data);
        self.record(PacketDir::SetFeature, data, pass);
        if pass {
            self.inner.set_feature(data)?;
        }
        Ok(())
    }

    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        let reply = self.inner.get_feature(report_id, len)?;
        self.record(PacketDir::GetFeature, &reply, true);
        Ok(reply)
    }

    fn read_event(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let ev = self.inner.read_event(timeout)?;
        if let Some(data) = &ev {
            self.record(PacketDir::Input, data, true);
        }
        Ok(ev)
    }

    fn delay(&mut self, d: Duration) {
        self.clock += d;
        self.inner.delay(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol;

    #[test]
    fn dry_run_passes_reads_and_blocks_writes() {
        let mut t = Recorder::dry_run(SimulatedDevice::new());
        t.set_feature(&protocol::select_profile_request(3).unwrap())
            .unwrap();
        t.set_feature(&protocol::current_profile_request()).unwrap();
        let reply = t.get_feature(7, 264).unwrap();
        assert_eq!(reply[2], 1, "profile select must not reach the device");
        assert_eq!(t.inner().active_profile(), 0);
        let sent: Vec<bool> = t.log().iter().map(|r| r.sent).collect();
        assert_eq!(sent, vec![false, true, true]);
    }

    #[test]
    fn dry_run_blocks_non_standard_reads() {
        let mut t = Recorder::dry_run(SimulatedDevice::new());
        let mut bad_profile = protocol::read_block_request(0).unwrap();
        bad_profile[2] = 0xFF;
        let mut odd_len = protocol::current_profile_request().to_vec();
        odd_len.resize(4096, 0);
        let mut other_id = protocol::version_request(protocol::ReportLen::Short);
        other_id[0] = 0x05;
        let mut payload = protocol::read_block_request(1).unwrap();
        payload[100] = 0xAA;
        for p in [&bad_profile[..], &odd_len, &other_id, &payload] {
            t.set_feature(p).unwrap();
        }
        assert!(t.log().iter().all(|r| !r.sent));
        assert!(t.inner().log().is_empty(), "nothing reached the device");
        t.set_feature(&protocol::read_block_request(4).unwrap())
            .unwrap();
        assert_eq!(t.inner().log().len(), 1);
    }

    #[test]
    fn recorder_forwards_everything_when_not_dry() {
        let mut t = Recorder::new(SimulatedDevice::new());
        t.set_feature(&protocol::select_profile_request(3).unwrap())
            .unwrap();
        t.delay(Duration::from_millis(20));
        assert_eq!(t.inner().active_profile(), 3);
        assert!(t.log()[0].sent);
        assert_eq!(t.log().len(), 1);
    }
}
