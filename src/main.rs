mod archive;
mod clipboard;
mod config;
mod dialogs;
mod models;
mod operations;
mod panels;
mod utils;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicPtr, Ordering};

use adw::prelude::*;
use gtk::{gdk, gio};

use crate::panels::SharedPanel;

static PANEL_STATE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

fn store_panel_state(state: &SharedPanel) {
    let ptr = Box::into_raw(Box::new(state.clone()));
    PANEL_STATE.store(ptr as *mut std::ffi::c_void, Ordering::Relaxed);
}

fn get_panel_state() -> Option<SharedPanel> {
    let ptr = PANEL_STATE.load(Ordering::Relaxed) as *const SharedPanel;
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { (*ptr).clone() })
    }
}

fn main() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let app = adw::Application::builder()
        .application_id(config::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_activate(build_ui);

    app.connect_open(move |app, files, _hints| {
        // When launched with files, activate may not fire — ensure UI exists
        let state = match get_panel_state() {
            Some(s) => s,
            None => {
                build_ui(app);
                get_panel_state().expect("build_ui must set PANEL_STATE")
            }
        };
        if let Some(file) = files.first() {
            if let Some(path) = file.path() {
                if path.is_dir() {
                    crate::panels::navigate_to(&state, &path);
                } else if crate::archive::browse::parse_archive_path(&path).is_some()
                    || crate::panels::is_archive_file_check(&path)
                {
                    crate::archive::browse::try_open_archive(&state, &path);
                } else {
                    let uri = format!("file://{}", path.display());
                    let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
                }
            }
        }
    });

    app.run();
}

