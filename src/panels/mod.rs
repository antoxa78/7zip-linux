use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio};

use crate::models::FileItem;
use crate::utils::icon_for_file;

pub struct PanelState {
    pub current_path: PathBuf,
    pub history: Vec<PathBuf>,
    pub history_index: usize,
    pub raw_store: gio::ListStore,
    pub sort_model: gtk::SortListModel,
    pub filter_model: gtk::FilterListModel,
    pub selection_model: gtk::MultiSelection,
    pub path_entry: gtk::Entry,
    pub column_view: gtk::ColumnView,
    pub status_label: gtk::Label,
    pub show_hidden: Rc<Cell<bool>>,
    pub search_entry: gtk::SearchEntry,
    pub search_pattern: Rc<RefCell<String>>,
    pub glob_filter: gtk::CustomFilter,
    pub current_password: Option<String>,
    pub progress_bar: gtk::ProgressBar,
    pub pulse_source: Option<glib::SourceId>,
}

pub type SharedPanel = Rc<RefCell<PanelState>>;

pub fn create_panel(initial_path: &Path, show_hidden: Rc<Cell<bool>>) -> (gtk::Box, SharedPanel) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);

    // ListStore (raw data)
    let raw_store = gio::ListStore::new::<FileItem>();

    // Filter model (custom glob filter)
    let search_pattern = Rc::new(RefCell::new(String::new()));
    let pattern_clone = search_pattern.clone();
    let glob_filter = gtk::CustomFilter::new(move |item| {
        let pat = pattern_clone.borrow();
        if pat.is_empty() {
            return true;
        }
        if let Some(fi) = item.downcast_ref::<FileItem>() {
            return glob_match(&pat, &fi.name());
        }
        true
    });

    let filter_model = gtk::FilterListModel::new(Some(raw_store.clone()), Some(glob_filter.clone()));

    // Sort model
    let sort_model = gtk::SortListModel::new(Some(filter_model.clone()), Option::<gtk::Sorter>::None);

    // Multi selection
    let selection = gtk::MultiSelection::new(Some(sort_model.clone()));

    // ColumnView
    let column_view = gtk::ColumnView::new(Some(selection.clone()));
    column_view.set_show_row_separators(true);
    column_view.set_show_column_separators(false);
    column_view.set_enable_rubberband(true);

    // Path entry
    let path_entry = gtk::Entry::new();
    path_entry.set_hexpand(true);
    path_entry.set_placeholder_text(Some("Enter path..."));

    // Status label
    let status_label = gtk::Label::new(Some("0 items"));
    status_label.set_xalign(0.0);
    status_label.set_margin_start(8);
    status_label.set_css_classes(&["caption", "dim-label", "status-label"]);

    // Search entry
    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Filter files..."));

    let progress_bar = gtk::ProgressBar::builder()
        .visible(false)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .margin_start(8)
        .margin_end(8)
        .build();
    progress_bar.add_css_class("osd");
    progress_bar.set_size_request(-1, 20);

    let state = Rc::new(RefCell::new(PanelState {
        current_path: initial_path.to_path_buf(),
        history: vec![initial_path.to_path_buf()],
        history_index: 0,
        raw_store: raw_store.clone(),
        sort_model: sort_model.clone(),
        filter_model: filter_model.clone(),
        selection_model: selection.clone(),
        path_entry: path_entry.clone(),
        column_view: column_view.clone(),
        status_label: status_label.clone(),
        show_hidden,
        search_entry: search_entry.clone(),
        search_pattern: search_pattern.clone(),
        glob_filter: glob_filter.clone(),
        current_password: None,
        progress_bar: progress_bar.clone(),
        pulse_source: None,
    }));

    setup_columns(&column_view, &state);

    // Attach column_view sorter to sort_model
    if let Some(cv_sorter) = column_view.sorter() {
        sort_model.set_sorter(Some(&cv_sorter));
    }

    // Navigation bar
    let nav_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    nav_bar.set_margin_top(4);
    nav_bar.set_margin_bottom(4);
    nav_bar.set_margin_start(4);
    nav_bar.set_margin_end(4);
    nav_bar.add_css_class("linked");

    let back_button = gtk::Button::from_icon_name("go-previous-symbolic");
    back_button.set_tooltip_text(Some("Go Back"));
    back_button.add_css_class("flat");
    let s = state.clone();
    back_button.connect_clicked(move |_| go_back(&s));
    nav_bar.append(&back_button);

    let forward_button = gtk::Button::from_icon_name("go-next-symbolic");
    forward_button.set_tooltip_text(Some("Go Forward"));
    forward_button.add_css_class("flat");
    let s = state.clone();
    forward_button.connect_clicked(move |_| go_forward(&s));
    nav_bar.append(&forward_button);

    let up_button = gtk::Button::from_icon_name("go-up-symbolic");
    up_button.set_tooltip_text(Some("Go Up"));
    up_button.add_css_class("flat");
    let s = state.clone();
    up_button.connect_clicked(move |_| go_up(&s));
    nav_bar.append(&up_button);

    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Refresh"));
    refresh_button.add_css_class("flat");
    let s = state.clone();
    refresh_button.connect_clicked(move |_| load_directory(&s));
    nav_bar.append(&refresh_button);

    let s = state.clone();
    path_entry.connect_activate(move |entry| {
        let text = entry.text().to_string();
        let path = PathBuf::from(&text);
        if path.is_dir() {
            navigate_to(&s, &path);
        } else if path.is_file() {
            crate::archive::browse::try_open_archive(&s, &path);
        }
    });
    nav_bar.append(&path_entry);
    container.append(&nav_bar);

    // Scrolled window for column view
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(&column_view));
    container.append(&scrolled);

    let bottom_bar = gtk::Box::new(gtk::Orientation::Vertical, 2);
    bottom_bar.set_margin_top(4);
    bottom_bar.set_margin_bottom(6);
    bottom_bar.set_margin_start(4);
    bottom_bar.set_margin_end(4);
    bottom_bar.append(&progress_bar);
    bottom_bar.append(&status_label);
    container.append(&bottom_bar);

    // Double-click to enter directory or open archive
    let s = state.clone();
    column_view.connect_activate(move |_cv, position| {
        on_activate(&s, position);
    });

    // Fallback: GestureClick for double-click (connect_activate may not fire in all GTK4 versions)
    let s = state.clone();
    let click_gesture = gtk::GestureClick::new();
    click_gesture.set_button(1);
    let last_click_time = std::rc::Rc::new(std::cell::Cell::new(0u64));
    let last_click_time2 = last_click_time.clone();
    click_gesture.connect_pressed(move |_gesture, _n_press, _x, _y| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let prev = last_click_time2.get();
        last_click_time2.set(now);
        if prev != 0 && now.saturating_sub(prev) < 400 {
            last_click_time2.set(0);
            let s = s.clone();
            glib::idle_add_local_once(move || {
                let selected = {
                    let s = s.borrow();
                    let sel = s.selection_model.selection();
                    if sel.is_empty() { None } else { Some(sel.nth(0)) }
                };
                if let Some(pos) = selected {
                    on_activate(&s, pos);
                }
            });
        }
    });
    column_view.add_controller(click_gesture);

    // --- Drop target (accept file drops into the panel) ---
    let drop_formats = gdk::ContentFormatsBuilder::new()
        .add_type(gdk::FileList::static_type())
        .add_type(glib::types::Type::STRING)
        .build();
    let drop_target = gtk::DropTarget::builder()
        .formats(&drop_formats)
        .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
        .build();

    let s_for_drop = state.clone();
    drop_target.connect_accept(move |_, _drop| {
        let s = s_for_drop.borrow();
        crate::archive::browse::parse_archive_path(&s.current_path).is_none()
    });

    let s_for_drop2 = state.clone();
    drop_target.connect_drop(move |ds, value, _x, _y| {
        let current = s_for_drop2.borrow().current_path.clone();
        let paths: Vec<std::path::PathBuf> = if let Ok(file_list) = value.get::<gdk::FileList>() {
            file_list.files().iter().filter_map(|f| f.path()).collect()
        } else if let Ok(text) = value.get::<String>() {
            text.lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        return None;
                    }
                    glib::filename_from_uri(line).ok().map(|(p, _)| p)
                })
                .collect()
        } else {
            return false;
        };
        if paths.is_empty() {
            return false;
        }
        let is_ctrl = ds.current_drop()
            .is_some_and(|d| d.device().modifier_state().contains(gdk::ModifierType::CONTROL_MASK));
        let is_move = !is_ctrl;
        let s3 = s_for_drop2.clone();
        glib::spawn_future_local(async move {
            let mut skip_existing = false;
            for path in &paths {
                if let Some(name) = path.file_name() {
                    let dest = current.join(name);
                    if dest.exists() && !skip_existing {
                        match confirm_overwrite(&name.to_string_lossy()).await {
                            0 => {}
                            1 => continue,
                            2 => { skip_existing = true; continue; }
                            _ => break,
                        }
                    }
                    if is_move {
                        if let Err(e) = crate::operations::move_move::move_file(path, &dest).await {
                            crate::utils::show_error("Drop Failed", &e);
                        }
                    } else if let Err(e) = crate::operations::copy::copy_file(path, &dest).await {
                        crate::utils::show_error("Drop Failed", &e);
                    }
                }
            }
            crate::panels::load_directory(&s3);
        });
        true
    });
    container.add_controller(drop_target);

    // Selection changed -> update status bar
    let s = state.clone();
    selection.connect_selection_changed(move |sel, _, _| {
        let sb = s.borrow();
        let selected = sel.selection().size();
        if selected > 0 {
            let total = sb.sort_model.n_items();
            sb.status_label.set_label(&format!("{} items ({} selected)", total, selected));
        } else {
            let total = sb.sort_model.n_items();
            if crate::archive::browse::parse_archive_path(&sb.current_path).is_some() {
                sb.status_label.set_label(&format!("{} items (in archive)", total - 1));
            } else {
                sb.status_label.set_label(&format!("{} items", total));
            }
        }
    });

    // Right-click context menu
    let menu_model = gio::Menu::new();

    let open_section = gio::Menu::new();
    open_section.append(Some("Open"), Some("ctx.open"));
    open_section.append(Some("Open With Default App"), Some("ctx.open-default"));
    menu_model.append_section(Some("Open"), &open_section);

    let edit_section = gio::Menu::new();
    edit_section.append(Some("Copy (F5)"), Some("ctx.copy"));
    edit_section.append(Some("Move (F6)"), Some("ctx.move"));
    edit_section.append(Some("Delete (Del)"), Some("ctx.delete"));
    edit_section.append(Some("Rename (F2)"), Some("ctx.rename"));
    edit_section.append(Some("Paste (Ctrl+V)"), Some("ctx.paste"));
    menu_model.append_section(Some("Edit"), &edit_section);

    let archive_section = gio::Menu::new();
    archive_section.append(Some("Create Archive..."), Some("ctx.create-archive"));
    archive_section.append(Some("Extract Here"), Some("ctx.extract-here"));
    archive_section.append(Some("Extract To..."), Some("ctx.extract-to"));
    archive_section.append(Some("Test Archive"), Some("ctx.test-archive"));
    menu_model.append_section(Some("Archive"), &archive_section);

    let misc_section = gio::Menu::new();
    misc_section.append(Some("New Folder (F7)"), Some("ctx.new-folder"));
    misc_section.append(Some("Add Bookmark"), Some("ctx.add-bookmark"));
    misc_section.append(Some("Properties"), Some("ctx.properties"));
    misc_section.append(Some("Refresh"), Some("ctx.refresh"));
    menu_model.append_section(Some("Tools"), &misc_section);

    let popover = gtk::PopoverMenu::from_model(Some(&menu_model));

    // Register actions via SimpleActionGroup
    let action_group = gio::SimpleActionGroup::new();
    register_ctx_action(&action_group, "open", &state, ctx_open);
    register_ctx_action(&action_group, "open-default", &state, ctx_open_default);
    register_ctx_action(&action_group, "copy", &state, ctx_copy);
    register_ctx_action(&action_group, "move", &state, ctx_move);
    register_ctx_action(&action_group, "delete", &state, ctx_delete);
    register_ctx_action(&action_group, "rename", &state, ctx_rename);
    register_ctx_action(&action_group, "paste", &state, ctx_paste);
    register_ctx_action(&action_group, "create-archive", &state, ctx_create_archive);
    register_ctx_action(&action_group, "extract-here", &state, ctx_extract_here);
    register_ctx_action(&action_group, "extract-to", &state, ctx_extract_to);
    register_ctx_action(&action_group, "test-archive", &state, ctx_test_archive);
    register_ctx_action(&action_group, "new-folder", &state, ctx_new_folder);
    register_ctx_action(&action_group, "add-bookmark", &state, ctx_add_bookmark);
    register_ctx_action(&action_group, "properties", &state, ctx_properties);
    register_ctx_action(&action_group, "refresh", &state, |s| load_directory(s));
    popover.insert_action_group("ctx", Some(&action_group));
    popover.set_parent(&column_view);

    let right_click = gtk::GestureClick::new();
    right_click.set_button(3);
    let cm = popover.clone();
    right_click.connect_pressed(move |gesture, _n_press, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        cm.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        cm.popup();
    });
    column_view.add_controller(right_click);

    load_directory(&state);

    (container, state)
}

