use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Default)]
pub struct ClipboardData {
    pub paths: Vec<PathBuf>,
    pub is_cut: bool,
    /// Password of the archive the paths came from (if they are inside an archive).
    pub password: Option<String>,
}

static CLIPBOARD: Mutex<ClipboardData> = Mutex::new(ClipboardData {
    paths: Vec::new(),
    is_cut: false,
    password: None,
});

pub fn set(data: ClipboardData) {
    *CLIPBOARD.lock().unwrap() = data;
}

pub fn get() -> ClipboardData {
    CLIPBOARD.lock().unwrap().clone()
}

pub fn clear() {
    set(ClipboardData::default());
}
