use std::path::Path;

use adw::prelude::*;

pub fn show(paths: &[std::path::PathBuf]) {
    if paths.is_empty() {
        return;
    }

    let dialog = adw::Dialog::builder()
        .title("Properties")
        .content_width(380)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_vexpand(true);

    // Header: big icon + name, then the details as a boxed list of property rows.
    let hero = gtk::Box::new(gtk::Orientation::Vertical, 6);
    hero.set_margin_bottom(6);
    let hero_icon = gtk::Image::new();
    hero_icon.set_pixel_size(64);
    let hero_title = gtk::Label::builder()
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["title-3"])
        .build();
    hero.append(&hero_icon);
    hero.append(&hero_title);
    content.append(&hero);

    let grid = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .valign(gtk::Align::Start)
        .build();

    if paths.len() == 1 {
        let path = &paths[0];
        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        dialog.set_title("Properties");
        hero_title.set_label(&name);
        if path.is_dir() {
            hero_icon.set_icon_name(Some("folder"));
        } else {
            let (ct, _) = gtk::gio::content_type_guess(Some(name.as_str()), None::<&[u8]>);
            hero_icon.set_from_gicon(&gtk::gio::content_type_get_icon(&ct));
        }
        let mut row = 0;
        add_property_row(&grid, row, "Location:", &crate::panels::display_path(path.parent().unwrap_or(path))); row += 1;

        let file_type = if path.is_symlink() {
            format!("Link to {}", std::fs::read_link(path).map(|t| t.display().to_string()).unwrap_or_default())
        } else if path.is_dir() || path.is_file() {
            crate::panels::type_description(&name, path.is_dir())
        } else if path.is_symlink() {
            "Symbolic Link".to_string()
        } else {
            "Unknown".to_string()
        };
        add_property_row(&grid, row, "Type:", &file_type); row += 1;

        if let Ok(metadata) = std::fs::metadata(path) {
            let size = if path.is_dir() {
                dir_size(path)
            } else {
                metadata.len()
            };
            add_property_row(&grid, row, "Size:", &crate::utils::format::format_size(size)); row += 1;

            if let Ok(modified) = metadata.modified() {
                if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                    add_property_row(&grid, row, "Modified:", &crate::utils::format::format_timestamp_full(dur.as_secs())); row += 1;
                }
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                add_property_row(&grid, row, "Permissions:", &permissions_text(metadata.mode())); row += 1;
                add_property_row(&grid, row, "Owner:", &owner_name(metadata.uid())); row += 1;
            }

            if path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(path) {
                    let count = entries.count();
                    add_property_row(&grid, row, "Items:", &format!("{}", count)); row += 1;
                }
            }
        }

        if path.is_file() && crate::panels::is_archive_file_check(path) {
            add_property_row(&grid, row, "Archive:", "Yes (double-click to browse)");
        }
    } else {
        dialog.set_title("Properties");
        hero_icon.set_icon_name(Some("edit-select-all-symbolic"));
        hero_icon.add_css_class("dim-label");
        hero_title.set_label(&format!("{} items", paths.len()));

        let mut total_size: u64 = 0;
        let mut total_dirs: u32 = 0;
        let mut total_files: u32 = 0;
        let mut names = Vec::new();

        for path in paths {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
            if path.is_dir() {
                total_dirs += 1;
                total_size += dir_size(path);
            } else if path.is_file() {
                total_files += 1;
                if let Ok(meta) = std::fs::metadata(path) {
                    total_size += meta.len();
                }
            }
        }

        if !names.is_empty() {
            let display = if names.len() <= 3 {
                names.join(", ")
            } else {
                format!("{}, ... (+{} more)", names[..3].join(", "), names.len() - 3)
            };
            add_property_row(&grid, 1, "Files:", &display);
        }

        let mut type_parts = Vec::new();
        if total_files > 0 {
            type_parts.push(format!("{} file(s)", total_files));
        }
        if total_dirs > 0 {
            type_parts.push(format!("{} folder(s)", total_dirs));
        }
        add_property_row(&grid, 2, "Type:", &type_parts.join(", "));
        add_property_row(&grid, 3, "Total Size:", &crate::utils::format::format_size(total_size));
    }

    content.append(&grid);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();
    toolbar_view.set_content(Some(&scrolled));

    dialog.set_child(Some(&toolbar_view));

    dialog.present(crate::utils::parent_window().as_ref());
}

/// "rw-r--r-- (644)" — readable, with the octal value for people who think in numbers.
fn permissions_text(mode: u32) -> String {
    let bits = mode & 0o777;
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let b = (bits >> shift) & 0o7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    format!("{} ({:o})", s, bits)
}

/// User name for a uid, from /etc/passwd; falls back to the number.
fn owner_name(uid: u32) -> String {
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let mut f = line.split(':');
                let name = f.next()?;
                let _ = f.next();
                (f.next()?.parse::<u32>().ok()? == uid).then(|| name.to_string())
            })
        })
        .unwrap_or_else(|| uid.to_string())
}

fn add_property_row(list: &gtk::ListBox, _row: i32, label: &str, value: &str) {
    let row = adw::ActionRow::builder()
        .title(label.trim_end_matches(':'))
        .subtitle(value)
        .subtitle_selectable(true)
        .build();
    row.add_css_class("property");
    list.append(&row);
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() {
                        stack.push(entry.path());
                    } else {
                        total += meta.len();
                    }
                }
            }
        }
    }
    total
}
