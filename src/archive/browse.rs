use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use crate::models::FileItem;
use crate::panels::SharedPanel;

pub fn open_archive(state: &SharedPanel, archive_path: &Path, archive_name: &str) {
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
        let pb = sb.progress_bar.clone();
        let source = glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            pb.pulse();
            glib::ControlFlow::Continue
        });
        sb.pulse_source = Some(source);
    }

    glib::spawn_future_local(async move {
        eprintln!("[OPEN] open_archive: path={}", archive_path.display());
        match super::lister::list_archive_with_password(&archive_path, None).await {
            Ok(entries) => {
                eprintln!("[OPEN] listing succeeded without password, {} entries", entries.len());
                populate_archive(&s, &virtual_path, &archive_name, &entries);
            }
            Err(e) if e == "__NEED_PASSWORD__" => {
                eprintln!("[OPEN] password required, prompting...");
                match prompt_for_password(&archive_name).await {
                    Some(password) => {
                        eprintln!("[OPEN] password entered, retrying listing...");
                        match super::lister::list_archive_with_password(
                            &archive_path,
                            Some(&password),
                        )
                        .await
                        {
                            Ok(entries) => {
                                eprintln!("[OPEN] listing succeeded with password, {} entries", entries.len());
                                s.borrow_mut().current_password = Some(password);
                                populate_archive(&s, &virtual_path, &archive_name, &entries);
                            }
                            Err(e) => {
                                let mut sb = s.borrow_mut();
                                if let Some(src) = sb.pulse_source.take() {
                                    src.remove();
                                }
                                sb.progress_bar.set_visible(false);
                                sb.path_entry.set_text(&archive_path.to_string_lossy());
                                sb.status_label.set_label("Cannot open archive");
                                drop(sb);
                                show_error_dialog(&e);
                            }
                        }
                    }
                    None => {
                        let mut sb = s.borrow_mut();
                        if let Some(src) = sb.pulse_source.take() {
                            src.remove();
                        }
                        sb.progress_bar.set_visible(false);
                        sb.path_entry.set_text(&archive_path.to_string_lossy());
                        sb.status_label.set_label("");
                    }
                }
            }
            Err(e) => {
                let mut sb = s.borrow_mut();
                if let Some(src) = sb.pulse_source.take() {
                    src.remove();
                }
                sb.progress_bar.set_visible(false);
                sb.path_entry.set_text(&archive_path.to_string_lossy());
                sb.status_label.set_label("Cannot open archive");
                drop(sb);
                show_error_dialog(&e);
            }
        }
    });
}

fn populate_archive(
    state: &SharedPanel,
    virtual_path: &str,
    archive_name: &str,
    entries: &[super::lister::ArchiveEntry],
) {
    let mut s = state.borrow_mut();
    if let Some(src) = s.pulse_source.take() {
        src.remove();
    }
    s.progress_bar.set_visible(false);
    s.raw_store.remove_all();
    s.current_path = PathBuf::from(virtual_path);
    s.archive_entries = entries.to_vec();
    s.archive_virtual_root = virtual_path.to_string();
    let idx = s.history_index;
    let cp = s.current_path.clone();
    s.history.truncate(idx + 1);
    s.history.push(cp);
    s.history_index = s.history.len() - 1;
    s.path_entry
        .set_text(&format!("{}:/", archive_name));

    let parent = FileItem::new("..", "..", true, 0, 0, 0, 0, "Directory");
    s.raw_store.append(&parent);

    let mut count = 0usize;
    for entry in entries {
        let entry_name = entry.name.trim_end_matches('/');
        if entry_name.contains('/') {
            continue;
        }
        let display_name = entry_name.rsplit('/').next().unwrap_or(entry_name).to_string();
        let full_virtual = format!("{}/{}", virtual_path, entry.name);
        let file_type = if entry.is_dir {
            String::from("Directory")
        } else {
            display_name
                .rsplit('.')
                .next()
                .map(|e| format!(".{}", e))
                .unwrap_or_default()
        };
        let item = FileItem::new(
            &display_name,
            &full_virtual,
            entry.is_dir,
            entry.size,
            0,
            0,
            0,
            &file_type,
        );
        s.raw_store.append(&item);
        count += 1;
    }

    s.status_label
        .set_label(&format!("{} items (in archive)", count));
}

fn show_error_dialog(e: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading("Cannot Open Archive")
        .body(e)
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(crate::utils::parent_window().as_ref());
}

pub async fn prompt_for_password(archive_name: &str) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
    let tx = Rc::new(RefCell::new(Some(tx)));

    let dialog = adw::AlertDialog::builder()
        .heading("Password Required")
        .body(format!("\"{}\" is password-protected. Enter password:", archive_name))
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
