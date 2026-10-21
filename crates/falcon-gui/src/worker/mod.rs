//! Device I/O off the UI path. The UI only exchanges [`Command`]s and [`Update`]s with a
//! [`WorkerHandle`]: a background thread on the desktop ([`native`]), an async task on the
//! browser's event loop ([`web`], WebHID).

mod messages;
mod sim;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
mod webhid;

pub use messages::{Command, Connection, Job, Mode, SimAction, Update};

/// Seconds since the Unix epoch, for backup files.
fn now_unix() -> Option<u64> {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
#[cfg(not(target_arch = "wasm32"))]
pub use native::WorkerHandle;
pub use sim::SharedSim;
#[cfg(target_arch = "wasm32")]
pub use web::WorkerHandle;
