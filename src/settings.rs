use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub rotation_enabled: bool,
    pub auto_save: bool,
    pub threshold: u32,
    pub directories: Vec<String>,
    pub ref_dirs: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rotation_enabled: false,
            auto_save: true,
            threshold: 2,
            directories: Vec::new(),
            ref_dirs: Vec::new(),
        }
    }
}

impl Settings {
    fn config_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(dir)
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        }
        .join("imphash")
    }

    fn path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                match serde_json::from_str(&content) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Warning: Failed to parse settings: {}, using defaults", e);
                        Settings::default()
                    }
                }
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("Warning: Failed to read settings: {}", e);
                }
                Settings::default()
            }
        }
    }

    pub fn save(&self) {
        let dir = Self::config_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("Warning: Failed to create config dir {:?}: {}", dir, e);
            return;
        }
        match serde_json::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = std::fs::write(Self::path(), &content) {
                    eprintln!("Warning: Failed to write settings: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to serialize settings: {}", e);
            }
        }
    }
}
