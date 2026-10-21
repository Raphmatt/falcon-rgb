//! Falcon RGB desktop app for the MAX Falcon-20 RGB keypad.
//!
//! `falcon-gui [--simulate] [--config FILE]`

mod app;
mod autoswitch;
mod host;
mod keymap;
mod preview;
mod theme;
mod views;
mod worker;

use std::path::PathBuf;

use app::{Args, FalconApp};
use eframe::egui;

const USAGE: &str = "usage: falcon-gui [--simulate] [--config FILE]

  --simulate     use an in-memory keypad (config file config.sim.json)
  --config FILE  config file [default: platform config dir]";

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

#[cfg(test)]
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
