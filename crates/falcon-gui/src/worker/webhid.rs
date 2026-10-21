//! The keypad through the browser's WebHID API (Chrome, Edge, Brave, Opera), as an
//! [`AsyncTransport`] for [`falcon_core::AsyncFalcon`].
//!
//! - `requestDevice` filters on the configuration collection (`FF01:0001`); Chrome then grants the
//!   device's other non-keyboard collections too, including the vendor event collection
//!   (`FF00:FF00`, report 4). Keyboard collections are never exposed to pages.
//! - `sendFeatureReport` takes the report ID separately, so `data[0]` is split off.
//!   `receiveFeatureReport` includes the ID on macOS (confirmed); replies without it are
//!   normalised to the hidapi layout, `reply[0] = 07`.
//! - Only 264-byte reports are sent, like the hid transport on macOS: WebHID uses the same IOKit
//!   path there, where shorter feature reports make the keypad stop answering.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use falcon_core::protocol::{
    CONFIG_USAGE, CONFIG_USAGE_PAGE, LONG_LEN, PID, VENDOR_USAGE_PAGE_MIN, VID,
};
use falcon_core::{AsyncTransport, Error, Result};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Hid, HidDevice, HidDeviceFilter, HidDeviceRequestOptions, HidInputReportEvent};

fn js_error(what: &str, e: &wasm_bindgen::JsValue) -> Error {
    let detail = e
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"));
    Error::Hid(format!("{what}: {detail}"))
}

/// `navigator.hid`, if the browser has WebHID (and the page is a secure context).
pub fn hid() -> Option<Hid> {
    let navigator = web_sys::window()?.navigator();
    let has = js_sys::Reflect::has(&navigator, &"hid".into()).unwrap_or(false);
    has.then(|| navigator.hid())
}

pub fn is_keypad(d: &HidDevice) -> bool {
    d.vendor_id() == VID && d.product_id() == PID
}

/// A keypad the user already allowed for this site, if plugged in.
pub async fn granted(hid: &Hid) -> Result<Option<HidDevice>> {
    let list = hid
        .get_devices()
        .await
        .map_err(|e| js_error("getDevices", &e))?;
    Ok(list.iter().find(is_keypad))
}

/// Shows the browser's device chooser. `None` if the user cancelled.
pub async fn request(hid: &Hid) -> Result<Option<HidDevice>> {
    let filter = HidDeviceFilter::new();
    filter.set_vendor_id(u32::from(VID));
    filter.set_product_id(PID);
    filter.set_usage_page(CONFIG_USAGE_PAGE);
    filter.set_usage(CONFIG_USAGE);
    let options = HidDeviceRequestOptions::new(&[filter]);
    match hid.request_device(&options).await {
        Ok(list) => Ok(list.iter().find(is_keypad)),
        // Chrome rejects with NotFoundError when the chooser is closed without a pick.
        Err(e)
            if e.dyn_ref::<js_sys::Error>()
                .is_some_and(|e| e.name() == "NotFoundError") =>
        {
            Ok(None)
        }
        Err(e) => Err(js_error("requestDevice", &e)),
    }
}

/// Resolves after `d` (`setTimeout`).
pub async fn sleep(d: Duration) {
    let ms = i32::try_from(d.as_millis()).unwrap_or(i32::MAX);
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = promise.await;
}

/// An opened keypad.
pub struct WebHid {
    device: HidDevice,
    events: Rc<RefCell<VecDeque<Vec<u8>>>>,
    _on_input: Closure<dyn FnMut(HidInputReportEvent)>,
}

impl WebHid {
    pub async fn open(device: HidDevice) -> Result<Self> {
        if !device.opened() {
            device.open().await.map_err(|e| js_error("open", &e))?;
        }
        let vendor = vendor_input_reports(&device);
        let events: Rc<RefCell<VecDeque<Vec<u8>>>> = Rc::default();
        let queue = events.clone();
        let on_input =
            Closure::<dyn FnMut(HidInputReportEvent)>::new(move |ev: HidInputReportEvent| {
                // Chrome also delivers the consumer collection (media keys, report 3); the desktop
                // transport reads vendor collections only, so do the same here.
                if !vendor.is_empty() && !vendor.contains(&ev.report_id()) {
                    return;
                }
                let data = ev.data();
                let mut raw = Vec::with_capacity(data.byte_length() + 1);
                raw.push(ev.report_id());
                raw.extend((0..data.byte_length()).map(|i| data.get_uint8(i)));
                let mut q = queue.borrow_mut();
                // Nobody polling (e.g. a background tab): keep only the recent past.
                if q.len() >= 256 {
                    q.pop_front();
                }
                q.push_back(raw);
            });
        device.set_oninputreport(Some(on_input.as_ref().unchecked_ref()));
        Ok(Self {
            device,
            events,
            _on_input: on_input,
        })
    }

    pub fn name(&self) -> String {
        self.device.product_name()
    }
}

impl Drop for WebHid {
    fn drop(&mut self) {
        self.device.set_oninputreport(None);
        // Fire and forget: the page may be closing.
        let _ = self.device.close();
    }
}

impl AsyncTransport for WebHid {
    async fn set_feature(&mut self, data: &[u8]) -> Result<()> {
        if data.len() != LONG_LEN {
            return Err(Error::ReportLengthBlocked(data.len()));
        }
        let mut payload = data[1..].to_vec();
        self.device
            .send_feature_report_with_u8_slice(data[0], &mut payload)
            .map_err(|e| js_error("sendFeatureReport", &e))?
            .await
            .map_err(|e| js_error("sendFeatureReport", &e))?;
        Ok(())
    }

    async fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>> {
        let view = self
            .device
            .receive_feature_report(report_id)
            .await
            .map_err(|e| js_error("receiveFeatureReport", &e))?;
        let bytes: Vec<u8> = (0..view.byte_length()).map(|i| view.get_uint8(i)).collect();
        Ok(normalise_feature_reply(report_id, len, bytes))
    }

    async fn delay(&mut self, d: Duration) {
        sleep(d).await;
    }

    fn try_event(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(self.events.borrow_mut().pop_front())
    }
}

/// Input report IDs of the vendor collections (usage page `0xFF00`+): 4 (events) and 6.
fn vendor_input_reports(device: &HidDevice) -> Vec<u8> {
    let mut ids = Vec::new();
    for c in device.collections().iter() {
        if c.get_usage_page().unwrap_or(0) < VENDOR_USAGE_PAGE_MIN {
            continue;
        }
        if let Some(reports) = c.get_input_reports() {
            ids.extend(reports.iter().filter_map(|r| r.get_report_id()));
        }
    }
    ids
}

/// `receiveFeatureReport` returns the report with its ID on macOS, without it elsewhere: keep a
/// full-length reply that starts with the ID, prepend the ID to anything else.
fn normalise_feature_reply(report_id: u8, len: usize, bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() == len && bytes.first() == Some(&report_id) {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.push(report_id);
    out.extend(bytes);
    out
}
