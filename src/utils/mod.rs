pub mod format;
pub mod fsops;
pub mod icons;


use adw::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static APP_WINDOW: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

/// Sentinel error returned by the archive helpers when 7z needs a (different) password.
pub const NEED_PASSWORD: &str = "__NEED_PASSWORD__";

pub fn set_app_window(window: &impl IsA<gtk::Window>) {
    APP_WINDOW.with(|w| *w.borrow_mut() = Some(window.clone().upcast()));
}

pub fn parent_window() -> Option<gtk::Window> {
    APP_WINDOW.with(|w| w.borrow().clone())
}

/// Turns internal error strings into something a user can read.
pub fn humanize_error(detail: &str) -> String {
    if detail == NEED_PASSWORD {
        "The archive is encrypted and the password is missing or incorrect.".to_string()
    } else if detail.trim().is_empty() {
        "The operation failed (7z gave no details).".to_string()
    } else {
        detail.to_string()
    }
}

/// Creates a fresh, private (0700) directory under the system temp dir.
/// Every call gets its own directory, so concurrent operations never clobber each other.
pub fn unique_temp_dir(tag: &str) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir();
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("sevenzip-gui-{}-{}-{}", tag, std::process::id(), n));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
                }
                return Ok(dir);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

pub fn show_error(title: &str, detail: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading(title)
        .body(humanize_error(detail))
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(parent_window().as_ref());
}

pub fn show_info(title: &str, detail: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading(title)
        .body(detail)
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(parent_window().as_ref());
}
