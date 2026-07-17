use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub path: String,
}

impl Bookmark {
    pub fn new(name: &str, path: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
        }
    }

    pub fn path_buf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

pub fn load_bookmarks() -> Vec<Bookmark> {
    let path = bookmarks_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(bookmarks) = serde_json::from_str(&data) {
                return bookmarks;
            }
        }
    }
    default_bookmarks()
}

pub fn save_bookmarks(bookmarks: &[Bookmark]) {
    super::ensure_config_dir();
    let path = bookmarks_path();
    if let Ok(data) = serde_json::to_string_pretty(bookmarks) {
        let _ = std::fs::write(&path, data);
    }
}

pub fn add_bookmark(name: &str, path: &str) -> Vec<Bookmark> {
    let mut bookmarks = load_bookmarks();
    if !bookmarks.iter().any(|b| b.path == path) {
        bookmarks.push(Bookmark::new(name, path));
        save_bookmarks(&bookmarks);
    }
    bookmarks
}

pub fn remove_bookmark(path: &str) -> Vec<Bookmark> {
    let mut bookmarks = load_bookmarks();
    bookmarks.retain(|b| b.path != path);
    save_bookmarks(&bookmarks);
    bookmarks
}

fn bookmarks_path() -> PathBuf {
    super::config_dir().join("bookmarks.json")
}

fn default_bookmarks() -> Vec<Bookmark> {
    let mut bookmarks = Vec::new();

    if let Some(home) = dirs::home_dir() {
        bookmarks.push(Bookmark::new("Home", &home.to_string_lossy()));

        for (name, sub) in &[
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Music", "Music"),
            ("Pictures", "Pictures"),
            ("Videos", "Videos"),
        ] {
            let path = home.join(sub);
            if path.exists() {
                bookmarks.push(Bookmark::new(name, &path.to_string_lossy()));
            }
        }
    }

    bookmarks.push(Bookmark::new("Root", "/"));

    save_bookmarks(&bookmarks);
    bookmarks
}
