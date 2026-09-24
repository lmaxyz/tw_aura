use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub access_token: Option<String>,
    pub last_quality: Option<String>,
}

impl Config {
    pub fn config_path() -> Option<PathBuf> {
        app_data_dir().map(|dir| dir.join("tw_aura.conf"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return None;
        }
        let contents = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "home directory is not available")
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, contents)
    }
}

pub fn app_data_dir() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".local/share/com.lmaxyz/TwAura"))
}
