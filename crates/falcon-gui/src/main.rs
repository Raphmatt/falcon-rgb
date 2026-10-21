//! Falcon RGB desktop app for the MAX Falcon-20 RGB keypad.
//!
//! `falcon-gui [--simulate] [--config FILE]`

mod app;
mod autoswitch;
mod files;
mod host;
mod keymap;
mod preview;
mod theme;
mod views;
mod worker;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use app::{Args, FalconApp};
#[cfg(not(target_arch = "wasm32"))]
use eframe::egui;

#[cfg(not(target_arch = "wasm32"))]
const USAGE: &str = "usage: falcon-gui [--simulate] [--config FILE]

  --simulate     use an in-memory keypad (config file config.sim.json)
  --config FILE  config file [default: platform config dir]";

#[cfg(not(target_arch = "wasm32"))]
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--simulate" | "-s" => out.simulate = true,
            "--config" => {
                let v = it.next().ok_or("--config needs a file")?;
                out.config = Some(PathBuf::from(v));
            }
            "-h" | "--help" => return Err(String::new()),
            // macOS passes -psn_… to apps started from Finder on old systems.
            s if s.starts_with("-psn_") => {}
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            eprintln!("{USAGE}");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Falcon RGB")
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([920.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Falcon RGB",
        options,
        Box::new(|cc| Ok(Box::new(FalconApp::new(cc, args)))),
    )
}

/// Browser entry point (trunk): runs the app on `<canvas id="falcon">`. `?simulate` in the URL
/// starts with the simulated keypad.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    let simulate = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|q| web_sys::UrlSearchParams::new_with_str(&q).ok())
        .is_some_and(|p| p.has("simulate"));
    let args = Args { simulate };
    wasm_bindgen_futures::spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("falcon"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("index.html has a <canvas id=\"falcon\">");
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| Ok(Box::new(FalconApp::new(cc, args)))),
            )
            .await;
        if let Err(e) = started {
            web_sys::console::error_1(&e);
        }
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn parse(a: &[&str]) -> Result<Args, String> {
        parse_args(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn args() {
        let a = parse(&["--simulate", "--config", "x.json"]).unwrap();
        assert!(a.simulate);
        assert_eq!(a.config, Some(PathBuf::from("x.json")));
        assert!(!parse(&[]).unwrap().simulate);
        assert!(parse(&["--config"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
        assert_eq!(parse(&["--help"]).unwrap_err(), "");
        assert!(parse(&["-psn_0_12345"]).is_ok());
    }
}