fn register_ctx_action(
    group: &gio::SimpleActionGroup,
    name: &str,
    state: &SharedPanel,
    handler: fn(&SharedPanel),
) {
    let action = gio::SimpleAction::new(name, None);
    let s = state.clone();
    let h = handler;
    action.connect_activate(move |_, _| h(&s));
    group.add_action(&action);
}

// --- Selection helpers ---

pub fn get_selected_path(state: &SharedPanel) -> Option<PathBuf> {
    let s = state.borrow();
    let bitset = s.selection_model.selection();
    if bitset.is_empty() {
        return None;
    }
    let pos = bitset.nth(0);
    if let Some(item) = s.sort_model.item(pos) {
        if let Ok(fi) = item.downcast::<FileItem>() {
            return Some(PathBuf::from(fi.path()));
        }
    }
    None
}

pub fn get_all_selected_paths(state: &SharedPanel) -> Vec<PathBuf> {
    let s = state.borrow();
    let bitset = s.selection_model.selection();
    let mut paths = Vec::new();

    let count = bitset.size() as u32;
    for i in 0..count {
        let pos = bitset.nth(i);
        if let Some(item) = s.sort_model.item(pos) {
            if let Ok(fi) = item.downcast::<FileItem>() {
                paths.push(PathBuf::from(fi.path()));
            }
        }
    }
    paths
}

