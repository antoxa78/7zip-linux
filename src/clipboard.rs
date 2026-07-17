use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Default)]
pub struct ClipboardData {
    pub paths: Vec<PathBuf>,
    pub is_cut: bool,
}

static CLIPBOARD: Mutex<ClipboardData> = Mutex::new(ClipboardData {
    paths: Vec::new(),
    is_cut: false,
});

pub fn set(data: ClipboardData) {
    *CLIPBOARD.lock().unwrap() = data;
}

pub fn get() -> ClipboardData {
    CLIPBOARD.lock().unwrap().clone()
}
