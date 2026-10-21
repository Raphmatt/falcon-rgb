//! JSON persistence of the [`Config`] (spec §9).
//!
//! Default location: `<config dir>/falcon-rgb/config.json`, i.e.
//! `~/Library/Application Support/falcon-rgb/config.json` on macOS, `~/.config/falcon-rgb/` on
//! Linux and `%APPDATA%\falcon-rgb\` on Windows.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::Config;

/// Directory name under the platform config dir.
pub const APP_DIR: &str = "falcon-rgb";
/// File name of the main configuration.
pub const CONFIG_FILE: &str = "config.json";

/// `<config dir>/falcon-rgb`, if the platform has a config dir.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR))
}

/// `<config dir>/falcon-rgb/config.json`.
pub fn default_config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(CONFIG_FILE))
}

/// Loads a config; a missing file yields [`Config::default`]. Invalid JSON is an error, so a
/// broken file is never silently overwritten.
pub fn load_config(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Writes pretty JSON atomically: temp file in the same directory, fsync, rename.
pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(CONFIG_FILE);
    let tmp = dir.join(format!(".{file_name}.tmp"));
    let json = serde_json::to_string_pretty(config)?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyAssignment;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("falcon-core-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn missing_file_gives_defaults_and_save_round_trips() {
        let dir = temp_dir("store");
        let path = dir.join("nested").join("config.json");
        assert_eq!(load_config(&path).unwrap(), Config::default());
        let mut c = Config::default();
        c.profiles[2].keys[7] = KeyAssignment::Key(0x2C);
        save_config(&path, &c).unwrap();
        assert_eq!(load_config(&path).unwrap(), c);
        assert!(!path.with_file_name(".config.json.tmp").exists());
        fs::write(&path, "{ not json").unwrap();
        assert!(matches!(load_config(&path), Err(Error::Json(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_path_ends_in_app_dir() {
        if let Some(p) = default_config_path() {
            assert!(p.ends_with("falcon-rgb/config.json"));
        }
    }
}