pub fn get_selected_names(state: &SharedPanel) -> Vec<String> {
    let s = state.borrow();
    let bitset = s.selection_model.selection();
    let mut names = Vec::new();

    let count = bitset.size() as u32;
    for i in 0..count {
        let pos = bitset.nth(i);
        if let Some(item) = s.sort_model.item(pos) {
            if let Ok(fi) = item.downcast::<FileItem>() {
                names.push(fi.name());
            }
        }
    }
    names
}

// --- Context menu handlers ---

fn ctx_open(state: &SharedPanel) {
    let path = get_selected_path(state);
    if let Some(path) = path {
        if path.is_dir() {
            navigate_to(state, &path);
        } else {
            crate::archive::browse::try_open_archive(state, &path);
        }
    }
}

fn ctx_open_default(state: &SharedPanel) {
    let path = get_selected_path(state);
    if let Some(path) = path {
        if path.is_file() {
            let uri = format!("file://{}", path.display());
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
        }
    }
}

fn ctx_copy(state: &SharedPanel) {
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let count = paths.len();
    let names = get_selected_names(state);
    crate::clipboard::set(crate::clipboard::ClipboardData {
        paths,
        is_cut: false,
    });
    let s = state.borrow();
    let msg = if count == 1 {
        format!("{} copied to clipboard", names[0])
    } else {
        format!("{} items copied to clipboard", count)
    };
    s.status_label.set_label(&msg);
}

fn ctx_move(state: &SharedPanel) {
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let count = paths.len();
    let names = get_selected_names(state);
    let all_in_archive = paths.iter().all(|p| {
        crate::archive::browse::parse_archive_path(p).is_some()
    });
    let is_cut = !all_in_archive;
    crate::clipboard::set(crate::clipboard::ClipboardData {
        paths,
        is_cut,
    });
    let s = state.borrow();
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
}

