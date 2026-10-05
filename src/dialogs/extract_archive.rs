use std::path::PathBuf;

use adw::prelude::*;
use gtk::gio;

use crate::panels::SharedPanel;

pub fn show(state: &SharedPanel, archive_path: &PathBuf, initial_password: Option<String>) {
    // Default destination: the current folder — or, when browsing inside an archive,
    // the real folder containing it (the virtual "x.7z [archive]/..." path is not a
    // folder; 7z would create one literally named "x.7z [archive]").
    let current = {
        let cur = state.borrow().current_path.clone();
        match crate::archive::browse::parse_archive_path(&cur) {
            Some((archive, _)) => archive.parent().map(|p| p.to_path_buf()).unwrap_or(cur),
            None => cur,
        }
    };
    let archive_name = archive_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive")
        .to_string();

    let dialog = adw::Dialog::builder()
        .title("Extract Archive")
        .content_width(460)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    let cancel_button = gtk::Button::with_label("Cancel");
    let extract_button = gtk::Button::with_label("Extract");
    extract_button.add_css_class("suggested-action");
    header.pack_start(&cancel_button);
    header.pack_end(&extract_button);
    toolbar_view.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // --- Destination ---
    let dest = std::rc::Rc::new(std::cell::RefCell::new(current.clone()));
    let dest_group = adw::PreferencesGroup::builder()
        .title(&archive_name)
        .build();
    let dest_row = adw::ActionRow::builder()
        .title("Extract to")
        .subtitle(crate::panels::display_path(&current))
        .activatable(true)
        .build();
    dest_row.add_css_class("property");
    dest_row.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    dest_group.add(&dest_row);
    content.append(&dest_group);

    {
        let dest = dest.clone();
        dest_row.connect_activated(move |row| {
            let chooser = gtk::FileDialog::builder()
                .title("Choose Where to Extract")
                .accept_label("Select")
                .initial_folder(&gio::File::for_path(&*dest.borrow()))
                .build();
            let dest = dest.clone();
            let row = row.clone();
            chooser.select_folder(crate::utils::parent_window().as_ref(), None::<&gio::Cancellable>, move |result| {
                if let Ok(folder) = result {
                    if let Some(path) = folder.path() {
                        row.set_subtitle(&crate::panels::display_path(&path));
                        *dest.borrow_mut() = path;
                    }
                }
            });
        });
    }

    // --- Options ---
    let options_group = adw::PreferencesGroup::new();
    let path_combo = adw::ComboRow::builder()
        .title("Folders")
        .model(&gtk::StringList::new(&["Keep folder structure", "Put all files in one folder"]))
        .build();
    options_group.add(&path_combo);
    let ow_combo = adw::ComboRow::builder()
        .title("If a file exists")
        .model(&gtk::StringList::new(&["Replace it", "Skip it", "Keep both"]))
        .build();
    options_group.add(&ow_combo);
    content.append(&options_group);

    // --- Password ---
    let pw_group = adw::PreferencesGroup::builder()
        .description("Only needed for encrypted archives")
        .build();
    let password_entry = adw::PasswordEntryRow::builder()
        .title("Password")
        .build();
    if let Some(pw) = initial_password {
        password_entry.set_text(&pw);
    }
    pw_group.add(&password_entry);
    content.append(&pw_group);

    let clamp = adw::Clamp::builder().maximum_size(520).child(&content).build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&clamp)
        .build();
    toolbar_view.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar_view));
    dialog.set_default_widget(Some(&extract_button));

    let dialog_ref = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        dialog_ref.close();
    });

    let s = state.clone();
    let archive_clone = archive_path.clone();
    let dialog_for_extract = dialog.clone();
    extract_button.connect_clicked(move |_| {
        let dest = dest.borrow().clone();
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
                        .body(crate::utils::humanize_error(&e))
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
