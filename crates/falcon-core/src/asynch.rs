//! Async twin of [`crate::Falcon`] for transports that must not block (WebHID in the browser).
//!
//! Same plans ([`crate::encode`]), parsers and delays as the sync device, so both send identical
//! packets in identical order (see the tests); only the waiting differs.

use std::time::Duration;

use crate::device::InputReport;
use crate::encode::{self, Step, StepKind};
use crate::error::Result;
use crate::model::{Lighting, Profile};
use crate::protocol::{
    self, Block, FirmwareVersion, REPORT_ID, ReportLen, delay, parse_input_report,
};
use crate::transport::{PacketDir, PacketRecord};

/// Non-blocking counterpart of [`crate::Transport`]. Buffers include the report ID at `[0]`.
///
/// No `Send` bound: the only implementation lives on the browser's single thread.
#[allow(async_fn_in_trait)]
pub trait AsyncTransport {
    /// SetFeature with `data[0]` = report ID.
    async fn set_feature(&mut self, data: &[u8]) -> Result<()>;

    /// GetFeature of `report_id`, `len` bytes expected; report ID at `[0]` of the result.
    async fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>>;

    /// Waits between packets.
    async fn delay(&mut self, d: Duration);

    /// The next input report already received, if any. Never waits; fails once the device is
    /// gone.
    fn try_event(&mut self) -> Result<Option<Vec<u8>>>;
}

/// Logs every packet like [`crate::Recorder`] (no dry-run mode).
#[derive(Debug)]
pub struct AsyncRecorder<T> {
    inner: T,
    log: Vec<PacketRecord>,
    clock: Duration,
}

impl<T: AsyncTransport> AsyncRecorder<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            log: Vec::new(),
            clock: Duration::ZERO,
        }
    }

    pub fn log(&self) -> &[PacketRecord] {
        &self.log
    }

    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    fn record(&mut self, dir: PacketDir, data: &[u8]) {
        self.log.push(PacketRecord {
            at: self.clock,
            dir,
            data: data.to_vec(),
            sent: true,
        });
    }
}

impl<T: AsyncTransport> AsyncTransport for AsyncRecorder<T> {
    async fn set_feature(&mut self, data: &[u8]) -> Result<()> {
        self.record(PacketDir::SetFeature, data);
        self.inner.set_feature(data).await
    }

    async fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        let reply = self.inner.get_feature(report_id, len).await?;
        self.record(PacketDir::GetFeature, &reply);
        Ok(reply)
    }

    async fn delay(&mut self, d: Duration) {
        self.clock += d;
        self.inner.delay(d).await;
    }

    fn try_event(&mut self) -> Result<Option<Vec<u8>>> {
        let ev = self.inner.try_event()?;
        if let Some(data) = &ev {
            self.record(PacketDir::Input, data);
        }
        Ok(ev)
    }
}

/// The keypad behind an [`AsyncTransport`]. Methods match [`crate::Falcon`].
#[derive(Debug)]
pub struct AsyncFalcon<T: AsyncTransport> {
    transport: T,
}