fn ctx_delete(state: &SharedPanel) {
    let names = get_selected_names(state);
    if names.is_empty() {
        return;
    }
    let current = { state.borrow().current_path.clone() };
    if crate::archive::browse::parse_archive_path(&current).is_some() {
        crate::utils::show_error("Cannot Delete", "Cannot modify files inside an archive");
        return;
    }
    let count = names.len();
    let msg = if count == 1 {
        format!("Delete \"{}\"?", names[0])
    } else {
        format!("Delete {} items?", count)
    };
    let dialog = adw::AlertDialog::builder()
        .heading("Confirm Delete")
        .body(&msg)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let s = state.clone();
    let current = { state.borrow().current_path.clone() };
    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            let s2 = s.clone();
            let c = current.clone();
            let n = names.clone();
            glib::spawn_future_local(async move {
                for name in &n {
                    let path = c.join(name);
                    if let Err(e) = crate::operations::delete::delete_entry(&path).await {
                        crate::utils::show_error("Delete Failed", &e);
                    }
                }
                load_directory(&s2);
            });
        }
    });
    dialog.present(crate::utils::parent_window().as_ref());
}

fn ctx_rename(state: &SharedPanel) {
    let path = match get_selected_path(state) {
        Some(p) if !p.file_name().map_or(false, |n| n == "..") => p,
        _ => return,
    };
    if crate::archive::browse::parse_archive_path(&path).is_some() {
        crate::utils::show_error("Cannot Rename", "Cannot rename files inside an archive");
        return;
    }
    let old_name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let dialog = adw::AlertDialog::builder()
        .heading("Rename")
        .body("Enter new name:")
        .build();
    let entry = gtk::Entry::builder()
        .text(&old_name)
        .hexpand(true)
        .build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");

    let s = state.clone();
    let parent = path.parent().unwrap_or(&path).to_path_buf();
    dialog.connect_response(None, move |_, response| {
        if response == "rename" {
            let new_name = entry.text().to_string();
            if !new_name.is_empty() && new_name != old_name {
                let old_path = parent.join(&old_name);
                let new_path = parent.join(&new_name);
                if let Err(e) = std::fs::rename(&old_path, &new_path) {
                    crate::utils::show_error("Rename Failed", &e.to_string());
                }
                load_directory(&s);
            }
        }
    });
    dialog.present(crate::utils::parent_window().as_ref());
}

fn ctx_paste(state: &SharedPanel) {
    let cb = crate::clipboard::get();
    if cb.paths.is_empty() {
        return;
    }
    let current = { state.borrow().current_path.clone() };
    let s = state.clone();
    glib::spawn_future_local(async move {
        let mut skip_existing = false;
        for path in &cb.paths {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            let dest = current.join(&name);

            if dest.exists() && !skip_existing {
                match confirm_overwrite(&name).await {
                    0 => {}
                    1 => continue,
                    2 => { skip_existing = true; continue; }
                    _ => break,
                }
            }

            if let Err(e) = crate::operations::copy::copy_file(path, &dest).await {
                crate::utils::show_error("Paste Failed", &e);
            }
        }
        if cb.is_cut {
            for path in &cb.paths {
                if crate::archive::browse::parse_archive_path(path).is_some() {
                    continue;
                }
                if path.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(path) {
                        crate::utils::show_error("Delete Failed", &e.to_string());
                    }
                } else if let Err(e) = std::fs::remove_file(path) {
                    crate::utils::show_error("Delete Failed", &e.to_string());
                }
            }
            crate::clipboard::set(crate::clipboard::ClipboardData {
                paths: Vec::new(),
                is_cut: false,
            });
        }
        load_directory(&s);
    });
}

async fn confirm_overwrite(name: &str) -> u8 {
    let (tx, rx) = async_channel::bounded::<u8>(1);
    let dialog = adw::AlertDialog::builder()
        .heading("File Already Exists")
        .body(&format!("\"{}\" already exists.\nOverwrite?", name))
        .build();
    dialog.add_response("skip", "Skip");
    dialog.add_response("skip_all", "Skip All");
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("overwrite", "Overwrite");
    dialog.set_response_appearance("overwrite", adw::ResponseAppearance::Suggested);
    dialog.set_response_appearance("cancel", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |_, response| {
        let code = match response {
            "overwrite" => 0,
            "skip" => 1,
            "skip_all" => 2,
            _ => 3,
        };
        let _ = tx.try_send(code);
    });
    dialog.present(crate::utils::parent_window().as_ref());
    rx.recv().await.unwrap_or(3)
}

fn ctx_create_archive(state: &SharedPanel) {
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    crate::dialogs::create_archive::show(state, &paths);
}

