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
    let old = PANEL_STATE.swap(ptr as *mut std::ffi::c_void, Ordering::Relaxed);
    if !old.is_null() {
        unsafe { drop(Box::from_raw(old as *mut SharedPanel)); }
    }
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
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("sevenzip-gui-drag"));

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
    // Native look first: Adwaita colours, the user's accent and light/dark follow the
    // system. The one brand touch is the amber of the app icon, used only to mark
    // "you are inside an archive" in the path bar.
    provider.load_from_string(
        ".action-bar-row { padding: 2px 6px 6px 6px; }\n\
         .tool-button { padding: 4px 6px; min-width: 52px; }\n\
         .tool-button label { font-size: 0.8em; font-weight: normal; }\n\
         .action-bar-row > separator { margin: 8px 6px; }\n\
         .path-bar { padding: 6px 8px; }\n\
         .path-bar entry { margin-left: 6px; }\n\
         .path-bar entry.in-archive { background-color: alpha(@yellow_3, 0.28); box-shadow: inset 0 0 0 1px alpha(@yellow_5, 0.6); }\n\
         .path-bar entry.in-archive > image:first-child { color: @yellow_5; opacity: 1; }\n\
         .statusbar { padding: 5px 12px; min-height: 26px; border-top: 1px solid alpha(currentColor, 0.1); }\n\
         .status-label { font-size: 0.9em; }\n\
         .status-progress trough, .status-progress progress { min-height: 6px; }\n\
         .places-title { margin: 12px 12px 4px 12px; }\n"
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(config::APP_NAME)
        .default_width(settings.borrow().window_width)
        .default_height(settings.borrow().window_height)
        .width_request(720)
        .height_request(420)
        .build();

    crate::utils::set_app_window(&window);

    // --- Header bar: sidebar toggle | current folder as title | menu ---
    let header = adw::HeaderBar::new();
    let window_title = adw::WindowTitle::new(config::APP_NAME, "");
    header.set_title_widget(Some(&window_title));

    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Show places (F9)")
        .active(true)
        .build();
    header.pack_start(&sidebar_toggle);

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
    help_section.append(Some("About 7-Zip Linux"), Some("win.about"));
    menu.append_section(None, &help_section);

    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Main menu")
        .primary(true)
        .build();
    header.pack_end(&menu_button);

    // --- Action bar (below the header): archive actions | file actions | properties ---
    let toolbar_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    toolbar_row.add_css_class("action-bar-row");

    fn make_tool_button(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
        vbox.set_halign(gtk::Align::Center);
        let img = gtk::Image::from_icon_name(icon);
        img.set_pixel_size(16);
        vbox.append(&img);
        let lbl = gtk::Label::new(Some(label));
        vbox.append(&lbl);
        let btn = gtk::Button::new();
        btn.set_child(Some(&vbox));
        btn.set_tooltip_text(Some(tooltip));
        btn.add_css_class("flat");
        btn.add_css_class("tool-button");
        btn
    }

    let btn_create_archive = make_tool_button("package-x-generic-symbolic", "Archive", "Create an archive from the selection");
    toolbar_row.append(&btn_create_archive);

    let btn_password_protect = make_tool_button("password-protect-archive-symbolic", "Encrypt", "Create a password-protected archive from the selection");
    toolbar_row.append(&btn_password_protect);

    let btn_add_to_archive = make_tool_button("archive-add-symbolic", "Add", "Add files to the open or selected archive");
    toolbar_row.append(&btn_add_to_archive);

    let btn_extract = make_tool_button("archive-extract-symbolic", "Extract", "Extract the selected or open archive");
    toolbar_row.append(&btn_extract);

    toolbar_row.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    let btn_copy = make_tool_button("edit-copy-symbolic", "Copy", "Copy (F5)");
    toolbar_row.append(&btn_copy);

    let btn_move = make_tool_button("edit-cut-symbolic", "Move", "Move (F6)");
    toolbar_row.append(&btn_move);

    let btn_paste = make_tool_button("edit-paste-symbolic", "Paste", "Paste (Ctrl+V)");
    toolbar_row.append(&btn_paste);

    let btn_new_folder = make_tool_button("folder-new-symbolic", "New Folder", "New folder (F7)");
    toolbar_row.append(&btn_new_folder);

    let btn_delete = make_tool_button("user-trash-symbolic", "Delete", "Delete (Del)");
    toolbar_row.append(&btn_delete);

    toolbar_row.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    let btn_info = make_tool_button("document-properties-symbolic", "Properties", "Properties");
    toolbar_row.append(&btn_info);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    toolbar_row.append(&spacer);

    let spinner = gtk::Spinner::new();
    spinner.set_valign(gtk::Align::Center);
    // Only take up room while something is running.
    spinner.bind_property("spinning", &spinner, "visible").sync_create().build();
    toolbar_row.append(&spinner);

    let search_box = gtk::SearchEntry::new();
    search_box.set_placeholder_text(Some("Filter, e.g. *.pdf"));
    search_box.set_width_chars(22);
    search_box.set_valign(gtk::Align::Center);
    search_box.set_tooltip_text(Some("Filter this folder (Ctrl+F). Supports * and ? wildcards."));
    toolbar_row.append(&search_box);

    let show_hidden = Rc::new(Cell::new(false));

    // --- Places sidebar ---
    let bookmarks_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let bookmarks_header = gtk::Label::builder()
        .label("Places")
        .xalign(0.0)
        .build();
    bookmarks_header.add_css_class("heading");
    bookmarks_header.add_css_class("dim-label");
    bookmarks_header.add_css_class("places-title");
    bookmarks_box.append(&bookmarks_header);

    let bookmarks_list = gtk::ListBox::new();
    bookmarks_list.add_css_class("navigation-sidebar");
    let bookmarks_scrolled = gtk::ScrolledWindow::builder()
        .child(&bookmarks_list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    bookmarks_box.append(&bookmarks_scrolled);
    refresh_bookmarks_list(&bookmarks_list);

    // Main panel
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
    let (panel_widget, panel_state) = panels::create_panel(&home, show_hidden.clone());
    store_panel_state(&panel_state);
    panel_state.borrow_mut().window_title = Some(window_title.clone());
    panels::load_directory(&panel_state);
    panel_widget.set_hexpand(true);
    panel_widget.set_vexpand(true);

    let split_view = adw::OverlaySplitView::builder()
        .sidebar(&bookmarks_box)
        .content(&panel_widget)
        .min_sidebar_width(170.0)
        .max_sidebar_width(230.0)
        .build();
    sidebar_toggle
        .bind_property("active", &split_view, "show-sidebar")
        .bidirectional()
        .sync_create()
        .build();

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.add_top_bar(&toolbar_row);
    toolbar_view.set_content(Some(&split_view));

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
                .copyright("© 2026 Antoxa78")
                .license_type(gtk::License::Gpl30)
                .website("https://github.com/antoxa78/7zip-linux")
                .build();
            about.add_credit_section(Some("Developer"), &["Antoxa78"]);
            about.add_credit_section(Some("Build Date"), &[config::BUILD_DATE_TIME]);
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
        let extract_here_script = include_str!("../data/scripts/extract-here.sh");

        let extract_to_script = include_str!("../data/scripts/extract-to.sh");

        let create_archive_script = include_str!("../data/scripts/create-archive.sh");

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
            panels::new_folder(&ps, Some(sp.clone()));
        });
    }

    {
        let ps = panel_state.clone();
        let sp = spinner.clone();
        btn_delete.connect_clicked(move |_| {
            panels::delete_selection(&ps, Some(sp.clone()));
        });
    }

    {
        let ps = panel_state.clone();
        btn_copy.connect_clicked(move |_| {
            panels::copy_selection(&ps, false);
        });
    }

    {
        let ps = panel_state.clone();
        btn_move.connect_clicked(move |_| {
            panels::copy_selection(&ps, true);
        });
    }

    {
        let ps = panel_state.clone();
        let sp = spinner.clone();
        btn_paste.connect_clicked(move |_| {
            panels::paste_clipboard(&ps, Some(sp.clone()));
        });
    }

    {
        let ps = panel_state.clone();
        btn_create_archive.connect_clicked(move |_| {
            let paths = panels::get_all_selected_paths(&ps);
            if !paths.is_empty() {
                dialogs::create_archive::show(&ps, &paths, false);
            }
        });
    }

    {
        let ps = panel_state.clone();
        btn_password_protect.connect_clicked(move |_| {
            let paths = panels::get_all_selected_paths(&ps);
            if paths.is_empty() {
                crate::utils::show_info("Password Protect Archive",
                    "Select files first, then choose Password Protect Archive.");
                return;
            }
            dialogs::create_archive::show(&ps, &paths, true);
        });
    }

    {
        let ps = panel_state.clone();
        let sp = spinner.clone();
        btn_add_to_archive.connect_clicked(move |_| {
            panels::add_to_archive_dialog(&ps, Some(sp.clone()));
        });
    }

    {
        let ps = panel_state.clone();
        btn_extract.connect_clicked(move |_| {
            // A selected archive file, or the archive currently being browsed.
            let archive = match panels::get_selected_path(&ps) {
                Some(path) => match crate::archive::browse::parse_archive_path(&path) {
                    Some((archive_path, _)) => Some(archive_path),
                    None if path.is_file() => Some(path),
                    None => None,
                },
                None => panels::current_archive_location(&ps).map(|(a, _)| a),
            };
            if let Some(archive) = archive {
                let pw = panels::password_for_archive(&ps, &archive);
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
            panels::update_status(&ps);
        });
    }

    // Bookmark clicks
    {
        let ps = panel_state.clone();
        bookmarks_list.connect_row_activated(move |_, row| {
            let path = std::path::PathBuf::from(row.widget_name().as_str());
            if path.is_dir() {
                panels::navigate_to(&ps, &path);
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
                        panels::copy_selection(&ps, false);
                        glib::Propagation::Stop
                    }
                    gtk::gdk::Key::x => {
                        panels::copy_selection(&ps, true);
                        glib::Propagation::Stop
                    }
                    gtk::gdk::Key::v => {
                        panels::paste_clipboard(&ps, Some(sp.clone()));
                        glib::Propagation::Stop
                    }
                    gtk::gdk::Key::f => { st.grab_focus(); glib::Propagation::Stop }
                    gtk::gdk::Key::r => { panels::load_directory(&ps); glib::Propagation::Stop }
                    gtk::gdk::Key::a => {
                        ps.borrow().selection_model.select_all();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            } else if modifiers.contains(gtk::gdk::ModifierType::ALT_MASK) {
                match key {
                    gtk::gdk::Key::Left => { panels::go_back(&ps); glib::Propagation::Stop }
                    gtk::gdk::Key::Right => { panels::go_forward(&ps); glib::Propagation::Stop }
                    gtk::gdk::Key::Up => { panels::go_up(&ps); glib::Propagation::Stop }
                    _ => glib::Propagation::Proceed,
                }
            } else {
                // Plain function keys, as advertised in tooltips and menus.
                match key {
                    gtk::gdk::Key::F2 => { panels::ctx_rename(&ps); glib::Propagation::Stop }
                    gtk::gdk::Key::F5 => { btn_copy.emit_clicked(); glib::Propagation::Stop }
                    gtk::gdk::Key::F6 => { btn_move.emit_clicked(); glib::Propagation::Stop }
                    gtk::gdk::Key::F7 => { btn_new_folder.emit_clicked(); glib::Propagation::Stop }
                    gtk::gdk::Key::F9 => { sidebar_toggle.set_active(!sidebar_toggle.is_active()); glib::Propagation::Stop }
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
            let tmp = std::env::temp_dir();
            let _ = std::fs::remove_dir_all(tmp.join("sevenzip-gui-open"));
            let _ = std::fs::remove_dir_all(tmp.join("sevenzip-gui-list"));
            let _ = std::fs::remove_dir_all(tmp.join("sevenzip-gui-drag"));
            glib::Propagation::Proceed
        });
    }

    window.set_content(Some(&toolbar_view));
    window.present();
}

fn refresh_bookmarks_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let home = dirs::home_dir();
    let icon_for = |path: &std::path::Path| -> &'static str {
        if path == std::path::Path::new("/") {
            return "drive-harddisk-symbolic";
        }
        if home.as_deref() == Some(path) {
            return "user-home-symbolic";
        }
        let is = |d: Option<std::path::PathBuf>| d.as_deref() == Some(path);
        if is(dirs::desktop_dir()) { "user-desktop-symbolic" }
        else if is(dirs::document_dir()) { "folder-documents-symbolic" }
        else if is(dirs::download_dir()) { "folder-download-symbolic" }
        else if is(dirs::audio_dir()) { "folder-music-symbolic" }
        else if is(dirs::picture_dir()) { "folder-pictures-symbolic" }
        else if is(dirs::video_dir()) { "folder-videos-symbolic" }
        else {
            match path.file_name().and_then(|n| n.to_str()) {
                Some("Desktop") => "user-desktop-symbolic",
                Some("Documents") => "folder-documents-symbolic",
                Some("Downloads") => "folder-download-symbolic",
                Some("Music") => "folder-music-symbolic",
                Some("Pictures") => "folder-pictures-symbolic",
                Some("Videos") => "folder-videos-symbolic",
                _ => "folder-symbolic",
            }
        }
    };
    for bm in &config::bookmarks::load_bookmarks() {
        let path = std::path::PathBuf::from(&bm.path);
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        hbox.append(&gtk::Image::from_icon_name(icon_for(&path)));
        let label = gtk::Label::builder()
            .label(&bm.name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        hbox.append(&label);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&hbox));
        row.set_tooltip_text(Some(&crate::panels::display_path(&path)));
        // The row's widget name carries the real path for the click handler.
        row.set_widget_name(&bm.path);
        list.append(&row);
    }
}
