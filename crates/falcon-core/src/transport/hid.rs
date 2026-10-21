//! Real hardware through hidapi (feature `hid`).
//!
//! - The config collection (usage page `0xFF01`, usage 1) is selected by usage, never by
//!   interface number, and opened by path (spec §1). Keyboard collections are never opened, but
//!   on macOS even the config collection needs Input Monitoring permission: without it the open
//!   fails with `0xE00002E2` (not permitted), reported as [`Error::PermissionDenied`].
//! - hidapi is built with `macos-shared-device`, so `HidApi::new()` opens non-exclusively and the
//!   keypad keeps typing.
//! - Feature reports follow the hidapi convention: `send_feature_report(buf)` with
//!   `buf[0] = 0x07` sends all 264 bytes including the ID; `get_feature_report(buf)` with
//!   `buf[0] = 0x07` fills `buf` *including* the report ID at `[0]` and returns that length
//!   (macOS: `IOHIDDeviceGetReport` with a non-zero ID writes the ID too; Windows `HidD_GetFeature`
//!   and Linux hidraw behave the same; confirmed on macOS).
//! - On macOS only 264-byte feature reports are sent: an 8-byte one fails in IOKit and leaves the
//!   keypad unresponsive until replugged ([`SHORT_REPORTS_BLOCKED`]).
//! - On macOS all collections of one interface share a path, so the config device doubles as an
//!   event reader when a vendor collection has the same path. If that path also carries a
//!   keyboard/keypad collection, macOS may ask for Input Monitoring; [`HidTransport::warnings`]
//!   says so (we still open it).
//! - Events are read from vendor collections only. Windows reads all seven collections,
//!   but opening keyboard collections triggers the Input Monitoring prompt on macOS
//!   and they carry typing. On macOS the vendor path is shared with the mouse collection, so
//!   report-1 mouse reports arrive too ([`crate::DeviceEvent::MouseReport`]).

use std::ffi::CString;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice, HidError};
use serde::{Deserialize, Serialize};

use super::Transport;
use crate::error::{Error, Result};
use crate::protocol::{
    CONFIG_USAGE, CONFIG_USAGE_PAGE, LONG_LEN, PID, SHORT_REPORTS_BLOCKED, VENDOR_USAGE_PAGE_MIN,
    VID,
};

impl From<HidError> for Error {
    fn from(e: HidError) -> Self {
        Self::Hid(e.to_string())
    }
}

/// System Settings pane where macOS grants Input Monitoring (open it with `open URL`).
pub const INPUT_MONITORING_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";

/// Whether a hidapi message is macOS refusing the open: `kIOReturnNotPermitted` (`0xE00002E2`),
/// i.e. the app lacks Input Monitoring permission (TCC).
pub fn is_permission_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("0xe00002e2") || m.contains("not permitted")
}

/// One HID top-level collection of the keypad, for `list`/`doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub path: String,
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    /// bcdDevice.
    pub release: u16,
}

impl CollectionInfo {
    /// The collection all commands go to.
    pub fn is_config(&self) -> bool {
        self.usage_page == CONFIG_USAGE_PAGE && self.usage == CONFIG_USAGE
    }

    /// Vendor-defined page: candidates for device events.
    pub fn is_vendor(&self) -> bool {
        self.usage_page >= VENDOR_USAGE_PAGE_MIN
    }

    /// Generic Desktop keyboard (0x01/0x06) or keypad (0x01/0x07).
    pub fn is_keyboard(&self) -> bool {
        self.usage_page == 0x01 && matches!(self.usage, 0x06 | 0x07)
    }

    /// Short description of well-known usages.
    pub fn usage_name(&self) -> &'static str {
        match (self.usage_page, self.usage) {
            (0x01, 0x02) => "mouse",
            (0x01, 0x06) => "keyboard",
            (0x01, 0x07) => "keypad",
            (0x01, 0x80) => "system control",
            (0x0C, 0x01) => "consumer control",
            (CONFIG_USAGE_PAGE, CONFIG_USAGE) => "vendor (config)",
            (p, _) if p >= VENDOR_USAGE_PAGE_MIN => "vendor",
            _ => "other",
        }
    }
}

fn collections_of(api: &HidApi) -> Vec<CollectionInfo> {
    api.device_list()
        .filter(|d| d.vendor_id() == VID && d.product_id() == PID)
        .map(|d| CollectionInfo {
            path: d.path().to_string_lossy().into_owned(),
            interface: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            manufacturer: d.manufacturer_string().map(str::to_owned),
            product: d.product_string().map(str::to_owned),
            serial: d.serial_number().map(str::to_owned),
            release: d.release_number(),
        })
        .collect()
}

/// All HID collections of `195D:6008`, without opening anything.
pub fn list_collections() -> Result<Vec<CollectionInfo>> {
    Ok(collections_of(&HidApi::new()?))
}