fn ctx_extract_here(state: &SharedPanel) {
    let path = get_selected_path(state);
    if let Some(path) = path {
        let password = state.borrow().current_password.clone();
        let (archive, output_dir) =
            if let Some((archive_path, _)) = crate::archive::browse::parse_archive_path(&path) {
                (archive_path.clone(), archive_path.parent().unwrap_or(&archive_path).to_path_buf())
            } else if path.is_file() {
                (path.clone(), state.borrow().current_path.clone())
            } else {
                return;
            };
        let s = state.clone();
        let archive_name = archive.file_name().unwrap_or_default().to_string_lossy().to_string();
        glib::spawn_future_local(async move {
            let options = crate::archive::extractor::ExtractOptions {
                output_dir: output_dir.clone(),
                full_paths: true,
                overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                password,
            };
            match crate::archive::extractor::extract_archive(&archive, &options, None, None, None).await {
                Ok(_) => load_directory(&s),
                Err(e) if e == "__NEED_PASSWORD__" => {
                    if let Some(password) = crate::archive::browse::prompt_for_password(&archive_name).await {
                        let options = crate::archive::extractor::ExtractOptions {
                            output_dir,
                            full_paths: true,
                            overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                            password: Some(password),
                        };
                        match crate::archive::extractor::extract_archive(&archive, &options, None, None, None).await {
                            Ok(_) => load_directory(&s),
                            Err(e) => crate::utils::show_error("Extract Failed", &e),
                        }
                    }
                }
                Err(e) => crate::utils::show_error("Extract Failed", &e),
            }
        });
    }
}

fn ctx_extract_to(state: &SharedPanel) {
    let password = state.borrow().current_password.clone();
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let path = &paths[0];
    let archive = if let Some((archive_path, _)) =
        crate::archive::browse::parse_archive_path(path)
    {
        archive_path
    } else {
        path.clone()
    };
    let s = state.clone();
    let archive_name = archive.file_name().unwrap_or_default().to_string_lossy().to_string();
    glib::idle_add_local_once(move || {
        let dialog = gtk::FileDialog::builder()
            .title("Extract To...")
            .accept_label("Extract")
            .build();
        dialog.select_folder(None::<&gtk::Window>, None::<&gio::Cancellable>, move |result| {
            if let Ok(dest_dir) = result {
                if let Some(dest_path) = dest_dir.path() {
                    let s2 = s.clone();
                    let archive = archive.clone();
                    let archive_name = archive_name.clone();
                    let output = dest_path.to_path_buf();
                    let pw = password.clone();
                    glib::spawn_future_local(async move {
                        let options = crate::archive::extractor::ExtractOptions {
                            output_dir: output.clone(),
                            full_paths: true,
                            overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                            password: pw,
                        };
                        match crate::archive::extractor::extract_archive(&archive, &options, None, None, None).await {
                            Ok(_) => load_directory(&s2),
                            Err(e) if e == "__NEED_PASSWORD__" => {
                                if let Some(password) = crate::archive::browse::prompt_for_password(&archive_name).await {
                                    let options = crate::archive::extractor::ExtractOptions {
                                        output_dir: output,
                                        full_paths: true,
                                        overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                                        password: Some(password),
                                    };
                                    match crate::archive::extractor::extract_archive(&archive, &options, None, None, None).await {
                                        Ok(_) => load_directory(&s2),
                                        Err(e) => crate::utils::show_error("Extract Failed", &e),
                                    }
                                }
                            }
                            Err(e) => crate::utils::show_error("Extract Failed", &e),
                        }
                    });
                }
            }
        });
    });
}

fn ctx_test_archive(state: &SharedPanel) {
    let path = get_selected_path(state);
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
        glib::spawn_future_local(async move {
            match crate::archive::tester::test_archive(&archive).await {
                    Ok(_) => crate::utils::show_info("Archive Test", "Archive integrity OK"),
                    Err(e) => crate::utils::show_error("Archive Test Failed", &e),
                }
            });
        }
    }

fn ctx_new_folder(state: &SharedPanel) {
    let current = { state.borrow().current_path.clone() };
    if crate::archive::browse::parse_archive_path(&current).is_some() {
        crate::utils::show_error("New Folder", "Cannot create folder inside an archive");
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading("New Folder")
        .body("Enter folder name:")
        .build();
    let entry = gtk::Entry::builder()
        .placeholder_text("New Folder")
        .hexpand(true)
        .build();
    entry.set_text("New Folder");
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");

    let s = state.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "create" {
            let name = entry.text().to_string();
            if !name.is_empty() {
                let path = current.join(&name);
                let s2 = s.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = crate::operations::mkdir::create_directory(&path).await {
                        crate::utils::show_error("Create Folder Failed", &e);
                    }
                    load_directory(&s2);
                });
            }
        }
    });
    dialog.present(crate::utils::parent_window().as_ref());
}

fn ctx_add_bookmark(state: &SharedPanel) {
    let path = get_selected_path(state);
    let path = match path {
        Some(p) if p.is_dir() => p,
        _ => {
            let s = state.borrow();
            s.current_path.clone()
        }
    };
    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Bookmark")
        .to_string();

    let dialog = adw::AlertDialog::builder()
        .heading("Add Bookmark")
        .body("Enter bookmark name:")
        .build();
    let entry = gtk::Entry::builder()
        .text(&name)
        .hexpand(true)
        .build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add");

    dialog.connect_response(None, move |_, response| {
        if response == "add" {
            let bm_name = entry.text().to_string();
            if !bm_name.is_empty() {
                crate::config::bookmarks::add_bookmark(&bm_name, &path.to_string_lossy());
            }
        }
    });
    dialog.present(crate::utils::parent_window().as_ref());
}

fn ctx_properties(state: &SharedPanel) {
    let paths = get_all_selected_paths(state);
    crate::dialogs::properties::show(&paths);
}

fn is_archive_file(path: &Path) -> bool {
    is_archive_file_check(path)
}

