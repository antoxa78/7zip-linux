use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::gio;

use crate::panels::SharedPanel;

pub fn show(state: &SharedPanel, paths: &[PathBuf]) {
    let current = { state.borrow().current_path.clone() };

    let dialog = adw::Dialog::builder()
        .title("Create Archive")
        .content_width(500)
        .content_height(420)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Archive name
    let name_label = gtk::Label::builder()
        .label("Archive name:")
        .xalign(0.0)
        .build();
    content.append(&name_label);

    let default_name = if paths.len() == 1 {
        let first_name = paths[0].file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("archive");
        format!("{}.7z", first_name)
    } else {
        "archive.7z".to_string()
    };
    let name_entry = gtk::Entry::builder()
        .text(&default_name)
        .hexpand(true)
        .build();
    content.append(&name_entry);

    // Format
    let fmt_label = gtk::Label::builder()
        .label("Format:")
        .xalign(0.0)
        .build();
    content.append(&fmt_label);

    let fmt_combo = gtk::DropDown::from_strings(&["7z", "zip", "tar", "tar.gz", "tar.bz2", "tar.xz", "tar.zst"]);
    fmt_combo.set_selected(0);
    content.append(&fmt_combo);

    // Compression level
    let level_label = gtk::Label::builder()
        .label("Compression level:")
        .xalign(0.0)
        .build();
    content.append(&level_label);

    let level_combo = gtk::DropDown::from_strings(&[
        "Store (no compression)",
        "Fastest",
        "Fast",
        "Normal",
        "Maximum",
        "Ultra",
    ]);
    level_combo.set_selected(3);
    content.append(&level_combo);

    // Encryption section
    let enc_label = gtk::Label::builder()
        .label("Encryption")
        .xalign(0.0)
        .build();
    enc_label.add_css_class("heading");
    content.append(&enc_label);

    let enc_box = gtk::Box::new(gtk::Orientation::Vertical, 6);

    let password_label = gtk::Label::builder()
        .label("Password:")
        .xalign(0.0)
        .build();
    let password_entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Optional")
        .hexpand(true)
        .build();
    enc_box.append(&password_label);
    enc_box.append(&password_entry);

    let encrypt_names_check = gtk::CheckButton::builder()
        .label("Encrypt file names")
        .margin_top(4)
        .build();
    enc_box.append(&encrypt_names_check);
    content.append(&enc_box);

    // Output path
    let out_label = gtk::Label::builder()
        .label("Output folder:")
        .xalign(0.0)
        .build();
    content.append(&out_label);

    let out_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let out_entry = gtk::Entry::builder()
        .text(&*current.to_string_lossy())
        .hexpand(true)
        .sensitive(false)
        .build();
    let out_button = gtk::Button::from_icon_name("folder-open-symbolic");
    out_button.set_tooltip_text(Some("Browse output folder"));
    out_row.append(&out_entry);
    out_row.append(&out_button);
    content.append(&out_row);

    {
        let out_entry_ref = out_entry.clone();
        out_button.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Select Output Folder")
                .build();
            let out_ref = out_entry_ref.clone();
            dialog.select_folder(None::<&gtk::Window>, None::<&gio::Cancellable>, move |result| {
                if let Ok(folder) = result {
                    if let Some(path) = folder.path() {
                        out_ref.set_text(&*path.to_string_lossy());
                    }
                }
            });
        });
    }

    toolbar_view.set_content(Some(&content));

    let cancel_button = gtk::Button::builder()
        .label("Cancel")
        .build();

    let build_button = gtk::Button::builder()
        .label("Build")
        .build();
    build_button.add_css_class("suggested-action");

    let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom_bar.set_margin_top(8);
    bottom_bar.set_margin_bottom(8);
    bottom_bar.set_margin_start(12);
    bottom_bar.set_margin_end(12);
    bottom_bar.append(&cancel_button);
    bottom_bar.set_halign(gtk::Align::End);
    bottom_bar.set_hexpand(true);
    bottom_bar.append(&build_button);
    toolbar_view.add_bottom_bar(&bottom_bar);

    dialog.set_child(Some(&toolbar_view));

    let dialog_ref = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        dialog_ref.close();
    });

    let state = state.clone();
    let paths: Vec<PathBuf> = paths.to_vec();
    let dialog_for_build = dialog.clone();
    build_button.connect_clicked(move |_| {
        let name = name_entry.text().to_string();
        let fmt_idx = fmt_combo.selected();
        let format = match fmt_idx {
            0 => "7z",
            1 => "zip",
            2 => "tar",
            3 => "tar.gz",
            4 => "tar.bz2",
            5 => "tar.xz",
            6 => "tar.zst",
            _ => "7z",
        };
        let level = level_combo.selected();
        let password = password_entry.text().to_string();
        let encrypt_names = encrypt_names_check.is_active();
        let output_dir = PathBuf::from(out_entry.text().to_string());

        let out_path = output_dir.join(&name);
        let password_opt = if password.is_empty() { None } else { Some(password) };

        let start_build = |out_path: PathBuf, format: String, level: u32, password_opt: Option<String>, encrypt_names: bool, s: SharedPanel, paths: Vec<PathBuf>, dfb: adw::Dialog| {
            let progress = crate::dialogs::progress::ProgressDialog::new("Creating archive...");
            let pb = progress.progress_bar.clone();
            let cancel = progress.cancel_flag.clone();
            let pause = progress.pause_flag.clone();
            let bg = progress.is_background.clone();

            let bg_bg = bg.clone();
            progress.background_button.connect_clicked({
                let d = progress.dialog.clone();
                move |_| {
                    bg_bg.store(true, std::sync::atomic::Ordering::Relaxed);
                    d.close();
                }
            });

            progress.cancel_button.connect_clicked({
                let cf = cancel.clone();
                let d = progress.dialog.clone();
                move |_| {
                    let confirm = adw::AlertDialog::builder()
                        .heading("Cancel")
                        .body("Really cancel?")
                        .build();
                    confirm.add_response("no", "No");
                    confirm.add_response("yes", "Yes");
                    confirm.set_response_appearance("yes", adw::ResponseAppearance::Destructive);
                    let cf = cf.clone();
                    let d = d.clone();
                    confirm.connect_response(None, move |_, resp| {
                        if resp == "yes" {
                            cf.store(true, std::sync::atomic::Ordering::Relaxed);
                            d.close();
                        }
                    });
                    confirm.present(crate::utils::parent_window().as_ref());
                }
            });

            progress.present();

            let (tx, rx) = async_channel::bounded::<u8>(32);
            let s_for_rx = s.clone();
            let rx_handle = glib::spawn_future_local(async move {
                while let Ok(pct) = rx.recv().await {
                    if bg.load(std::sync::atomic::Ordering::Relaxed) {
                        let sb = s_for_rx.borrow_mut();
                        sb.progress_bar.set_fraction(pct as f64 / 100.0);
                        sb.progress_bar.set_text(Some(&format!("{}%", pct)));
                        sb.progress_bar.set_visible(true);
                        sb.status_label.set_label(&format!("Creating archive... {}%", pct));
                    } else {
                        pb.set_fraction(pct as f64 / 100.0);
                        pb.set_text(Some(&format!("{}%", pct)));
                    }
                }
                let sb = s_for_rx.borrow_mut();
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

                    progress.close();
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
                                .body(&e)
                                .build();
                            d.add_response("ok", "OK");
                            d.present(crate::utils::parent_window().as_ref());
                        }
                    }
                }
            });

            dfb.close();
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
}