/// hidapi transport bound to the config collection, plus readers for the vendor collections.
pub struct HidTransport {
    _api: HidApi,
    config: HidDevice,
    config_info: CollectionInfo,
    /// Read the config device for events too (a vendor collection shares its path).
    config_reads_events: bool,
    event_devices: Vec<(CollectionInfo, HidDevice)>,
    warnings: Vec<String>,
}

impl std::fmt::Debug for HidTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HidTransport")
            .field("config", &self.config_info.path)
            .field("config_reads_events", &self.config_reads_events)
            .field(
                "event_devices",
                &self
                    .event_devices
                    .iter()
                    .map(|(i, _)| &i.path)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

fn open(api: &HidApi, path: &str) -> Result<HidDevice> {
    let c = CString::new(path).map_err(|_| Error::Hid(format!("bad device path {path:?}")))?;
    api.open_path(&c).map_err(|e| {
        let msg = e.to_string();
        if is_permission_error(&msg) {
            Error::PermissionDenied(msg)
        } else {
            Error::Hid(msg)
        }
    })
}

impl HidTransport {
    /// Opens the first keypad found.
    pub fn open() -> Result<Self> {
        let api = HidApi::new()?;
        let all = collections_of(&api);
        if all.is_empty() {
            return Err(Error::DeviceNotFound);
        }
        let config_info = all
            .iter()
            .find(|c| c.is_config())
            .cloned()
            .ok_or(Error::ConfigCollectionNotFound)?;
        let config = open(&api, &config_info.path)?;

        let mut config_reads_events = false;
        let mut event_devices: Vec<(CollectionInfo, HidDevice)> = Vec::new();
        for info in all.iter().filter(|c| c.is_vendor()) {
            if info.path == config_info.path {
                config_reads_events = true;
            } else if !event_devices.iter().any(|(i, _)| i.path == info.path) {
                // Event readers are best effort: a collection that refuses to open only costs
                // its events.
                if let Ok(dev) = open(&api, &info.path) {
                    event_devices.push((info.clone(), dev));
                }
            }
        }
        let mut warnings = Vec::new();
        let mut opened = vec![config_info.path.as_str()];
        opened.extend(event_devices.iter().map(|(i, _)| i.path.as_str()));
        for path in opened {
            if let Some(kbd) = all.iter().find(|c| c.path == path && c.is_keyboard()) {
                warnings.push(format!(
                    "{path} also carries a {} collection (usage {:04X}:{:04X}); macOS may ask for \
                     Input Monitoring permission",
                    kbd.usage_name(),
                    kbd.usage_page,
                    kbd.usage
                ));
            }
        }
        Ok(Self {
            _api: api,
            config,
            config_info,
            config_reads_events,
            event_devices,
            warnings,
        })
    }

    /// Non-fatal findings from [`HidTransport::open`] (e.g. a keyboard collection sharing an
    /// opened path).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The collection commands are sent to.
    pub fn config_info(&self) -> &CollectionInfo {
        &self.config_info
    }

    /// Paths read for input reports.
    pub fn event_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        if self.config_reads_events {
            paths.push(self.config_info.path.clone());
        }
        paths.extend(self.event_devices.iter().map(|(i, _)| i.path.clone()));
        paths
    }

    /// Raw HID report descriptor of the config collection (see [`crate::descriptor`]).
    pub fn report_descriptor(&self) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 4096];
        let n = self.config.get_report_descriptor(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    fn readers(&self) -> impl Iterator<Item = &HidDevice> {
        self.config_reads_events
            .then_some(&self.config)
            .into_iter()
            .chain(self.event_devices.iter().map(|(_, d)| d))
    }
}

/// Refuses lengths that hang the keypad on macOS ([`SHORT_REPORTS_BLOCKED`]).
fn check_len(len: usize) -> Result<()> {
    if SHORT_REPORTS_BLOCKED && len != LONG_LEN {
        Err(Error::ReportLengthBlocked(len))
    } else {
        Ok(())
    }
}

impl Transport for HidTransport {
    fn set_feature(&mut self, data: &[u8]) -> Result<()> {
        check_len(data.len())?;
        Ok(self.config.send_feature_report(data)?)
    }

    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        check_len(len)?;
        let mut buf = vec![0u8; len.max(1)];
        buf[0] = report_id;
        let n = self.config.get_feature_report(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    fn read_event(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + timeout;
        loop {
            for dev in self.readers() {
                // Timeout 0 = non-blocking, independent of the device's blocking mode.
                let n = dev.read_timeout(&mut buf, 0)?;
                if n > 0 {
                    return Ok(Some(buf[..n].to_vec()));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_errors_are_recognised() {
        assert!(is_permission_error(
            "hid_open_path: failed to open IOHIDDevice from mach entry: (0xE00002E2) \
             (iokit/common) not permitted"
        ));
        assert!(is_permission_error("(0xe00002e2)"));
        assert!(!is_permission_error(
            "hid_open_path: failed to open IOHIDDevice from mach entry: (0xE00002C0)"
        ));
    }
}