pub fn is_archive_file_check(path: &Path) -> bool {
    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    name.ends_with(".7z")
        || name.ends_with(".zip")
        || name.ends_with(".tar")
        || name.ends_with(".gz")
        || name.ends_with(".bz2")
        || name.ends_with(".xz")
        || name.ends_with(".rar")
        || name.ends_with(".tgz")
        || name.ends_with(".tbz2")
        || name.ends_with(".tbz")
        || name.ends_with(".txz")
        || name.ends_with(".zst")
        || name.ends_with(".lz4")
        || name.ends_with(".lzma")
        || name.ends_with(".arj")
        || name.ends_with(".cab")
        || name.ends_with(".chm")
        || name.ends_with(".cpio")
        || name.ends_with(".deb")
        || name.ends_with(".dmg")
        || name.ends_with(".iso")
        || name.ends_with(".lzh")
        || name.ends_with(".lha")
        || name.ends_with(".rpm")
        || name.ends_with(".squashfs")
        || name.ends_with(".vhd")
        || name.ends_with(".vmdk")
        || name.ends_with(".wim")
        || name.ends_with(".xar")
        || name.ends_with(".z")
        || name.ends_with(".taz")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tar.xz")
        || name.ends_with(".tar.zst")
        || name.ends_with(".tar.lz4")
        || name.ends_with(".tar.lzma")
}

fn setup_columns(column_view: &gtk::ColumnView, state: &crate::panels::SharedPanel) {
    // Name column
    let name_factory = gtk::SignalListItemFactory::new();
    let panel_state = state.clone();
    name_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = gtk::Image::new();
        icon.set_pixel_size(16);
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .hexpand(true)
            .build();
        hbox.append(&icon);
        hbox.append(&label);
        item.set_child(Some(&hbox));

        // Drag source — initiate drag from this item, dragging all selected files
        let drag_source = gtk::DragSource::builder()
            .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
            .build();
        let ps = panel_state.clone();
        drag_source.connect_prepare(move |_ds, _x, _y| {
            let s = ps.borrow();
            let bitset = s.selection_model.selection();
            if bitset.is_empty() {
                return None;
            }
            let count = bitset.size() as u32;
            let mut uri_list = String::new();
            for i in 0..count {
                let pos = bitset.nth(i);
                if let Some(item) = s.sort_model.item(pos) {
                    if let Ok(fi) = item.downcast::<FileItem>() {
                        let path_str = fi.path();
                        if !path_str.is_empty() {
                            if !uri_list.is_empty() {
                                uri_list.push_str("\r\n");
                            }
                            uri_list.push_str(&format!("file://{}", path_str));
                        }
                    }
                } else {
                    break;
                }
            }
            if uri_list.is_empty() {
                return None;
            }
            let bytes = glib::Bytes::from(uri_list.as_bytes());
            Some(gdk::ContentProvider::for_bytes("text/uri-list", &bytes))
        });
        hbox.add_controller(drag_source);

        // Drop target — receive drops on this item (copy into folder rows)
        let item_drop_formats = gdk::ContentFormatsBuilder::new()
            .add_type(gdk::FileList::static_type())
            .add_type(glib::types::Type::STRING)
            .build();
        let drop_target_on_item = gtk::DropTarget::builder()
            .formats(&item_drop_formats)
            .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
            .build();
        let ps_for_item_drop = panel_state.clone();
        let ps_for_item_accept = panel_state.clone();
        drop_target_on_item.connect_accept(move |_, _drop| {
            let s = ps_for_item_accept.borrow();
            crate::archive::browse::parse_archive_path(&s.current_path).is_none()
        });
        drop_target_on_item.connect_drop(move |ds, value, _x, _y| {
            let widget = match ds.widget() {
                Some(w) => w,
                None => return false,
            };
            let is_dir: bool = unsafe { widget.data::<bool>("item-is-dir") }
                .map(|v| unsafe { *v.as_ref() })
                .unwrap_or(false);
            let item_path: Option<String> = unsafe { widget.data::<String>("item-path") }
                .map(|v| unsafe { v.as_ref().clone() });
            let item_dir = if is_dir { item_path.map(std::path::PathBuf::from) } else { None };

            let current = ps_for_item_drop.borrow().current_path.clone();
            let target_path = item_dir.unwrap_or(current);

            let paths: Vec<std::path::PathBuf> = if let Ok(file_list) = value.get::<gdk::FileList>() {
                file_list.files().iter().filter_map(|f| f.path()).collect()
            } else if let Ok(text) = value.get::<String>() {
                text.lines()
                    .filter_map(|line| {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            return None;
                        }
                        glib::filename_from_uri(line).ok().map(|(p, _)| p)
                    })
                    .collect()
            } else {
                return false;
            };
            if paths.is_empty() {
                return false;
            }

            let s3 = ps_for_item_drop.clone();
            let is_ctrl = ds.current_drop()
                .is_some_and(|d| d.device().modifier_state().contains(gdk::ModifierType::CONTROL_MASK));
            let is_move = !is_ctrl;
            glib::spawn_future_local(async move {
                let mut skip_existing = false;
                for path in &paths {
                    if let Some(name) = path.file_name() {
                        let dest = target_path.join(name);
                        if dest.exists() && !skip_existing {
                            match confirm_overwrite(&name.to_string_lossy()).await {
                                0 => {}
                                1 => continue,
                                2 => { skip_existing = true; continue; }
                                _ => break,
                            }
                        }
                        if is_move {
                            if let Err(e) = crate::operations::move_move::move_file(path, &dest).await {
                                crate::utils::show_error("Drop Failed", &e);
                            }
                        } else if let Err(e) = crate::operations::copy::copy_file(path, &dest).await {
                            crate::utils::show_error("Drop Failed", &e);
                        }
                    }
                }
                crate::panels::load_directory(&s3);
            });
            true
        });
        hbox.add_controller(drop_target_on_item);
    });
    name_factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let file_item = item.item().and_downcast::<FileItem>().unwrap();
        let hbox = item.child().and_downcast::<gtk::Box>().unwrap();
        let icon = hbox.first_child().and_downcast::<gtk::Image>().unwrap();
        let label = hbox.last_child().and_downcast::<gtk::Label>().unwrap();
        let icon_name = icon_for_file(&file_item.name(), file_item.is_dir());
        icon.set_icon_name(Some(icon_name));
        label.set_label(&file_item.name());
        unsafe { hbox.set_data("item-path", file_item.path()) };
        unsafe { hbox.set_data("item-is-dir", file_item.is_dir()) };
        if file_item.is_dir() {
            label.add_css_class("accent");
        } else {
            label.remove_css_class("accent");
        }
    });
    let name_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.name().to_lowercase().cmp(&b.name().to_lowercase()).into()
    });
    let name_col = gtk::ColumnViewColumn::new(Some("Name"), Some(name_factory));
    name_col.set_expand(true);
    name_col.set_fixed_width(300);
    name_col.set_resizable(true);
    name_col.set_sorter(Some(&name_sorter));
    column_view.append_column(&name_col);

    // Size column
    let size_factory = gtk::SignalListItemFactory::new();
    size_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(1.0).build();
        item.set_child(Some(&label));
    });
    size_factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let file_item = item.item().and_downcast::<FileItem>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        if file_item.is_dir() {
            label.set_label("");
        } else {
            label.set_label(&file_item.size_display());
        }
    });
    let size_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.size().cmp(&b.size()).into()
    });
    let size_col = gtk::ColumnViewColumn::new(Some("Size"), Some(size_factory));
    size_col.set_fixed_width(100);
    size_col.set_resizable(true);
    size_col.set_sorter(Some(&size_sorter));
    column_view.append_column(&size_col);

    // Modified column
    let date_factory = gtk::SignalListItemFactory::new();
    date_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).build();
        item.set_child(Some(&label));
    });
    date_factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let file_item = item.item().and_downcast::<FileItem>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        label.set_label(&file_item.modified_display());
    });
    let date_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.modified().cmp(&b.modified()).into()
    });
    let date_col = gtk::ColumnViewColumn::new(Some("Modified"), Some(date_factory));
    date_col.set_fixed_width(160);
    date_col.set_resizable(true);
    date_col.set_sorter(Some(&date_sorter));
    column_view.append_column(&date_col);

    // Created column
    let created_factory = gtk::SignalListItemFactory::new();
    created_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).build();
        item.set_child(Some(&label));
    });
    created_factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let file_item = item.item().and_downcast::<FileItem>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        label.set_label(&file_item.created_display());
    });
    let created_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.created().cmp(&b.created()).into()
    });
    let created_col = gtk::ColumnViewColumn::new(Some("Created"), Some(created_factory));
    created_col.set_fixed_width(160);
    created_col.set_resizable(true);
    created_col.set_sorter(Some(&created_sorter));
    column_view.append_column(&created_col);

    // Accessed column
    let accessed_factory = gtk::SignalListItemFactory::new();
    accessed_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).build();
        item.set_child(Some(&label));
    });
    accessed_factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let file_item = item.item().and_downcast::<FileItem>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        label.set_label(&file_item.accessed_display());
    });
    let accessed_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.accessed().cmp(&b.accessed()).into()
    });
    let accessed_col = gtk::ColumnViewColumn::new(Some("Accessed"), Some(accessed_factory));
    accessed_col.set_fixed_width(160);
    accessed_col.set_resizable(true);
    accessed_col.set_sorter(Some(&accessed_sorter));
    column_view.append_column(&accessed_col);

    // Type column
    let type_factory = gtk::SignalListItemFactory::new();
    type_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).build();
        item.set_child(Some(&label));
    });
    type_factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let file_item = item.item().and_downcast::<FileItem>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        label.set_label(&file_item.file_type());
    });
    let type_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.file_type().to_lowercase().cmp(&b.file_type().to_lowercase()).into()
    });
    let type_col = gtk::ColumnViewColumn::new(Some("Type"), Some(type_factory));
    type_col.set_fixed_width(100);
    type_col.set_resizable(true);
    type_col.set_sorter(Some(&type_sorter));
    column_view.append_column(&type_col);
}