impl<T: AsyncTransport> AsyncFalcon<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// SetFeature, wait, GetFeature of the same length, wait. Returns the raw reply.
    pub async fn query(&mut self, packet: &[u8], wait: Duration) -> Result<Vec<u8>> {
        self.transport.set_feature(packet).await?;
        self.transport.delay(wait).await;
        let reply = self.transport.get_feature(REPORT_ID, packet.len()).await?;
        self.transport.delay(wait).await;
        Ok(reply)
    }

    /// Sends one packet and waits.
    pub async fn send(&mut self, packet: &[u8], wait: Duration) -> Result<()> {
        self.transport.set_feature(packet).await?;
        self.transport.delay(wait).await;
        Ok(())
    }

    /// Runs a plan. Returns the replies of the [`StepKind::SetThenGet`] steps, in order.
    pub async fn execute(&mut self, steps: &[Step]) -> Result<Vec<Vec<u8>>> {
        let mut replies = Vec::new();
        for step in steps {
            match step.kind {
                StepKind::Set => self.send(&step.packet, step.delay_after).await?,
                StepKind::SetThenGet => {
                    replies.push(self.query(&step.packet, step.delay_after).await?);
                }
            }
        }
        Ok(replies)
    }

    /// Waits [`delay::CONNECT_SETTLE`], as Windows does after the device appears.
    pub async fn settle(&mut self) {
        self.transport.delay(delay::CONNECT_SETTLE).await;
    }

    /// Firmware version (`0x80`); see [`crate::Falcon::firmware_version`].
    pub async fn firmware_version(&mut self, len: ReportLen) -> Result<Option<FirmwareVersion>> {
        let reply = self
            .query(&protocol::version_request(len), delay::DEFAULT)
            .await?;
        protocol::parse_version(&reply)
    }

    /// Active profile, 0-based (`0x81`).
    pub async fn current_profile(&mut self) -> Result<usize> {
        let reply = self
            .query(&protocol::current_profile_request(), delay::DEFAULT)
            .await?;
        protocol::parse_current_profile(&reply)
    }

    /// Makes `profile` (0-based) the active layer (`0x01`).
    pub async fn select_profile(&mut self, profile: usize) -> Result<()> {
        self.send(&protocol::select_profile_request(profile)?, delay::DEFAULT)
            .await
    }

    /// The 200-byte block of `profile` (0-based).
    pub async fn read_block(&mut self, profile: usize) -> Result<Block> {
        let step = encode::plan_read_block(profile)?;
        protocol::parse_block(&self.query(&step.packet, step.delay_after).await?)
    }

    /// Full download; see [`crate::Falcon::write_profile`].
    pub async fn write_profile(&mut self, profile_idx: usize, profile: &Profile) -> Result<()> {
        protocol::check_profile(profile_idx)?;
        profile.validate()?;
        self.transport.delay(delay::PRE_DOWNLOAD).await;
        let current = self.read_block(profile_idx).await?;
        let steps = encode::plan_full_download(profile_idx, profile, &current)?;
        self.execute(&steps).await.map(drop)
    }

    /// Lighting-only update; see [`crate::Falcon::apply_lighting`].
    pub async fn apply_lighting(&mut self, profile_idx: usize, lighting: &Lighting) -> Result<()> {
        protocol::check_profile(profile_idx)?;
        lighting.validate()?;
        let current = self.read_block(profile_idx).await?;
        self.transport.delay(delay::LIGHTING_GAP).await;
        let steps = encode::plan_lighting_apply(profile_idx, lighting, &current)?;
        self.execute(&steps).await.map(drop)
    }

    /// The next input report already received, if any.
    pub fn poll_event(&mut self) -> Result<Option<InputReport>> {
        Ok(self.transport.try_event()?.map(|raw| InputReport {
            event: parse_input_report(&raw),
            raw,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes::Function;
    use crate::model::{Effect, KeyAssignment, Macro, MacroEvent, Repeat, Rgb};
    use crate::transport::{Recorder, SimulatedDevice, Transport};
    use crate::{DeviceEvent, Falcon};

    /// A sync transport seen through the async trait (each future is ready at once).
    struct SyncAsAsync<T>(T);

    impl<T: Transport> AsyncTransport for SyncAsAsync<T> {
        async fn set_feature(&mut self, data: &[u8]) -> Result<()> {
            self.0.set_feature(data)
        }

        async fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
            self.0.get_feature(report_id, len)
        }

        async fn delay(&mut self, d: Duration) {
            self.0.delay(d);
        }

        fn try_event(&mut self) -> Result<Option<Vec<u8>>> {
            self.0.read_event(Duration::ZERO)
        }
    }

    fn pair() -> (
        Falcon<Recorder<SimulatedDevice>>,
        AsyncFalcon<AsyncRecorder<SyncAsAsync<SimulatedDevice>>>,
    ) {
        (
            Falcon::new(Recorder::new(SimulatedDevice::new())),
            AsyncFalcon::new(AsyncRecorder::new(SyncAsAsync(SimulatedDevice::new()))),
        )
    }

    fn profile() -> Profile {
        let mut p = Profile::default_for(1);
        p.keys[0] = KeyAssignment::Key(0x04);
        p.keys[1] = KeyAssignment::Macro(Macro {
            repeat: Repeat::Times(2),
            events: vec![MacroEvent::down(0x05, 0), MacroEvent::up(0x05, 3)],
        });
        p.keys[2] = KeyAssignment::Combo(vec![0xE1, 0x05]);
        p.keys[19] = KeyAssignment::Function(Function::ShowDesktop);
        p.lighting.effect = Effect::Custom;
        p.lighting.key_colors[0] = Rgb::new(1, 2, 3);
        p
    }

    #[test]
    fn same_packets_as_the_sync_device() {
        let (mut s, mut a) = pair();
        let p = profile();
        let mut l = Lighting::default_for_profile(3);
        l.color = Rgb::new(0xFF, 0x88, 0x00);

        assert_eq!(s.current_profile().unwrap(), 0);
        s.select_profile(2).unwrap();
        s.write_profile(1, &p).unwrap();
        s.apply_lighting(3, &l).unwrap();
        assert_eq!(s.firmware_version(ReportLen::Long).unwrap(), None);

        pollster::block_on(async {
            assert_eq!(a.current_profile().await.unwrap(), 0);
            a.select_profile(2).await.unwrap();
            a.write_profile(1, &p).await.unwrap();
            a.apply_lighting(3, &l).await.unwrap();
            assert_eq!(a.firmware_version(ReportLen::Long).await.unwrap(), None);
        });

        assert_eq!(s.transport().log(), a.transport().log());
        assert!(a.transport().log().len() > 10);
        assert_eq!(
            a.transport().inner().0.block(1),
            s.transport().inner().block(1)
        );
        assert_eq!(
            a.transport().inner().0.block(3),
            s.transport().inner().block(3)
        );
    }

    #[test]
    fn events_and_validation() {
        let (_, mut a) = pair();
        assert_eq!(a.poll_event().unwrap(), None);
        a.transport_mut()
            .inner_mut()
            .0
            .push_event([0x04, 0x01, 0x00, 3, 0x00, 0x00]);
        let ev = a.poll_event().unwrap().unwrap();
        assert_eq!(ev.event, Some(DeviceEvent::ProfileChanged { profile: 2 }));
        assert_eq!(a.transport().log().last().unwrap().dir, PacketDir::Input);

        let mut bad = profile();
        bad.lighting.brightness = 0;
        a.transport_mut().clear_log();
        assert!(pollster::block_on(a.write_profile(0, &bad)).is_err());
        assert!(pollster::block_on(a.select_profile(5)).is_err());
        assert!(
            a.transport().log().is_empty(),
            "nothing sent for invalid input"
        );
    }
}
