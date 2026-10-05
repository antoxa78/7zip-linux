use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::gio;

use crate::panels::SharedPanel;

/// Format names as shown in the dropdown; each is also the file extension.
const FORMATS: [&str; 7] = ["7z", "zip", "tar", "tar.gz", "tar.bz2", "tar.xz", "tar.zst"];

/// Replaces a known archive extension on `name` with `.{format}`, or appends it
/// if the name has none (so changing the format never yields e.g. a zip named .7z).
fn with_format_extension(name: &str, format: &str) -> String {
    let lower = name.to_lowercase();
    // Longest first so ".tar.gz" wins over ".gz"-less ".tar".
    let mut exts: Vec<&str> = FORMATS.to_vec();
    exts.sort_by_key(|e| std::cmp::Reverse(e.len()));
    for ext in exts {
        let suffix = format!(".{}", ext);
        if lower.ends_with(&suffix) && name.len() > suffix.len() {
            return format!("{}.{}", &name[..name.len() - suffix.len()], format);
        }
    }
    format!("{}.{}", name, format)
}

pub fn show(state: &SharedPanel, paths: &[PathBuf], password_protect: bool) {
    // When browsing inside an archive, default to the folder that contains it
    // (the virtual "x.7z [archive]/..." path is not a real folder).
    let current = {
        let cur = state.borrow().current_path.clone();
        match crate::archive::browse::parse_archive_path(&cur) {
            Some((archive, _)) => archive.parent().map(|p| p.to_path_buf()).unwrap_or(cur),
            None => cur,
        }
    };

    let dialog = adw::Dialog::builder()
        .title(if password_protect { "Password-Protected Archive" } else { "Create Archive" })
        .content_width(460)
        .build();

    // GNOME dialog pattern: Cancel on the left, the action on the right of the header.
    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    let cancel_button = gtk::Button::with_label("Cancel");
    let build_button = gtk::Button::with_label("Create");
    build_button.add_css_class("suggested-action");
    header.pack_start(&cancel_button);
    header.pack_end(&build_button);
    toolbar_view.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let what = if paths.len() == 1 {
        paths[0].file_name().map(|n| format!("\u{201c}{}\u{201d}", n.to_string_lossy())).unwrap_or_default()
    } else {
        format!("{} items", paths.len())
    };

    // --- Archive ---
    let now = chrono::Local::now();
    let timestamp = now.format("%Y-%m-%d_%H-%M-%S").to_string();
    let default_name = if paths.len() == 1 {
        let first_name = paths[0].file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("archive");
        format!("{}_{}.7z", first_name, timestamp)
    } else {
        format!("archive_{}.7z", timestamp)
    };
    let archive_group = adw::PreferencesGroup::builder()
        .title("Archive")
        .description(format!("Packs {}", what))
        .build();
    let name_entry = adw::EntryRow::builder()
        .title("File name")
        .text(&default_name)
        .build();
    archive_group.add(&name_entry);

    let fmt_combo = adw::ComboRow::builder()
        .title("Format")
        .model(&gtk::StringList::new(&FORMATS))
        .build();
    archive_group.add(&fmt_combo);

    let level_combo = adw::ComboRow::builder()
        .title("Compression")
        .subtitle("Higher levels are smaller but slower")
        .model(&gtk::StringList::new(&[
            "Store (no compression)",
            "Fastest",
            "Fast",
            "Normal",
            "Maximum",
            "Ultra",
        ]))
        .selected(3)
        .build();
    archive_group.add(&level_combo);
    content.append(&archive_group);

    // --- Location ---
    let out_dir = std::rc::Rc::new(std::cell::RefCell::new(current.clone()));
    let location_group = adw::PreferencesGroup::new();
    let out_row = adw::ActionRow::builder()
        .title("Save in")
        .subtitle(crate::panels::display_path(&current))
        .activatable(true)
        .build();
    out_row.add_css_class("property");
    let out_icon = gtk::Image::from_icon_name("folder-open-symbolic");
    out_row.add_suffix(&out_icon);
    location_group.add(&out_row);
    content.append(&location_group);

    {
        let out_dir = out_dir.clone();
        out_row.connect_activated(move |row| {
            let chooser = gtk::FileDialog::builder()
                .title("Choose Where to Save the Archive")
                .accept_label("Select")
                .initial_folder(&gio::File::for_path(&*out_dir.borrow()))
                .build();
            let out_dir = out_dir.clone();
            let row = row.clone();
            chooser.select_folder(crate::utils::parent_window().as_ref(), None::<&gio::Cancellable>, move |result| {
                if let Ok(folder) = result {
                    if let Some(path) = folder.path() {
                        row.set_subtitle(&crate::panels::display_path(&path));
                        *out_dir.borrow_mut() = path;
                    }
                }
            });
        });
    }

    // --- Encryption ---
    let enc_group = adw::PreferencesGroup::builder()
        .title("Encryption")
        .description(if password_protect {
            "Anyone opening the archive will need this password"
        } else {
            "Optional. Leave the password empty for no encryption."
        })
        .build();
    let password_entry = adw::PasswordEntryRow::builder()
        .title("Password")
        .build();
    enc_group.add(&password_entry);

    let encrypt_names_check = adw::SwitchRow::builder()
        .title("Encrypt file names")
        .subtitle("Hide the list of files until the password is entered")
        .active(password_protect)
        .build();
    enc_group.add(&encrypt_names_check);
    content.append(&enc_group);

    {
        let chk = encrypt_names_check.clone();
        let pw_row = password_entry.clone();
        let group = enc_group.clone();
        let name_ref = name_entry.clone();
        fmt_combo.connect_selected_notify(move |c| {
            let format = FORMATS.get(c.selected() as usize).copied().unwrap_or("7z");
            let current_name = name_ref.text().to_string();
            if !current_name.is_empty() {
                name_ref.set_text(&with_format_extension(&current_name, format));
            }
            // Only 7z can hide file names; tar can't be encrypted at all.
            let is_7z = format == "7z";
            let is_tar = format.starts_with("tar");
            chk.set_sensitive(is_7z);
            if !is_7z {
                chk.set_active(false);
            }
            pw_row.set_sensitive(!is_tar);
            group.set_description(Some(if is_tar {
                "tar archives can\u{2019}t be encrypted. Choose 7z or zip to set a password."
            } else if !is_7z {
                "zip encrypts file contents only; file names stay visible."
            } else {
                "Optional. Leave the password empty for no encryption."
            }));
        });
    }

    let clamp = adw::Clamp::builder().maximum_size(520).child(&content).build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&clamp)
        .build();
    toolbar_view.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar_view));
    dialog.set_default_widget(Some(&build_button));

    let dialog_ref = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        dialog_ref.close();
    });

    let state = state.clone();
    let paths: Vec<PathBuf> = paths.to_vec();
    let dialog_for_build = dialog.clone();
    let password_entry_focus = password_entry.clone();
    build_button.connect_clicked(move |_| {
        let fmt_idx = fmt_combo.selected();
        let format = FORMATS.get(fmt_idx as usize).copied().unwrap_or("7z");
        let typed = name_entry.text().trim().to_string();
        if typed.is_empty() {
            name_entry.grab_focus();
            return;
        }
        let name = with_format_extension(&typed, format);
        if name != typed {
            name_entry.set_text(&name);
        }
        let level = level_combo.selected();
        let password = password_entry.text().to_string();
        let encrypt_names = encrypt_names_check.is_active() && format == "7z";
        let output_dir = out_dir.borrow().clone();

        if !password.is_empty() && format.starts_with("tar") {
            let alert = adw::AlertDialog::builder()
                .heading("Encryption Not Supported")
                .body("tar archives cannot be encrypted. Choose the 7z or zip format.")
                .build();
            alert.add_response("ok", "OK");
            alert.present(crate::utils::parent_window().as_ref());
            return;
        }

        if password_protect && password.is_empty() {
            let alert = adw::AlertDialog::builder()
                .heading("Password Required")
                .body("Enter a password to protect the archive.")
                .build();
            alert.add_response("ok", "OK");
            alert.present(crate::utils::parent_window().as_ref());
            password_entry.grab_focus();
            return;
        }

        let out_path = output_dir.join(&name);
        let password_opt = if password.is_empty() { None } else { Some(password) };

        let start_build = |out_path: PathBuf, format: String, level: u32, password_opt: Option<String>, encrypt_names: bool, s: SharedPanel, paths: Vec<PathBuf>, dfb: adw::Dialog| {
            dfb.close();

            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let pause = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

            {
                let sb = s.borrow();
                sb.progress_bar.set_visible(true);
                sb.progress_bar.set_fraction(0.0);
                sb.progress_bar.set_text(Some("0%"));
                sb.status_label.set_label("Creating archive...");
            }

            let (tx, rx) = async_channel::bounded::<u8>(32);
            let s_for_rx = s.clone();
            let rx_handle = glib::spawn_future_local(async move {
                while let Ok(pct) = rx.recv().await {
                    let sb = s_for_rx.borrow();
                    sb.progress_bar.set_fraction(pct as f64 / 100.0);
                    sb.progress_bar.set_text(Some(&format!("{}%", pct)));
                    sb.status_label.set_label(&format!("Creating archive... {}%", pct));
                }
                let sb = s_for_rx.borrow();
                sb.progress_bar.set_visible(false);
                sb.status_label.set_label("");
            });

            glib::spawn_future_local({
                let s = s.clone();
                let paths = paths.clone();
                let format = format.clone();
                let out_path = out_path.clone();
                async move {
                    let options = crate::archive::creator::ArchiveOptions {
                        format,
                        level,
                        method: String::new(),
                        password: password_opt,
                        split_size: None,
                        encrypt_file_names: encrypt_names,
                    };
                    let refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
                    let result = crate::archive::creator::create_archive(&out_path, &refs, &options, Some(tx), Some(cancel), Some(pause)).await;

                    drop(rx_handle);

                    match result {
                        Ok(_) => {
                            crate::panels::load_directory(&s);
                        }
                        Err(e) => {
                            if e == "Cancelled" {
                                return;
                            }
                            let d = adw::AlertDialog::builder()
                                .heading("Create Archive Failed")
                                .body(crate::utils::humanize_error(&e))
                                .build();
                            d.add_response("ok", "OK");
                            d.present(crate::utils::parent_window().as_ref());
                        }
                    }
                }
            });
        };

        let s_build = state.clone();
        let p_build = paths.clone();
        let d_build = dialog_for_build.clone();

        if out_path.exists() {
            let conflict = adw::AlertDialog::builder()
                .heading("File already exists")
                .body(&format!("Do you want to overwrite \"{}\" or choose a different name?", name))
                .build();
            conflict.add_response("cancel", "Cancel");
            conflict.add_response("rename", "Choose Another Name");
            conflict.add_response("overwrite", "Overwrite");
            conflict.set_response_appearance("overwrite", adw::ResponseAppearance::Destructive);
            conflict.set_default_response(Some("rename"));
            let f = format.to_string();
            let pw = password_opt.clone();
            let s = s_build.clone();
            let p = p_build.clone();
            let d = d_build.clone();
            let op = out_path.clone();
            let name_entry_focus = name_entry.clone();
            conflict.connect_response(None, move |_, resp| {
                if resp == "overwrite" {
                    let _ = std::fs::remove_file(&op);
                    start_build(op.clone(), f.clone(), level, pw.clone(), encrypt_names, s.clone(), p.clone(), d.clone());
                } else if resp == "rename" {
                    name_entry_focus.grab_focus();
                }
            });
            conflict.present(crate::utils::parent_window().as_ref());
        } else {
            start_build(out_path, format.to_string(), level, password_opt, encrypt_names, s_build, p_build, d_build);
        }
    });

    dialog.present(crate::utils::parent_window().as_ref());

    if password_protect {
        password_entry_focus.grab_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::with_format_extension;

    #[test]
    fn extension_follows_format() {
        assert_eq!(with_format_extension("a_2026.7z", "zip"), "a_2026.zip");
        assert_eq!(with_format_extension("a.tar.gz", "7z"), "a.7z");
        assert_eq!(with_format_extension("a.zip", "tar.zst"), "a.tar.zst");
        assert_eq!(with_format_extension("a.tar", "tar.xz"), "a.tar.xz");
        assert_eq!(with_format_extension("backup", "7z"), "backup.7z");
        assert_eq!(with_format_extension("photos.ZIP", "zip"), "photos.zip");
        assert_eq!(with_format_extension("v1.2", "zip"), "v1.2.zip");
    }
}
