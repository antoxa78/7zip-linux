use std::path::PathBuf;

use adw::prelude::*;
use gtk::gio;

use crate::panels::SharedPanel;

pub fn show(state: &SharedPanel, archive_path: &PathBuf, initial_password: Option<String>) {
    let current = { state.borrow().current_path.clone() };
    let archive_name = archive_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive")
        .to_string();

    let dialog = adw::Dialog::builder()
        .title("Extract Archive")
        .content_width(450)
        .content_height(350)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let info_label = gtk::Label::builder()
        .label(&format!("Archive: {}", archive_name))
        .xalign(0.0)
        .wrap(true)
        .build();
    content.append(&info_label);

    let dest_label = gtk::Label::builder()
        .label("Extract to:")
        .xalign(0.0)
        .build();
    content.append(&dest_label);

    let dest_entry = gtk::Entry::builder()
        .text(&*current.to_string_lossy())
        .hexpand(true)
        .sensitive(false)
        .build();
    dest_entry.add_css_class("flat");
    let browse_button = gtk::Button::from_icon_name("folder-open-symbolic");
    browse_button.set_tooltip_text(Some("Browse destination folder"));
    let dest_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    dest_row.append(&dest_entry);
    dest_row.append(&browse_button);
    content.append(&dest_row);

    {
        let dest_ref = dest_entry.clone();
        browse_button.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Select Destination Folder")
                .build();
            let dest_ref = dest_ref.clone();
            dialog.select_folder(None::<&gtk::Window>, None::<&gio::Cancellable>, move |result| {
                if let Ok(folder) = result {
                    if let Some(path) = folder.path() {
                        dest_ref.set_text(&*path.to_string_lossy());
                    }
                }
            });
        });
    }

    let path_label = gtk::Label::builder()
        .label("Path mode:")
        .xalign(0.0)
        .build();
    content.append(&path_label);

    let path_combo = gtk::DropDown::from_strings(&[
        "Full paths",
        "No paths (flat)",
    ]);
    path_combo.set_selected(0);
    content.append(&path_combo);

    let ow_label = gtk::Label::builder()
        .label("Overwrite mode:")
        .xalign(0.0)
        .build();
    content.append(&ow_label);

    let ow_combo = gtk::DropDown::from_strings(&[
        "Overwrite all",
        "Skip existing",
        "Auto-rename",
    ]);
    ow_combo.set_selected(0);
    content.append(&ow_combo);

    let pw_label = gtk::Label::builder()
        .label("Password (if encrypted):")
        .xalign(0.0)
        .build();
    content.append(&pw_label);

    let password_entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("Optional")
        .hexpand(true)
        .build();
    if let Some(pw) = initial_password {
        password_entry.set_text(&pw);
    }
    content.append(&password_entry);

    toolbar_view.set_content(Some(&content));

    let cancel_button = gtk::Button::builder()
        .label("Cancel")
        .build();

    let extract_button = gtk::Button::builder()
        .label("Extract")
        .build();
    extract_button.add_css_class("suggested-action");

    let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom_bar.set_margin_top(8);
    bottom_bar.set_margin_bottom(8);
    bottom_bar.set_margin_start(12);
    bottom_bar.set_margin_end(12);
    bottom_bar.append(&cancel_button);
    bottom_bar.set_halign(gtk::Align::End);
    bottom_bar.set_hexpand(true);
    bottom_bar.append(&extract_button);
    toolbar_view.add_bottom_bar(&bottom_bar);

    dialog.set_child(Some(&toolbar_view));

    let dialog_ref = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        dialog_ref.close();
    });

    let s = state.clone();
    let archive_clone = archive_path.clone();
    let dialog_for_extract = dialog.clone();
    extract_button.connect_clicked(move |_| {
        let dest = PathBuf::from(dest_entry.text().to_string());
        let full_paths = path_combo.selected() == 0;
        let overwrite = match ow_combo.selected() {
            0 => crate::archive::extractor::OverwriteMode::Overwrite,
            1 => crate::archive::extractor::OverwriteMode::SkipExisting,
            2 => crate::archive::extractor::OverwriteMode::AutoRename,
            _ => crate::archive::extractor::OverwriteMode::Overwrite,
        };
        let password = password_entry.text().to_string();
        let password_opt = if password.is_empty() { None } else { Some(password) };

        let progress = crate::dialogs::progress::ProgressDialog::new("Extracting archive...");
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
                    sb.status_label.set_label(&format!("Extracting archive... {}%", pct));
                } else {
                    pb.set_fraction(pct as f64 / 100.0);
                    pb.set_text(Some(&format!("{}%", pct)));
                }
            }
            let sb = s_for_rx.borrow_mut();
            sb.progress_bar.set_visible(false);
            sb.status_label.set_label("");
        });

        let s2 = s.clone();
        let archive = archive_clone.clone();
        let dest_clone = dest.clone();

        glib::spawn_future_local(async move {
            let options = crate::archive::extractor::ExtractOptions {
                output_dir: dest_clone,
                full_paths,
                overwrite,
                password: password_opt,
            };
            let result = crate::archive::extractor::extract_archive(&archive, &options, Some(tx), Some(cancel), Some(pause)).await;

            progress.close();
            drop(rx_handle);

            match result {
                Ok(_) => {
                    crate::panels::load_directory(&s2);
                }
                Err(e) => {
                    if e == "Cancelled" {
                        return;
                    }
                    let dialog = adw::AlertDialog::builder()
                        .heading("Extract Failed")
                        .body(&e)
                        .build();
                    dialog.add_response("ok", "OK");
                    dialog.present(crate::utils::parent_window().as_ref());
                }
            }
        });

        dialog_for_extract.close();
    });

    dialog.present(crate::utils::parent_window().as_ref());
}