// --- Navigation ---

pub fn navigate_to(state: &SharedPanel, path: &Path) {
    let new_path = path.to_path_buf();
    {
        let mut s = state.borrow_mut();
        let idx = s.history_index;
        s.history.truncate(idx + 1);
        s.history.push(new_path.clone());
        s.history_index = s.history.len() - 1;
        s.current_path = new_path;
    }
    load_directory(state);
}

fn go_back(state: &SharedPanel) {
    let can_go = { state.borrow().history_index > 0 };
    if can_go {
        {
            let mut s = state.borrow_mut();
            s.history_index -= 1;
            s.current_path = s.history[s.history_index].clone();
        }
        load_directory(state);
    }
}

fn go_forward(state: &SharedPanel) {
    let can_go = {
        let s = state.borrow();
        s.history_index < s.history.len() - 1
    };
    if can_go {
        {
            let mut s = state.borrow_mut();
            s.history_index += 1;
            s.current_path = s.history[s.history_index].clone();
        }
        load_directory(state);
    }
}

fn go_up(state: &SharedPanel) {
    let parent = {
        let s = state.borrow();
        s.current_path.parent().map(|p| p.to_path_buf())
    };
    if let Some(parent) = parent {
        navigate_to(state, &parent);
    }
}

pub fn on_activate(state: &SharedPanel, position: u32) {
    let info = {
        let s = state.borrow();
        s.sort_model
            .item(position)
            .and_then(|item| item.downcast::<FileItem>().ok())
            .map(|fi| (fi.path(), fi.is_dir()))
    };
    if let Some((path_str, is_dir)) = info {
        let path = PathBuf::from(&path_str);
        if is_dir {
            if path_str == ".." {
                let cur = state.borrow().current_path.clone();
                if let Some((archive_path, _)) = crate::archive::browse::parse_archive_path(&cur) {
                    if let Some(parent) = archive_path.parent() {
                        navigate_to(state, parent);
                        return;
                    }
                }
            }
            navigate_to(state, &path);
        } else if is_archive_file(&path) {
            crate::archive::browse::try_open_archive(state, &path);
        } else if let Some((archive, internal)) =
            crate::archive::browse::parse_archive_path(&path)
        {
            open_archive_entry(state, &archive, &internal);
        } else {
            let uri = format!("file://{}", path.display());
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
        }
    }
}

