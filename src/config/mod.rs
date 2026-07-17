pub mod bookmarks;
pub mod settings;

use std::path::PathBuf;

pub const APP_ID: &str = "com.idanplus.sevenzip-linux";
pub const APP_NAME: &str = "7-Zip Linux";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn config_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("7zip-linux")
}

pub fn ensure_config_dir() {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
}
