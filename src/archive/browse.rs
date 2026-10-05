use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use crate::panels::SharedPanel;
use crate::utils::NEED_PASSWORD;

/// Opens an archive for browsing and pushes it onto the navigation history.
pub fn open_archive(state: &SharedPanel, archive_path: &Path, archive_name: &str) {
    open_archive_impl(state, archive_path, archive_name, true);
}

/// Re-reads an archive whose virtual path is already the current location
/// (e.g. after Back/Forward into a different archive) without touching history.
pub fn reopen_archive(state: &SharedPanel, archive_path: &Path) {
    let name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    open_archive_impl(state, archive_path, &name, false);
}

fn open_archive_impl(state: &SharedPanel, archive_path: &Path, archive_name: &str, push_history: bool) {
    let archive_path = archive_path.to_path_buf();
    let archive_name = archive_name.to_string();
    let s = state.clone();

    let virtual_path = format!("{} [archive]", archive_path.display());

    {
        let mut sb = state.borrow_mut();
        sb.raw_store.remove_all();
        sb.path_entry.set_text(&format!("Reading {}...", archive_name));
        sb.status_label.set_label("Reading archive...");
        sb.progress_bar.set_visible(true);
        if let Some(old) = sb.pulse_source.take() {
            old.remove();
        }
        let pb = sb.progress_bar.clone();
        let source = glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            pb.pulse();
            glib::ControlFlow::Continue
        });
        sb.pulse_source = Some(source);
    }

    glib::spawn_future_local(async move {
        // Start with a password we already know for this archive, if any.
        let mut password = s.borrow().archive_passwords.get(&archive_path).cloned();
        loop {
            match super::lister::list_archive_with_password(&archive_path, password.as_deref()).await {
                Ok(entries) => {
                    populate_archive(&s, &archive_path, &virtual_path, entries, password, push_history);
                    return;
                }
                Err(e) if e == NEED_PASSWORD => {
                    let wrong = password.is_some();
                    if wrong {
                        s.borrow_mut().archive_passwords.remove(&archive_path);
                    }
                    match prompt_for_password_retry(&archive_name, wrong).await {
                        Some(pw) => password = Some(pw),
                        None => {
                            abort_open(&s, &archive_path, push_history, None);
                            return;
                        }
                    }
                }
                Err(e) => {
                    abort_open(&s, &archive_path, push_history, Some(&e));
                    return;
                }
            }
        }
    });
}

fn stop_pulse(state: &SharedPanel) {
    let mut sb = state.borrow_mut();
    if let Some(src) = sb.pulse_source.take() {
        src.remove();
    }
    sb.progress_bar.set_visible(false);
}

/// Called when opening was cancelled or failed: go back to something sensible.
fn abort_open(state: &SharedPanel, archive_path: &Path, push_history: bool, error: Option<&str>) {
    stop_pulse(state);
    let showing_this_archive = parse_archive_path(&state.borrow().current_path)
        .map(|(a, _)| a == archive_path)
        .unwrap_or(false);
    if !push_history || showing_this_archive {
        // We are "inside" the archive we could not read; leave to its folder.
        match archive_path.parent() {
            Some(parent) => crate::panels::navigate_to(state, parent),
            None => crate::panels::load_directory(state),
        }
    } else {
        // Restore the listing we came from.
        crate::panels::load_directory(state);
    }
    if let Some(e) = error {
        show_error_dialog(e);
    }
}

fn populate_archive(
    state: &SharedPanel,
    archive_path: &Path,
    virtual_path: &str,
    entries: Vec<super::lister::ArchiveEntry>,
    password: Option<String>,
    push_history: bool,
) {
    stop_pulse(state);
    {
        let mut s = state.borrow_mut();
        s.archive_entries = entries;
        s.archive_virtual_root = virtual_path.to_string();
        // The password belongs to *this* archive only (None if it is not encrypted),
        // so a password from a previously opened archive is never reused by accident.
        match &password {
            Some(pw) => {
                s.archive_passwords.insert(archive_path.to_path_buf(), pw.clone());
            }
            None => {
                s.archive_passwords.remove(archive_path);
            }
        }
        s.current_password = password;
        if push_history {
            s.current_path = PathBuf::from(virtual_path);
            let idx = s.history_index;
            let cp = s.current_path.clone();
            s.history.truncate(idx + 1);
            s.history.push(cp);
            s.history_index = s.history.len() - 1;
        }
    }
    crate::panels::load_directory(state);
}

fn show_error_dialog(e: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading("Cannot Open Archive")
        .body(crate::utils::humanize_error(e))
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(crate::utils::parent_window().as_ref());
}

pub async fn prompt_for_password(archive_name: &str) -> Option<String> {
    prompt_for_password_retry(archive_name, false).await
}

/// Asks for a password; `wrong` changes the text to say the previous one was rejected.
pub async fn prompt_for_password_retry(archive_name: &str, wrong: bool) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
    let tx = Rc::new(RefCell::new(Some(tx)));

    let dialog = adw::AlertDialog::builder()
        .heading(if wrong { "Wrong Password" } else { "Password Required" })
        .body(if wrong {
            format!("The password for \"{}\" is incorrect. Try again:", archive_name)
        } else {
            format!("\"{}\" is password-protected. Enter password:", archive_name)
        })
        .build();

    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Password")
        .hexpand(true)
        .build();
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("open", "Open");
    dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("open"));
    dialog.set_close_response("cancel");
    entry.set_activates_default(true);

    let tx1 = tx.clone();
    let entry_ref = entry.clone();
    dialog.connect_response(None, move |_, response| {
        let result = if response == "open" {
            Some(entry_ref.text().to_string())
        } else {
            None
        };
        if let Some(tx) = tx1.borrow_mut().take() {
            let _ = tx.send(result);
        }
    });

    dialog.present(crate::utils::parent_window().as_ref());

    let entry_focus = entry.clone();
    glib::idle_add_local_once(move || {
        entry_focus.grab_focus();
    });

    rx.await.unwrap_or(None)
}

pub fn try_open_archive(state: &SharedPanel, path: &Path) {
    eprintln!("[TRY] try_open_archive: {}", path.display());
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        open_archive(state, path, name);
    } else {
        eprintln!("[TRY] no file_name in path!");
    }
}

pub fn is_archive_path(path: &Path) -> bool {
    path.to_string_lossy().contains(" [archive]")
}

pub fn parse_archive_path(path: &Path) -> Option<(PathBuf, String)> {
    let s = path.to_string_lossy();
    let marker = " [archive]/";
    if let Some(idx) = s.find(marker) {
        let archive = PathBuf::from(&s[..idx]);
        let internal = s[idx + marker.len()..].to_string();
        if !internal.is_empty() {
            return Some((archive, internal));
        }
    }
    let marker = " [archive]";
    if let Some(idx) = s.rfind(marker) {
        let archive = PathBuf::from(&s[..idx]);
        return Some((archive, String::new()));
    }
    None
}
