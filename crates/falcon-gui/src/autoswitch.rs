//! Host-side automatic layer switch by frontmost app (Windows "SYNC Program", spec §8).
//!
//! macOS: a background thread asks `lsappinfo` for the frontmost app every [`POLL`] while the
//! feature is enabled and reports changes. Other platforms: not implemented yet (the watcher
//! never reports anything).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use falcon_core::Profile;

const POLL: Duration = Duration::from_secs(2);

/// The frontmost application.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrontApp {
    pub name: String,
    /// Bundle or executable path, if known.
    pub path: String,
}

/// Whether a `linked_apps` entry names `app`: equal display name (case-insensitive), equal
/// path, or the path's last component (`Safari.app`, `safari`).
pub fn entry_matches(entry: &str, app: &FrontApp) -> bool {
    let e = entry.trim().trim_end_matches('/');
    if e.is_empty() {
        return false;
    }
    let last = app
        .path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let stem = last.strip_suffix(".app").unwrap_or(last);
    e.eq_ignore_ascii_case(&app.name)
        || e == app.path.trim_end_matches('/')
        || (!last.is_empty() && (e.eq_ignore_ascii_case(last) || e.eq_ignore_ascii_case(stem)))
}

/// First profile with a linked app matching `app`.
pub fn match_profile(profiles: &[Profile], app: &FrontApp) -> Option<usize> {
    profiles
        .iter()
        .position(|p| p.linked_apps.iter().any(|e| entry_matches(e, app)))
}

/// Parses `lsappinfo info -only name -only bundlepath` output (`"Key"="Value"` lines).
pub fn parse_lsappinfo(text: &str) -> Option<FrontApp> {
    let mut app = FrontApp::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim().trim_matches('"') {
            "LSDisplayName" => app.name = value,
            "LSBundlePath" => app.path = value,
            _ => {}
        }
    }
    (!app.name.is_empty()).then_some(app)
}

#[cfg(target_os = "macos")]
fn frontmost() -> Option<FrontApp> {
    use std::process::Command;
    let out = Command::new("lsappinfo").arg("front").output().ok()?;
    let asn = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if asn.is_empty() {
        return None;
    }
    let out = Command::new("lsappinfo")
        .args(["info", "-only", "name", "-only", "bundlepath", &asn])
        .output()
        .ok()?;
    parse_lsappinfo(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(target_os = "macos"))]
fn frontmost() -> Option<FrontApp> {
    None
}

/// Background watcher; reports each change of the frontmost app while enabled.
pub struct Watcher {
    enabled: Arc<AtomicBool>,
    rx: Receiver<FrontApp>,
}

impl Watcher {
    pub fn spawn(notify: impl Fn() + Send + 'static) -> Self {
        let enabled = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let flag = enabled.clone();
        std::thread::Builder::new()
            .name("falcon-autoswitch".into())
            .spawn(move || {
                let mut last: Option<FrontApp> = None;
                loop {
                    std::thread::sleep(POLL);
                    if !flag.load(Ordering::Relaxed) {
                        last = None;
                        continue;
                    }
                    let Some(app) = frontmost() else { continue };
                    if last.as_ref() != Some(&app) {
                        last = Some(app.clone());
                        if tx.send(app).is_err() {
                            break;
                        }
                        notify();
                    }
                }
            })
            .expect("spawn autoswitch thread");
        Self { enabled, rx }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn try_iter(&self) -> impl Iterator<Item = FrontApp> + '_ {
        self.rx.try_iter()
    }
}

/// Whether frontmost-app detection exists on this platform.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

#[cfg(test)]
mod tests {
    use super::*;

    fn safari() -> FrontApp {
        FrontApp {
            name: "Safari".into(),
            path: "/Applications/Safari.app".into(),
        }
    }

    #[test]
    fn entries_match_name_path_or_bundle() {
        let app = safari();
        for e in [
            "Safari",
            "safari",
            "/Applications/Safari.app",
            "/Applications/Safari.app/",
            "Safari.app",
        ] {
            assert!(entry_matches(e, &app), "{e}");
        }
        for e in ["", "  ", "Saf", "/Applications", "Mail"] {
            assert!(!entry_matches(e, &app), "{e}");
        }
    }

    #[test]
    fn first_matching_profile_wins() {
        let mut profiles: Vec<Profile> = (0..5).map(Profile::default_for).collect();
        profiles[3].linked_apps.push("Safari".into());
        profiles[4].linked_apps.push("Safari.app".into());
        assert_eq!(match_profile(&profiles, &safari()), Some(3));
        let other = FrontApp {
            name: "Mail".into(),
            path: "/System/Applications/Mail.app".into(),
        };
        assert_eq!(match_profile(&profiles, &other), None);
    }

    #[test]
    fn parses_lsappinfo_output() {
        let out = "\"LSDisplayName\"=\"Brave Browser\"\n\"LSBundlePath\"=\"/Applications/Brave Browser.app\"\n";
        assert_eq!(
            parse_lsappinfo(out),
            Some(FrontApp {
                name: "Brave Browser".into(),
                path: "/Applications/Brave Browser.app".into(),
            })
        );
        assert_eq!(parse_lsappinfo(""), None);
    }
}