fn open_archive_entry(state: &SharedPanel, archive: &Path, internal: &str) {
    let archive = archive.to_path_buf();
    let internal = internal.to_string();
    let archive_name = archive.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive")
        .to_string();
    let mut stored_password = state.borrow().current_password.clone();
    let s = state.clone();
    glib::spawn_future_local(async move {
        let tmp = std::env::temp_dir().join("sevenzip-gui-open");
        let _ = std::fs::create_dir_all(&tmp);
        let name = internal.rsplit('/').next().unwrap_or(&internal).to_string();
        let dest = tmp.join(&name);

        loop {
            let _ = std::fs::remove_file(&dest);

            let pw = stored_password.as_deref();
            let result = crate::archive::extractor::extract_entry(
                &archive, &internal, &tmp, pw,
            ).await;

            match result {
                Ok(()) => {
                    break;
                }
                Err(e) if e == "__NEED_PASSWORD__" => {
                    match crate::archive::browse::prompt_for_password(&archive_name).await {
                        Some(password) => {
                            s.borrow_mut().current_password = Some(password.clone());
                            stored_password = Some(password);
                        }
                        None => return,
                    }
                }
                Err(e) => {
                    crate::utils::show_error("Open Failed", &e);
                    return;
                }
            }
        }

        if !dest.exists() || dest.metadata().map_or(true, |m| m.len() == 0) {
            crate::utils::show_error("Open Failed", "Extracted file not found or empty");
            return;
        }

        let content_type = gio::content_type_guess(Some(&name), None).0;
        if let Some(app_info) = gio::AppInfo::default_for_type(&content_type, false) {
            let files = [gio::File::for_path(&dest)];
            if let Err(e) = app_info.launch(&files, None::<&gio::AppLaunchContext>) {
                crate::utils::show_error("Open Failed", &e.to_string());
            } else {
                let d = dest.clone();
                glib::timeout_add_seconds_once(10, move || { let _ = std::fs::remove_file(&d); });
            }
        } else {
            let uri = format!("file://{}", dest.display());
            if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
                crate::utils::show_error("Open Failed", &e.to_string());
            } else {
                let d = dest.clone();
                glib::timeout_add_seconds_once(10, move || { let _ = std::fs::remove_file(&d); });
            }
        }
    });
}

pub fn load_directory(state: &SharedPanel) {
    let s = state.borrow();
    let current_path = s.current_path.clone();

    if crate::archive::browse::parse_archive_path(&current_path).is_some() {
        return;
    }

    let raw_store = s.raw_store.clone();
    let show_hidden = s.show_hidden.get();

    raw_store.remove_all();

    let mut entries: Vec<FileItem> = Vec::new();

    if let Some(parent) = current_path.parent() {
        entries.push(FileItem::new(
            "..",
            &parent.to_string_lossy(),
            true,
            0,
            0,
            0,
            0,
            "Directory",
        ));
    }

    if let Ok(read_dir) = std::fs::read_dir(&current_path) {
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }

            let metadata = entry.metadata().ok();
            let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());
            let size = metadata.as_ref().map_or(0, |m| m.len());
            let modified = metadata
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let created = metadata
                .as_ref()
                .and_then(|m| m.created().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let accessed = metadata
                .as_ref()
                .and_then(|m| m.accessed().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let file_type = if is_dir {
                String::from("Directory")
            } else {
                name.rsplit('.')
                    .next()
                    .map(|e| format!(".{}", e))
                    .unwrap_or_default()
            };

            entries.push(FileItem::new(
                &name,
                &entry.path().to_string_lossy(),
                is_dir,
                size,
                modified,
                created,
                accessed,
                &file_type,
            ));
        }
    }

    for item in &entries {
        raw_store.append(item);
    }

    let count = entries.iter().filter(|e| e.name() != "..").count();

    s.path_entry.set_text(&current_path.to_string_lossy());
    s.status_label.set_label(&format!("{} items", count));
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    glob_rec(&p, &t)
}

fn glob_rec(p: &[char], t: &[char]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some('*') => {
            let rest = &p[1..];
            glob_rec(rest, t) || (!t.is_empty() && glob_rec(p, &t[1..]))
        }
        Some('?') => !t.is_empty() && glob_rec(&p[1..], &t[1..]),
        Some(pc) => !t.is_empty() && *pc == t[0] && glob_rec(&p[1..], &t[1..]),
    }
}
