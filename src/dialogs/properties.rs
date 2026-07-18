use std::path::Path;

use adw::prelude::*;

pub fn show(paths: &[std::path::PathBuf]) {
    if paths.is_empty() {
        return;
    }

    let dialog = adw::Dialog::builder()
        .title("Properties")
        .content_width(400)
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
    content.set_vexpand(true);

    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(8);

    if paths.len() == 1 {
        let path = &paths[0];
        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        dialog.set_title(&format!("Properties - {}", name));
        let mut row = 0;
        add_property_row(&grid, row, "Name:", &name); row += 1;
        add_property_row(&grid, row, "Path:", &path.to_string_lossy()); row += 1;

        let file_type = if path.is_dir() {
            "Directory".to_string()
        } else if path.is_file() {
            path.extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("File (.{})", e))
                .unwrap_or_else(|| "File".to_string())
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
                    add_property_row(&grid, row, "Modified:", &crate::utils::format::format_timestamp(dur.as_secs())); row += 1;
                }
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                add_property_row(&grid, row, "Permissions:", &format!("{:o}", metadata.mode() & 0o777)); row += 1;
                add_property_row(&grid, row, "Owner:", &format!("{}:{}", metadata.uid(), metadata.gid())); row += 1;
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
        dialog.set_title(&format!("Properties - {} items", paths.len()));
        add_property_row(&grid, 0, "Selection:", &format!("{} items", paths.len()));

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

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&grid));
    content.append(&scrolled);
    toolbar_view.set_content(Some(&content));

    let ok_button = gtk::Button::builder()
        .label("OK")
        .build();
    ok_button.add_css_class("suggested-action");

    let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom_bar.set_margin_top(8);
    bottom_bar.set_margin_bottom(8);
    bottom_bar.set_margin_start(12);
    bottom_bar.set_margin_end(12);
    bottom_bar.set_halign(gtk::Align::End);
    bottom_bar.set_hexpand(true);
    bottom_bar.append(&ok_button);
    toolbar_view.add_bottom_bar(&bottom_bar);

    let dialog_ref = dialog.clone();
    ok_button.connect_clicked(move |_| { dialog_ref.close(); });

    dialog.set_child(Some(&toolbar_view));

    dialog.present(crate::utils::parent_window().as_ref());
}

fn add_property_row(grid: &gtk::Grid, row: i32, label: &str, value: &str) {
    let lbl = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .margin_end(8)
        .build();
    lbl.add_css_class("dim-label");
    grid.attach(&lbl, 0, row, 1, 1);

    let val = gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .selectable(true)
        .build();
    grid.attach(&val, 1, row, 1, 1);
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