fn build_ui(app: &adw::Application) {
    let settings = config::settings::load_settings();

    // Clean up stale temp dirs from previous sessions
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("sevenzip-gui-open"));
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("sevenzip-gui-list"));

    // Apply saved color scheme
    {
        let style_manager = adw::StyleManager::default();
        let color_scheme = match settings.borrow().color_scheme {
            1 => adw::ColorScheme::ForceLight,
            2 => adw::ColorScheme::ForceDark,
            _ => adw::ColorScheme::Default,
        };
        style_manager.set_color_scheme(color_scheme);
    }

    // Register project data dir so GTK finds our app icon
    if let Some(display) = gdk::Display::default() {
        let icon_theme = gtk::IconTheme::for_display(&display);
        let icon_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/icons");
        icon_theme.add_search_path(&icon_path);
    }

    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".toolbar { background: @card_bg_color; border-bottom: 1px solid @borders; padding: 2px; }\n\
         .caption { font-size: 0.75rem; opacity: 0.8; }\n\
         .dim-label { opacity: 0.65; }\n\
         .status-label { font-size: 0.8rem; font-weight: 600; }\n\
         .toolbar separator { margin-top: 6px; margin-bottom: 6px; }\n\
         .nav-bar button { min-width: 32px; min-height: 32px; padding: 2px; }\n"
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(config::APP_NAME)
        .default_width(settings.borrow().window_width)
        .default_height(settings.borrow().window_height)
        .build();

    crate::utils::set_app_window(&window);

    let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

    // --- Header Bar (title only) ---
    let header = adw::HeaderBar::new();
    let title_label = gtk::Label::new(Some(config::APP_NAME));
    header.set_title_widget(Some(&title_label));

    let menu = gio::Menu::new();
    let view_section = gio::Menu::new();
    view_section.append(Some("Show Hidden Files"), Some("win.toggle-hidden"));
    menu.append_section(None, &view_section);

    let assoc_submenu = gio::Menu::new();
    assoc_submenu.append(Some("Register MIME Types"), Some("win.register-assoc"));
    assoc_submenu.append(Some("Unregister MIME Types"), Some("win.unregister-assoc"));
    assoc_submenu.append(Some("Install File Manager Scripts"), Some("win.install-fm-scripts"));
    assoc_submenu.append(Some("Uninstall File Manager Scripts"), Some("win.uninstall-fm-scripts"));

    let settings_section = gio::Menu::new();
    settings_section.append_submenu(Some("File Associations"), &assoc_submenu);
    menu.append_section(None, &settings_section);

    let help_section = gio::Menu::new();
    help_section.append(Some("About"), Some("win.about"));
    menu.append_section(None, &help_section);

    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Menu")
        .build();
    header.pack_end(&menu_button);

    content_box.append(&header);

    // --- Toolbar row (below titlebar) ---
    let toolbar_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    toolbar_row.set_margin_top(4);
    toolbar_row.set_margin_bottom(4);
    toolbar_row.set_margin_start(4);
    toolbar_row.set_margin_end(4);
    toolbar_row.set_halign(gtk::Align::Start);
    toolbar_row.set_hexpand(true);
    toolbar_row.add_css_class("toolbar");

    fn make_tool_button(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
        vbox.set_halign(gtk::Align::Center);
        let img = gtk::Image::from_icon_name(icon);
        img.set_pixel_size(24);
        vbox.append(&img);
        let lbl = gtk::Label::new(Some(label));
        lbl.add_css_class("caption");
        vbox.append(&lbl);
        let btn = gtk::Button::new();
        btn.set_child(Some(&vbox));
        btn.set_tooltip_text(Some(tooltip));
        btn.add_css_class("flat");
        btn
    }

    let btn_new_folder = make_tool_button("folder-new-symbolic", "New Folder", "Create Folder (F7)");
    toolbar_row.append(&btn_new_folder);

    let btn_delete = make_tool_button("edit-delete-symbolic", "Delete", "Delete (Del)");
    toolbar_row.append(&btn_delete);

    let btn_copy = make_tool_button("edit-copy-symbolic", "Copy", "Copy (F5)");
    toolbar_row.append(&btn_copy);

    let btn_move = make_tool_button("document-send-symbolic", "Move", "Move (F6)");
    toolbar_row.append(&btn_move);

    let btn_refresh = make_tool_button("view-refresh-symbolic", "Refresh", "Refresh");
    toolbar_row.append(&btn_refresh);

    let sep1 = gtk::Separator::new(gtk::Orientation::Vertical);
    toolbar_row.append(&sep1);

    let btn_create_archive = make_tool_button("document-new-symbolic", "Archive", "Create Archive");
    toolbar_row.append(&btn_create_archive);

    let btn_extract = make_tool_button("extract-archive-symbolic", "Extract", "Extract Archive");
    toolbar_row.append(&btn_extract);

    let sep2 = gtk::Separator::new(gtk::Orientation::Vertical);
    toolbar_row.append(&sep2);

    let bookmarks_toggle = {
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
        vbox.set_halign(gtk::Align::Center);
        let img = gtk::Image::from_icon_name("user-bookmarks-symbolic");
        img.set_pixel_size(24);
        vbox.append(&img);
        let lbl = gtk::Label::new(Some("Bookmarks"));
        lbl.add_css_class("caption");
        vbox.append(&lbl);
        let btn = gtk::ToggleButton::new();
        btn.set_child(Some(&vbox));
        btn.set_tooltip_text(Some("Bookmarks"));
        btn.add_css_class("flat");
        btn
    };
    toolbar_row.append(&bookmarks_toggle);

    let btn_info = make_tool_button("dialog-information-symbolic", "Info", "File Information");
    toolbar_row.append(&btn_info);

    let search_box = gtk::SearchEntry::new();
    search_box.set_placeholder_text(Some("Filter files..."));
    search_box.set_hexpand(true);
    toolbar_row.append(&search_box);

    let spinner = gtk::Spinner::new();
    toolbar_row.append(&spinner);

    content_box.append(&toolbar_row);

    let show_hidden = Rc::new(Cell::new(false));

    // --- Main content area ---
    let main_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 0);

    // Bookmarks sidebar
    let bookmarks_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bookmarks_box.set_width_request(200);
    bookmarks_box.set_visible(false);

    let bookmarks_header = gtk::Label::builder()
        .label("Bookmarks")
        .xalign(0.5)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    bookmarks_header.add_css_class("heading");
    bookmarks_box.append(&bookmarks_header);

    let bookmarks_list = gtk::ListBox::new();
    bookmarks_list.add_css_class("navigation-sidebar");
    let bookmarks_scrolled = gtk::ScrolledWindow::builder()
        .child(&bookmarks_list)
        .vexpand(true)
        .build();
    bookmarks_box.append(&bookmarks_scrolled);
    refresh_bookmarks_list(&bookmarks_list);

    main_hbox.append(&bookmarks_box);

    // Main panel
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
    let (panel_widget, panel_state) = panels::create_panel(&home, show_hidden.clone());
    store_panel_state(&panel_state);
    panel_widget.set_hexpand(true);
    panel_widget.set_vexpand(true);
    main_hbox.append(&panel_widget);
    content_box.append(&main_hbox);

    // --- Toggle hidden ---
    {
        let action = gio::SimpleAction::new_stateful("toggle-hidden", None, &glib::Variant::from(false));
        let ps = panel_state.clone();
        action.connect_activate(move |act, _param| {
            let new_val = !show_hidden.get();
            show_hidden.set(new_val);
            act.set_state(&glib::Variant::from(new_val));
            panels::load_directory(&ps);
        });
        window.add_action(&action);
    }

    // --- About ---
    {
        let action = gio::SimpleAction::new("about", None);
        action.connect_activate(move |_, _| {
            let about = adw::AboutDialog::builder()
                .application_name(config::APP_NAME)
                .application_icon("7zip-linux")
                .version(config::VERSION)
                .copyright("© 2026 IdanPlusTV")
                .license_type(gtk::License::Gpl30)
                .website("https://github.com/antoxa78/7zip-linux")
                .build();
            about.add_credit_section(Some("Developer"), &["IdanPlusTV"]);
            about.present(crate::utils::parent_window().as_ref());
        });
        window.add_action(&action);
    }

    // --- File Associations ---
    {
        let mime_types = [
            "application/x-7z-compressed",
            "application/x-rar",
            "application/zip",
            "application/gzip",
            "application/x-tar",
            "application/x-bzip2",
            "application/x-xz",
            "application/x-zstd",
            "application/x-lz4",
        ];
        let desktop_file = "7zip-linux.desktop";

        let action = gio::SimpleAction::new("register-assoc", None);
        action.connect_activate(move |_, _| {
            let mut errors = Vec::new();
            for mime in &mime_types {
                let status = std::process::Command::new("xdg-mime")
                    .args(["default", desktop_file, mime])
                    .status();
                if let Err(e) = status {
                    errors.push(format!("{}: {}", mime, e));
                }
            }
            if errors.is_empty() {
                crate::utils::show_info("File Associations Registered",
                    "This application is now the default for supported archive types.");
            } else {
                crate::utils::show_error("Registration Failed", &errors.join("\n"));
            }
        });
        window.add_action(&action);

        let action = gio::SimpleAction::new("unregister-assoc", None);
        action.connect_activate(move |_, _| {
            let mut errors = Vec::new();
            for mime in &mime_types {
                let status = std::process::Command::new("xdg-mime")
                    .args(["undefault", desktop_file, mime])
                    .status();
                if let Err(e) = status {
                    errors.push(format!("{}: {}", mime, e));
                }
            }
            if errors.is_empty() {
                crate::utils::show_info("File Associations Unregistered",
                    "This application is no longer the default for archive types.");
            } else {
                crate::utils::show_error("Unregistration Failed", &errors.join("\n"));
            }
        });
        window.add_action(&action);
    }

    // --- File Manager Integration Scripts ---
    {
        let extract_here_script = r#"#!/bin/bash
# Extract archive(s) here using 7-Zip
for f in "$@"; do
    7z x "$f" -o"$(dirname "$f")" -aoa
done"#;

        let extract_to_script = r#"#!/bin/bash
# Extract archive(s) to chosen folder
DIR=$(zenity --file-selection --directory --title="Extract to folder")
if [ -n "$DIR" ]; then
    for f in "$@"; do
        7z x "$f" -o"$DIR" -aoa
    done
fi"#;

        let create_archive_script = r#"#!/bin/bash
# Create 7z archive from selected files
ARCHIVE=$(zenity --entry --title="Create Archive" --text="Archive name:" --entry-text="archive.7z")
if [ -n "$ARCHIVE" ]; then
    7z a "$ARCHIVE" "$@"
    zenity --info --text="Archive created: $ARCHIVE"
fi"#;

        let action = gio::SimpleAction::new("install-fm-scripts", None);
        action.connect_activate(move |_, _| {
            let nautilus_dir = dirs::home_dir().map(|h| h.join(".local/share/nautilus/scripts"));
            let nemo_dir = dirs::home_dir().map(|h| h.join(".local/share/nemo/scripts"));
            let thunar_dir = dirs::home_dir().map(|h| h.join(".config/Thunar"));
            let dolphin_dir = dirs::home_dir().map(|h| h.join(".local/share/kservices5/servicemenus"));

            let mut installed = Vec::new();
            let mut errors = Vec::new();

            // Nautilus scripts
            if let Some(ref dir) = nautilus_dir {
                let _ = std::fs::create_dir_all(dir);
                let scripts = [
                    ("7zip-Extract Here", extract_here_script),
                    ("7zip-Extract To...", extract_to_script),
                    ("7zip-Create Archive", create_archive_script),
                ];
                for (name, content) in &scripts {
                    let path = dir.join(name);
                    if std::fs::write(&path, content).is_ok() {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
                        }
                        installed.push(format!("Nautilus: {}", name));
                    } else {
                        errors.push(format!("Nautilus: {}", name));
                    }
                }
            }

            // Nemo scripts
            if let Some(ref dir) = nemo_dir {
                let _ = std::fs::create_dir_all(dir);
                let scripts = [
                    ("7zip-Extract Here", extract_here_script),
                    ("7zip-Extract To...", extract_to_script),
                    ("7zip-Create Archive", create_archive_script),
                ];
                for (name, content) in &scripts {
                    let path = dir.join(name);
                    if std::fs::write(&path, content).is_ok() {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
                        }
                        installed.push(format!("Nemo: {}", name));
                    } else {
                        errors.push(format!("Nemo: {}", name));
                    }
                }
            }

            // Thunar custom actions
            if let Some(ref dir) = thunar_dir {
                let _ = std::fs::create_dir_all(dir);
                let uca_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<actions>
<action>
    <icon>extract-archive</icon>
    <name>Extract Here (7-Zip)</name>
    <unique-id>7zip-extract-here</unique-id>
    <command>7z x %f -o%p -aoa</command>
    <patterns>*.7z;*.rar;*.zip;*.tar;*.tar.gz;*.tar.bz2;*.tar.xz;*.tar.zst</patterns>
    <conditions>!d</conditions>
    <description>Extract the archive in its current directory</description>
</action>
<action>
    <icon>create-archive</icon>
    <name>Create Archive (7-Zip)</name>
    <unique-id>7zip-create-archive</unique-id>
    <command>7z a %f.7z %F</command>
    <patterns>*</patterns>
    <description>Create a 7z archive from selected files</description>
</action>
</actions>"#;
                let path = dir.join("uca.xml");
                if std::fs::write(&path, uca_xml).is_ok() {
                    installed.push("Thunar: custom actions".to_string());
                } else {
                    errors.push("Thunar: custom actions".to_string());
                }
            }

            // Dolphin service menus
            if let Some(ref dir) = dolphin_dir {
                let _ = std::fs::create_dir_all(dir);
                let desktop_extract = format!(
                    "[Desktop Entry]
Type=Service
ServiceTypes=Application/zip;application/x-7z-compressed;application/x-rar;application/gzip;application/x-tar;application/x-bzip2;application/x-xz;application/x-zstd
X-KDE-Submenu=7-Zip
Actions=ExtractHere;ExtractTo

[Desktop Action ExtractHere]
Name=Extract Here
Exec=7z x %f -o$(dirname %f) -aoa
Icon=extract-archive

[Desktop Action ExtractTo]
Name=Extract To...
Exec=bash -c 'DIR=$(zenity --file-selection --directory); 7z x %f -o\"$DIR\" -aoa'
Icon=extract-archive-to"
                );
                let desktop_create = format!(
                    "[Desktop Entry]
Type=Service
ServiceTypes=all/all
X-KDE-Submenu=7-Zip
Actions=CreateArchive

[Desktop Action CreateArchive]
Name=Create Archive (7z)
Exec=bash -c 'ARCHIVE=$(zenity --entry --title=\"Create Archive\" --text=\"Archive name:\" --entry-text=\"archive.7z\"); 7z a \"$ARCHIVE\" %F'
Icon=package-new"
                );
                let scripts = [
                    ("7zip-extract.desktop", &desktop_extract),
                    ("7zip-create.desktop", &desktop_create),
                ];
                for (name, content) in &scripts {
                    let path = dir.join(name);
                    if std::fs::write(&path, content).is_ok() {
                        installed.push(format!("Dolphin: {}", name));
                    } else {
                        errors.push(format!("Dolphin: {}", name));
                    }
                }
            }

            let detail = format!("Installed {} scripts:\n{}", installed.len(), installed.join("\n"));
            if errors.is_empty() {
                crate::utils::show_info("File Manager Scripts Installed", &detail);
            } else {
                crate::utils::show_error("Partial Install", &format!("{}\n\nErrors:\n{}", detail, errors.join("\n")));
            }
        });
        window.add_action(&action);

        let action = gio::SimpleAction::new("uninstall-fm-scripts", None);
        action.connect_activate(move |_, _| {
            let nautilus_dir = dirs::home_dir().map(|h| h.join(".local/share/nautilus/scripts"));
            let nemo_dir = dirs::home_dir().map(|h| h.join(".local/share/nemo/scripts"));
            let thunar_dir = dirs::home_dir().map(|h| h.join(".config/Thunar"));
            let dolphin_dir = dirs::home_dir().map(|h| h.join(".local/share/kservices5/servicemenus"));

            let mut removed = Vec::new();

            // Nautilus
            if let Some(ref dir) = nautilus_dir {
                for name in &["7zip-Extract Here", "7zip-Extract To...", "7zip-Create Archive"] {
                    let path = dir.join(name);
                    if std::fs::remove_file(&path).is_ok() {
                        removed.push(format!("Nautilus: {}", name));
                    }
                }
            }

            // Nemo
            if let Some(ref dir) = nemo_dir {
                for name in &["7zip-Extract Here", "7zip-Extract To...", "7zip-Create Archive"] {
                    let path = dir.join(name);
                    if std::fs::remove_file(&path).is_ok() {
                        removed.push(format!("Nemo: {}", name));
                    }
                }
            }

            // Thunar
            if let Some(ref dir) = thunar_dir {
                let path = dir.join("uca.xml");
                if std::fs::remove_file(&path).is_ok() {
                    removed.push("Thunar: uca.xml".to_string());
                }
            }

            // Dolphin
            if let Some(ref dir) = dolphin_dir {
                for name in &["7zip-extract.desktop", "7zip-create.desktop"] {
                    let path = dir.join(name);
                    if std::fs::remove_file(&path).is_ok() {
                        removed.push(format!("Dolphin: {}", name));
                    }
                }
            }

            if removed.is_empty() {
                crate::utils::show_info("Nothing to Remove", "No file manager scripts were found.");
            } else {
                crate::utils::show_info("Scripts Removed", &format!("Removed {} scripts:\n{}", removed.len(), removed.join("\n")));
            }
        });
        window.add_action(&action);
    }
    {
        let ps = panel_state.clone();
        let sp = spinner.clone();
        btn_new_folder.connect_clicked(move |_| {
            let current = { ps.borrow().current_path.clone() };
            if crate::archive::browse::parse_archive_path(&current).is_some() {
                crate::utils::show_error("New Folder", "Cannot create folder inside an archive");
                return;
            }
            let dialog = adw::AlertDialog::builder()
                .heading("New Folder")
                .body("Enter folder name:")
                .build();
            let entry = gtk::Entry::builder().placeholder_text("New Folder").hexpand(true).build();
            entry.set_text("New Folder");
            dialog.set_extra_child(Some(&entry));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("create", "Create");
            let ps2 = ps.clone();
            let sp2 = sp.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "create" {
                    let name = entry.text().to_string();
                    if !name.is_empty() {
                        let path = current.join(&name);
                        let ps3 = ps2.clone();
                        let sp3 = sp2.clone();
                        glib::spawn_future_local(async move {
                            sp3.set_spinning(true);
                            if let Err(e) = crate::operations::mkdir::create_directory(&path).await {
                                crate::utils::show_error("New Folder", &e);
                            }
                            panels::load_directory(&ps3);
                            sp3.set_spinning(false);
                        });
                    }
                }
            });
            dialog.present(crate::utils::parent_window().as_ref());
        });
    }

    {
        let ps = panel_state.clone();
        let sp = spinner.clone();
        btn_delete.connect_clicked(move |_| {
            let selected = panels::get_selected_names(&ps);
            if selected.is_empty() { return; }
            let count = selected.len();
            let msg = if count == 1 { format!("Delete \"{}\"?", selected[0]) } else { format!("Delete {} items?", count) };
            let dialog = adw::AlertDialog::builder().heading("Confirm Delete").body(&msg).build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("delete", "Delete");
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            let ps2 = ps.clone();
            let sp2 = sp.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "delete" {
                    let ps3 = ps2.clone();
                    let sel = selected.clone();
                    let sp3 = sp2.clone();
                    let current = { ps3.borrow().current_path.clone() };
                    glib::spawn_future_local(async move {
                        sp3.set_spinning(true);
                        for name in &sel {
                            let _ = crate::operations::delete::delete_entry(&current.join(name)).await;
                        }
                        panels::load_directory(&ps3);
                        sp3.set_spinning(false);
                    });
                }
            });
            dialog.present(crate::utils::parent_window().as_ref());
        });
    }

    {
        let ps = panel_state.clone();
        btn_copy.connect_clicked(move |_| {
            let paths = panels::get_all_selected_paths(&ps);
            if paths.is_empty() {
                return;
            }
            let names = panels::get_selected_names(&ps);
            let count = paths.len();
            crate::clipboard::set(crate::clipboard::ClipboardData {
                paths,
                is_cut: false,
            });
            let s = ps.borrow();
            let msg = if count == 1 {
                format!("{} copied to clipboard", names[0])
            } else {
                format!("{} items copied to clipboard", count)
            };
            s.status_label.set_label(&msg);
        });
    }

    {
        let ps = panel_state.clone();
        btn_move.connect_clicked(move |_| {
            let paths = panels::get_all_selected_paths(&ps);
            if paths.is_empty() {
                return;
            }
            let names = panels::get_selected_names(&ps);
            let count = paths.len();
            let all_in_archive = paths.iter().all(|p| {
                crate::archive::browse::parse_archive_path(p).is_some()
            });
            let is_cut = !all_in_archive;
            crate::clipboard::set(crate::clipboard::ClipboardData {
                paths,
                is_cut,
            });
            let s = ps.borrow();
            let msg = if count == 1 {
                if is_cut {
                    format!("{} cut to clipboard", names[0])
                } else {
                    format!("{} copied to clipboard (archive)", names[0])
                }
            } else if is_cut {
                format!("{} items cut to clipboard", count)
            } else {
                format!("{} items copied to clipboard (archive)", count)
            };
            s.status_label.set_label(&msg);
        });
    }

    {
        let ps = panel_state.clone();
        btn_refresh.connect_clicked(move |_| {
            panels::load_directory(&ps);
        });
    }

    {
        let ps = panel_state.clone();
        btn_create_archive.connect_clicked(move |_| {
            let paths = panels::get_all_selected_paths(&ps);
            if !paths.is_empty() {
                dialogs::create_archive::show(&ps, &paths);
            }
        });
    }

    {
        let ps = panel_state.clone();
        btn_extract.connect_clicked(move |_| {
            let selected = panels::get_selected_path(&ps);
            let path = selected.or_else(|| {
                let current = ps.borrow().current_path.clone();
                let s = current.to_string_lossy().to_string();
                if s.contains(" [archive]") {
                    Some(current)
                } else {
                    None
                }
            });
            if let Some(path) = path {
                let archive = if let Some((archive_path, _)) =
                    crate::archive::browse::parse_archive_path(&path)
                {
                    archive_path
                } else if path.is_file() {
                    path
                } else {
                    return;
                };
                let pw = ps.borrow().current_password.clone();
                dialogs::extract_archive::show(&ps, &archive, pw);
            }
        });
    }

    {
        let ps = panel_state.clone();
        btn_info.connect_clicked(move |_| {
            let paths = panels::get_all_selected_paths(&ps);
            crate::dialogs::properties::show(&paths);
        });
    }

    // Bind toolbar search box to panel filter (supports glob patterns like *.deb)
    {
        let ps = panel_state.clone();
        search_box.connect_changed(move |entry| {
            let text = entry.text().to_string();
            {
                *ps.borrow().search_pattern.borrow_mut() = text;
            }
            ps.borrow().glob_filter.changed(gtk::FilterChange::Different);
        });
    }

    // Bookmarks toggle
    {
        let bb = bookmarks_box.clone();
        bookmarks_toggle.connect_toggled(move |btn| {
            bb.set_visible(btn.is_active());
        });
    }

    // Bookmark clicks
    {
        let ps = panel_state.clone();
        bookmarks_list.connect_row_activated(move |_, row| {
            if let Some(label) = row.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                if let Some(tooltip) = label.tooltip_text() {
                    let path = std::path::PathBuf::from(&tooltip);
                    if path.is_dir() {
                        panels::navigate_to(&ps, &path);
                    }
                }
            }
        });
    }

    // Keyboard shortcuts
    {
        let ps = panel_state.clone();
    let st = search_box.clone();
    let sp = spinner.clone();
        let key_controller = gtk::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                match key {
                    gtk::gdk::Key::c => {
                        let paths = panels::get_all_selected_paths(&ps);
                        if !paths.is_empty() {
                            crate::clipboard::set(crate::clipboard::ClipboardData {
                                paths,
                                is_cut: false,
                            });
                        }
                        glib::Propagation::Stop
                    }
                    gtk::gdk::Key::x => {
                        let paths = panels::get_all_selected_paths(&ps);
                        if !paths.is_empty() {
                            crate::clipboard::set(crate::clipboard::ClipboardData {
                                paths,
                                is_cut: true,
                            });
                        }
                        glib::Propagation::Stop
                    }
                    gtk::gdk::Key::v => {
                        let cb = crate::clipboard::get();
                        if !cb.paths.is_empty() {
                            let current = { ps.borrow().current_path.clone() };
                            let ps2 = ps.clone();
                            let sp2 = sp.clone();
                            glib::spawn_future_local(async move {
                                sp2.set_spinning(true);
                                for path in &cb.paths {
                                    let name = match path.file_name() {
                                        Some(n) => n.to_string_lossy().to_string(),
                                        None => continue,
                                    };
                                    let dest = current.join(&name);
                                    if let Err(e) = crate::operations::copy::copy_file(path, &dest).await {
                                        crate::utils::show_error("Paste Failed", &e);
                                    }
                                }
                                if cb.is_cut {
                                    for path in &cb.paths {
                                        if crate::archive::browse::parse_archive_path(path).is_some() {
                                            continue;
                                        }
                                        let _ = std::fs::remove_file(path);
                                        let _ = std::fs::remove_dir(path);
                                    }
                                }
                                panels::load_directory(&ps2);
                                sp2.set_spinning(false);
                            });
                        }
                        glib::Propagation::Stop
                    }
                    gtk::gdk::Key::F5 => { btn_copy.emit_clicked(); glib::Propagation::Stop }
                    gtk::gdk::Key::F6 => { btn_move.emit_clicked(); glib::Propagation::Stop }
                    gtk::gdk::Key::F7 => { btn_new_folder.emit_clicked(); glib::Propagation::Stop }
                    gtk::gdk::Key::f => { st.grab_focus(); glib::Propagation::Stop }
                    gtk::gdk::Key::a => {
                        ps.borrow().selection_model.select_all();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            } else {
                match key {
                    gtk::gdk::Key::Delete => { btn_delete.emit_clicked(); glib::Propagation::Stop }
                    gtk::gdk::Key::Return => {
                        let bitset = ps.borrow().selection_model.selection();
                        if !bitset.is_empty() {
                            let pos = bitset.nth(0);
                            panels::on_activate(&ps, pos);
                        }
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        });
        window.add_controller(key_controller);
    }

    // Save window size on close
    {
        let settings = settings.clone();
        window.connect_close_request(move |win| {
            let mut s = settings.borrow_mut();
            let (w, h) = win.default_size();
            s.window_width = w;
            s.window_height = h;
            s.save();
            glib::Propagation::Proceed
        });
    }

    window.set_content(Some(&content_box));
    window.present();
}

fn refresh_bookmarks_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let bookmarks = config::bookmarks::load_bookmarks();
    for bm in &bookmarks {
        let row = gtk::Label::builder()
            .label(&bm.name)
            .xalign(0.0)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(8)
            .margin_end(8)
            .build();
        row.set_tooltip_text(Some(&bm.path));
        list.append(&row);
    }
}
