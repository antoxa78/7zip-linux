use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub default_format: String,
    pub default_compression_level: u32,
    pub default_overwrite: String,
    pub show_hidden_by_default: bool,
    pub remember_window_size: bool,
    pub last_directory: Option<String>,
    pub window_width: i32,
    pub window_height: i32,
    pub color_scheme: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_format: "7z".into(),
            default_compression_level: 3,
            default_overwrite: "overwrite".into(),
            show_hidden_by_default: false,
            remember_window_size: true,
            last_directory: None,
            window_width: 1200,
            window_height: 700,
            color_scheme: 0,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_path();
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(settings) = serde_json::from_str(&data) {
                    return settings;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = settings_path();
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, data);
        }
    }
}

fn settings_path() -> PathBuf {
    super::config_dir().join("settings.json")
}

pub type SharedSettings = Rc<RefCell<Settings>>;

pub fn load_settings() -> SharedSettings {
    super::ensure_config_dir();
    Rc::new(RefCell::new(Settings::load()))
}

pub fn save_settings(settings: &Settings) {
    settings.save();
}
