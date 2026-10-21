//! A [`SimulatedDevice`] shared between the worker's transport and the simulator controls, so the
//! GUI can inject device-side events (layer key, host key) while the worker owns the transport.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use falcon_core::protocol;
use falcon_core::{Result, SimulatedDevice, Transport, layout};

#[derive(Debug, Clone, Default)]
pub struct SharedSim(Arc<Mutex<SimulatedDevice>>);

impl SharedSim {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lock(&self) -> MutexGuard<'_, SimulatedDevice> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The user pressed a layer key on the keypad: the device switches and reports it (spec §7).
    pub fn press_layer(&self, profile: usize) -> Result<()> {
        protocol::check_profile(profile)?;
        let mut sim = self.lock();
        // Straight to the simulator, so the packet log only shows real host traffic.
        sim.set_feature(&protocol::select_profile_request(profile)?)?;
        sim.clear_log();
        sim.push_event([0x04, 0x01, 0x00, profile as u8 + 1, 0x00, 0x00]);
        Ok(())
    }

    /// The user pressed key `k`, whose device byte reports to the host (launch program).
    pub fn press_host_key(&self, k: usize) {
        if let Some(hw) = layout::k_to_hw(k) {
            self.lock()
                .push_event([0x05, 0x02, 0x19, 0x5D, 0x60, 0x08, hw]);
        }
    }

    /// One example of every spec §7 event.
    pub fn push_demo_events(&self) {
        self.lock().push_demo_events();
    }
}

impl Transport for SharedSim {
    fn set_feature(&mut self, data: &[u8]) -> Result<()> {
        let mut sim = self.lock();
        let r = sim.set_feature(data);
        // The worker's Recorder keeps the log; don't let the simulator's copy grow forever.
        sim.clear_log();
        r
    }

    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        let mut sim = self.lock();
        let r = sim.get_feature(report_id, len);
        sim.clear_log();
        r
    }

    fn read_event(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut sim = self.lock();
        let r = sim.read_event(timeout);
        sim.clear_log();
        r
    }

    fn delay(&mut self, d: Duration) {
        self.lock().delay(d);
    }
}
