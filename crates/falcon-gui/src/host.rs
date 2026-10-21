//! Host-side key actions (spec §8): the keypad reports the key, the app runs what the user
//! configured. Only paths from the user's own configuration are ever started, never a shell.

use std::path::Path;
use std::process::{Command, Stdio};

/// Program and arguments that open `target` on this OS.
///
/// - macOS: `open <path>` for an existing path (apps, documents, scripts), otherwise
///   `open -a <name>` so plain app names like "Calculator" work.
/// - Windows: the path itself, or `explorer <path>` for non-executables.
/// - Others: the path itself if executable, otherwise `xdg-open <path>`.
pub fn launch_command(target: &str) -> (String, Vec<String>) {
    let target = target.trim().to_owned();
    let exists = Path::new(&target).exists();
    if cfg!(target_os = "macos") {
        if exists {
            ("open".into(), vec![target])
        } else {
            ("open".into(), vec!["-a".into(), target])
        }
    } else if cfg!(windows) {
        let is_exe = Path::new(&target)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
        if is_exe || !exists {
            (target, Vec::new())
        } else {
            ("explorer".into(), vec![target])
        }
    } else if exists && !is_executable(Path::new(&target)) {
        ("xdg-open".into(), vec![target])
    } else {
        (target, Vec::new())
    }
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_file()
        && p.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Starts `target` detached from the app's stdio.
pub fn launch(target: &str) -> std::io::Result<()> {
    if target.trim().is_empty() {
        return Err(std::io::Error::other("empty launch path"));
    }
    let (program, args) = launch_command(target);
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| {
            // Reap the launcher (`open` exits at once) so no zombie is left behind.
            std::thread::spawn(move || child.wait());
        })
}

/// Opens System Settings → Privacy & Security → Input Monitoring (macOS).
pub fn open_input_monitoring_settings() -> std::io::Result<()> {
    Command::new("open")
        .arg(falcon_core::transport::hid::INPUT_MONITORING_SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| {
            std::thread::spawn(move || child.wait());
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_open_forms() {
        let (p, a) = launch_command("/");
        assert_eq!((p.as_str(), a), ("open", vec!["/".to_owned()]));
        let (p, a) = launch_command("  Some App That Does Not Exist  ");
        assert_eq!(p, "open");
        assert_eq!(
            a,
            vec!["-a".to_owned(), "Some App That Does Not Exist".into()]
        );
    }

    #[test]
    fn empty_target_is_refused() {
        assert!(launch("  ").is_err());
    }
}
