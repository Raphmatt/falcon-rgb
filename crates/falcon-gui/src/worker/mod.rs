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
#[cfg(not(target_arch = "wasm32"))]
pub use native::WorkerHandle;
pub use sim::SharedSim;
#[cfg(target_arch = "wasm32")]
pub use web::WorkerHandle;
