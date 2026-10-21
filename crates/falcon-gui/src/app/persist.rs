//! Where the configuration lives: a JSON file natively (`falcon_core::store`), `localStorage`
//! in the browser. Same JSON either way, so layers can move between the two via export/import.

use falcon_core::Config;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// The configuration's home.
#[derive(Debug, Clone)]
pub enum ConfigStore {
    #[cfg(not(target_arch = "wasm32"))]
    File(PathBuf),
    /// `localStorage` entry with this key.
    #[cfg(target_arch = "wasm32")]
    Browser(&'static str),
    /// Nowhere (no config dir, or no storage): edits live until the app closes.
    None,
}

impl ConfigStore {
    /// The default store: the `--config` file or the platform config dir, `config.sim.json`
    /// when simulating.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn default_for(simulate: bool, file: Option<PathBuf>) -> Self {
        let path = file.or_else(|| {
            let p = falcon_core::store::default_config_path()?;
            Some(if simulate {
                p.with_file_name("config.sim.json")
            } else {
                p
            })
        });
        path.map_or(Self::None, Self::File)
    }

    /// The default store in the browser; the simulator keeps its own entry.
    #[cfg(target_arch = "wasm32")]
    pub fn default_for(simulate: bool) -> Self {
        if web::storage().is_none() {
            return Self::None;
        }
        Self::Browser(if simulate {
            "falcon-rgb/config.sim"
        } else {
            "falcon-rgb/config"
        })
    }

    /// Loads the config; a missing entry yields [`Config::default`].
    pub fn load(&self) -> Result<Config, String> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::File(p) => falcon_core::store::load_config(p).map_err(|e| e.to_string()),
            #[cfg(target_arch = "wasm32")]
            Self::Browser(key) => match web::storage().and_then(|s| s.get_item(key).ok().flatten())
            {
                Some(json) => {
                    let c: Config = serde_json::from_str(&json).map_err(|e| e.to_string())?;
                    c.validate().map_err(|e| e.to_string())?;
                    Ok(c)
                }
                None => Ok(Config::default()),
            },
            Self::None => Ok(Config::default()),
        }
    }

    /// Saves the config. `Ok(false)` when there is nowhere to save.
    pub fn save(&self, config: &Config) -> Result<bool, String> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::File(p) => falcon_core::store::save_config(p, config)
                .map(|()| true)
                .map_err(|e| format!("could not write {}: {e}", p.display())),
            #[cfg(target_arch = "wasm32")]
            Self::Browser(key) => {
                let json = serde_json::to_string(config).map_err(|e| e.to_string())?;
                web::storage()
                    .ok_or("browser storage is not available")?
                    .set_item(key, &json)
                    .map(|()| true)
                    .map_err(|_| "the browser refused to store the configuration".to_owned())
            }
            Self::None => Ok(false),
        }
    }

    /// For the status bar.
    pub fn describe(&self) -> String {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::File(p) => p.display().to_string(),
            #[cfg(target_arch = "wasm32")]
            Self::Browser(_) => "this browser's local storage".into(),
            Self::None => "(not saved)".into(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    pub fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }
}
