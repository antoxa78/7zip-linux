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
    pub archive_entries: Vec<crate::archive::lister::ArchiveEntry>,
    pub archive_virtual_root: String,
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
        archive_entries: Vec::new(),
        archive_virtual_root: String::new(),
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

    let _s_for_drop = state.clone();
    drop_target.connect_accept(move |_, _drop| true);

    let s_for_drop2 = state.clone();
    drop_target.connect_drop(move |ds, value, _x, _y| {
        let (archive_path, in_archive, archive_pw, internal_prefix) = {
            let s = s_for_drop2.borrow();
            let cur = s.current_path.clone();
            let in_arc = crate::archive::browse::parse_archive_path(&cur).is_some();
            let (arc_path, pw, prefix) = if in_arc {
                let (p, _) = crate::archive::browse::parse_archive_path(&cur)
                    .unwrap_or((cur.clone(), String::new()));
                let vroot = s.archive_virtual_root.clone();
                let pref = if cur.to_string_lossy().starts_with(&vroot) {
                    cur.to_string_lossy()[vroot.len()..].trim_start_matches('/').trim_end_matches('/').to_string()
                } else {
                    String::new()
                };
                (p, s.current_password.clone(), pref)
            } else {
                (cur.clone(), None, String::new())
            };
            (arc_path, in_arc, pw, prefix)
        };
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
            if in_archive {
                let (tx, rx) = async_channel::bounded::<u8>(32);
                {
                    let sb = s3.borrow();
                    sb.progress_bar.set_visible(true);
                    sb.progress_bar.set_fraction(0.0);
                    sb.progress_bar.set_text(Some("0%"));
                    sb.status_label.set_label("Adding files...");
                }

                let refs: Vec<&std::path::Path> = paths.iter().map(|pb| pb.as_path()).collect();
                let s3_progress = s3.clone();
                let add_fut = crate::archive::creator::add_files_into_archive_path(
                    &archive_path, &refs, &internal_prefix, archive_pw.as_deref(), Some(tx),
                );
                let progress_fut = async move {
                    while let Ok(pct) = rx.recv().await {
                        let sb = s3_progress.borrow();
                        sb.progress_bar.set_fraction(pct as f64 / 100.0);
                        sb.progress_bar.set_text(Some(&format!("{}%", pct)));
                        sb.status_label.set_label(&format!("Adding files... {}%", pct));
                    }
                };

                let (add_result, _) = tokio::join!(add_fut, progress_fut);

                {
                    let sb = s3.borrow();
                    sb.progress_bar.set_visible(false);
                }
                if let Err(e) = add_result {
                    crate::utils::show_error("Drop Failed", &e);
                }
                if is_move {
                    let drag_tmp = std::env::temp_dir().join("sevenzip-gui-drag");
                    for path in &paths {
                        if let Ok(rel) = path.strip_prefix(&drag_tmp) {
                            let original = rel.to_string_lossy().to_string();
                            if let Some(name) = path.file_name() {
                                let new_path = if internal_prefix.is_empty() {
                                    name.to_string_lossy().to_string()
                                } else {
                                    format!("{}/{}", internal_prefix, name.to_string_lossy())
                                };
                                if original != new_path {
                                    if let Err(e) = crate::archive::creator::delete_entry_from_archive(
                                        &archive_path, &original, archive_pw.as_deref(),
                                    ).await {
                                        crate::utils::show_error("Delete Failed", &e);
                                    }
                                }
                            }
                        }
                    }
                }
                match crate::archive::lister::list_archive_with_password(
                    &archive_path, archive_pw.as_deref(),
                ).await {
                    Ok(entries) => {
                        s3.borrow_mut().archive_entries = entries;
                    }
                    Err(e) => {
                        crate::utils::show_error("Refresh Failed", &e);
                    }
                }
                crate::panels::load_directory(&s3);
            } else {
                let mut skip_existing = false;
                for path in &paths {
                    if let Some(name) = path.file_name() {
                        let dest = archive_path.join(name);
                        if dest.exists() && !skip_existing {
                            match confirm_overwrite(&name.to_string_lossy()).await {
                                0 => {
                                    if dest.is_dir() {
                                        let _ = std::fs::remove_dir_all(&dest);
                                    } else {
                                        let _ = std::fs::remove_file(&dest);
                                    }
                                }
                                1 => continue,
                                2 => { skip_existing = true; continue; }
                                _ => break,
                            }
                        }
                        if is_move {
                            if let Err(e) = crate::operations::move_move::move_file(path, &dest, None).await {
                                crate::utils::show_error("Drop Failed", &e);
                            }
                        } else if let Err(e) = crate::operations::copy::copy_file(path, &dest, None).await {
                            crate::utils::show_error("Drop Failed", &e);
                        }
                    }
                }
                crate::panels::load_directory(&s3);
            }
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
    open_section.append(Some("Open With..."), Some("ctx.open-with"));
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
    archive_section.append(Some("Add to Archive..."), Some("ctx.add-to-archive"));
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
    register_ctx_action(&action_group, "open-with", &state, ctx_open_with);
    register_ctx_action(&action_group, "copy", &state, ctx_copy);
    register_ctx_action(&action_group, "move", &state, ctx_move);
    register_ctx_action(&action_group, "delete", &state, ctx_delete);
    register_ctx_action(&action_group, "rename", &state, ctx_rename);
    register_ctx_action(&action_group, "paste", &state, ctx_paste);
    register_ctx_action(&action_group, "create-archive", &state, ctx_create_archive);
    register_ctx_action(&action_group, "add-to-archive", &state, ctx_add_to_archive);
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

fn ctx_open_with(state: &SharedPanel) {
    let path = match get_selected_path(state) {
        Some(p) => p,
        _ => return,
    };

    if path.is_dir() {
        return;
    }

    let (real_path, is_archive_file) = if let Some((archive, internal)) = crate::archive::browse::parse_archive_path(&path) {
        let password = { state.borrow().current_password.clone() };
        match extract_to_temp(&archive, &internal, password.as_deref()) {
            Some(tmp) => (tmp, true),
            None => {
                crate::utils::show_error("Open With", "Failed to extract file from archive.");
                return;
            }
        }
    } else {
        (path.clone(), false)
    };

    let file = gio::File::for_path(&real_path);
    let mime_str = file
        .query_info("standard::content-type", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>)
        .ok()
        .and_then(|info| info.content_type())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let guess = gio::content_type_guess(Some(real_path.as_path()), None::<&[u8]>);
            guess.0.to_string()
        });
    let apps = gio::AppInfo::all_for_type(&mime_str);
    if apps.is_empty() {
        crate::utils::show_error("Open With", "No applications found for this file type.");
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading("Open With...")
        .build();
    let listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list"])
        .build();
    for app in &apps {
        let row = adw::ActionRow::builder()
            .title(app.display_name())
            .activatable(true)
            .build();
        if let Some(icon) = app.icon() {
            if let Some(icon_name) = icon.to_string() {
                row.set_icon_name(Some(&icon_name));
            }
        }
        listbox.append(&row);
    }
    dialog.set_extra_child(Some(&listbox));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("open", "Open");
    dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
    let path_clone = real_path.clone();
    let apps_clone = apps.clone();
    dialog.connect_response(Some("open"), move |d, _| {
        if let Some(row) = d.extra_child().and_then(|w| {
            w.downcast_ref::<gtk::ListBox>()
                .and_then(|lb| lb.selected_row())
        }) {
            let idx = row.index() as usize;
            if let Some(app) = apps_clone.get(idx) {
                let file = gio::File::for_path(&path_clone);
                let _ = app.launch(&[file], None::<&gio::AppLaunchContext>);
            }
        }
    });
    dialog.present(crate::utils::parent_window().as_ref());
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
    crate::clipboard::set(crate::clipboard::ClipboardData {
        paths,
        is_cut: true,
    });
    let s = state.borrow();
    let msg = if count == 1 {
        format!("{} cut to clipboard", names[0])
    } else {
        format!("{} items cut to clipboard", count)
    };
    s.status_label.set_label(&msg);
}

fn ctx_delete(state: &SharedPanel) {
    let names = get_selected_names(state);
    if names.is_empty() {
        return;
    }
    let current = { state.borrow().current_path.clone() };
    let archive_info = crate::archive::browse::parse_archive_path(&current)
        .map(|(archive_path, _)| {
            let pw = state.borrow().current_password.clone();
            (archive_path, pw)
        });
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
            if let Some((archive_path, password)) = &archive_info {
                let ap = archive_path.clone();
                let pw = password.clone();
                glib::spawn_future_local(async move {
                    let vr = { s2.borrow().archive_virtual_root.clone() };
                    let cur_str = c.to_string_lossy().to_string();
                    let internal_prefix = cur_str[vr.len()..].trim_start_matches('/').to_string();
                    for name in &n {
                        let internal = if internal_prefix.is_empty() {
                            name.clone()
                        } else {
                            format!("{}/{}", internal_prefix, name)
                        };
                        if let Err(e) = crate::archive::creator::delete_entry_from_archive(
                            &ap, &internal, pw.as_deref(),
                        ).await {
                            crate::utils::show_error("Delete Failed", &e);
                        }
                    }
                    match crate::archive::lister::list_archive_with_password(
                        &ap, pw.as_deref(),
                    ).await {
                        Ok(entries) => {
                            s2.borrow_mut().archive_entries = entries;
                        }
                        Err(_) => {}
                    }
                    load_directory(&s2);
                });
            } else {
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
        }
    });
    dialog.present(crate::utils::parent_window().as_ref());
}

fn ctx_rename(state: &SharedPanel) {
    let path = match get_selected_path(state) {
        Some(p) if !p.file_name().map_or(false, |n| n == "..") => p,
        _ => return,
    };
    let archive_info = crate::archive::browse::parse_archive_path(&path)
        .map(|(archive_path, internal)| {
            let pw = state.borrow().current_password.clone();
            (archive_path, internal, pw)
        });
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

    if let Some((archive_path, internal, password)) = archive_info {
        let s = state.clone();
        let ap = archive_path.clone();
        let int = internal.clone();
        let pw = password.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "rename" {
                let new_name = entry.text().to_string();
                if !new_name.is_empty() && new_name != old_name {
                    let s2 = s.clone();
                    let ap2 = ap.clone();
                    let pw2 = pw.clone();
                    let int2 = int.clone();
                    let n = new_name.clone();
                    glib::spawn_future_local(async move {
                        if let Err(e) = crate::archive::creator::rename_entry_in_archive(
                            &ap2, &int2, &n, pw2.as_deref(),
                        ).await {
                            crate::utils::show_error("Rename Failed", &e);
                        }
                        match crate::archive::lister::list_archive_with_password(
                            &ap2, pw2.as_deref(),
                        ).await {
                            Ok(entries) => {
                                s2.borrow_mut().archive_entries = entries;
                            }
                            Err(_) => {}
                        }
                        load_directory(&s2);
                    });
                }
            }
        });
    } else {
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
    }
    dialog.present(crate::utils::parent_window().as_ref());
}

fn ctx_paste(state: &SharedPanel) {
    let cb = crate::clipboard::get();
    if cb.paths.is_empty() {
        return;
    }
    let current = { state.borrow().current_path.clone() };
    let pw = { state.borrow().current_password.clone() };
    let dest_archive_info = crate::archive::browse::parse_archive_path(&current)
        .map(|(archive_path, internal_prefix)| {
            let pw = state.borrow().current_password.clone();
            (archive_path, internal_prefix, pw)
        });
    let s = state.clone();
    let total = cb.paths.len();
    glib::spawn_future_local(async move {
        {
            let sb = s.borrow();
            sb.progress_bar.set_visible(true);
            sb.progress_bar.set_fraction(0.0);
            sb.progress_bar.set_text(Some("0%"));
            sb.status_label.set_label("Pasting files...");
        }
        let mut done = 0usize;

        if let Some((dest_archive, ref internal_prefix, ref dest_pw)) = dest_archive_info {
            for path in &cb.paths {
                let name = match path.file_name() {
                    Some(n) => n.to_string_lossy().to_string(),
                    None => continue,
                };
                {
                    let pct = ((done as f64 / total as f64) * 100.0) as u32;
                    let sb = s.borrow();
                    sb.progress_bar.set_fraction(pct as f64 / 100.0);
                    sb.progress_bar.set_text(Some(&format!("{}%", pct)));
                    sb.status_label.set_label(&format!("Pasting file {} of {}...", done + 1, total));
                }

                if let Some((src_archive, src_internal)) = crate::archive::browse::parse_archive_path(path) {
                    let tmp = std::env::temp_dir().join("sevenzip-gui-paste");
                    let _ = std::fs::create_dir_all(&tmp);
                    let extract_dir = tmp.join(&name);
                    let _ = std::fs::remove_dir_all(&extract_dir);
                    if let Err(e) = crate::archive::extractor::extract_entry(
                        &src_archive, &src_internal, &tmp, dest_pw.as_deref(),
                    ).await {
                        crate::utils::show_error("Paste Failed", &e);
                        done += 1;
                        continue;
                    }
                    let source_path = if src_internal.ends_with('/') || src_internal.contains('/') {
                        let nested = tmp.join(&name);
                        if nested.exists() { nested } else { tmp.join(name.rsplit('/').next().unwrap_or(&name)) }
                    } else {
                        tmp.join(&name)
                    };
                    let refs = vec![source_path.as_path()];
                    if let Err(e) = crate::archive::creator::add_files_into_archive_path(
                        &dest_archive, &refs, internal_prefix, dest_pw.as_deref(), None,
                    ).await {
                        crate::utils::show_error("Paste Failed", &e);
                    }
                    let _ = std::fs::remove_dir_all(&tmp);
                } else {
                    let refs = vec![path.as_path()];
                    if let Err(e) = crate::archive::creator::add_files_into_archive_path(
                        &dest_archive, &refs, internal_prefix, dest_pw.as_deref(), None,
                    ).await {
                        crate::utils::show_error("Paste Failed", &e);
                    }
                }
                done += 1;
            }

            if cb.is_cut {
                for path in &cb.paths {
                    if let Some((src_archive, src_internal)) = crate::archive::browse::parse_archive_path(path) {
                        if let Err(e) = crate::archive::creator::delete_entry_from_archive(
                            &src_archive, &src_internal, dest_pw.as_deref(),
                        ).await {
                            crate::utils::show_error("Delete Failed", &e);
                        }
                    } else {
                        if path.is_dir() {
                            let _ = std::fs::remove_dir_all(path);
                        } else {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
                crate::clipboard::set(crate::clipboard::ClipboardData {
                    paths: Vec::new(),
                    is_cut: false,
                });
            }

            match crate::archive::lister::list_archive_with_password(
                &dest_archive, dest_pw.as_deref(),
            ).await {
                Ok(entries) => {
                    s.borrow_mut().archive_entries = entries;
                }
                Err(_) => {}
            }
        } else {
            let mut skip_existing = false;
            for path in &cb.paths {
                let name = match path.file_name() {
                    Some(n) => n.to_string_lossy().to_string(),
                    None => continue,
                };
                let dest = current.join(&name);

                if dest.exists() && !skip_existing {
                    match confirm_overwrite(&name).await {
                        0 => {
                            if dest.is_dir() {
                                let _ = std::fs::remove_dir_all(&dest);
                            } else {
                                let _ = std::fs::remove_file(&dest);
                            }
                        }
                        1 => { done += 1; continue; }
                        2 => { skip_existing = true; done += 1; continue; }
                        _ => break,
                    }
                }

                {
                    let pct = ((done as f64 / total as f64) * 100.0) as u32;
                    let sb = s.borrow();
                    sb.progress_bar.set_fraction(pct as f64 / 100.0);
                    sb.progress_bar.set_text(Some(&format!("{}%", pct)));
                    sb.status_label.set_label(&format!("Pasting file {} of {}...", done + 1, total));
                }

                if let Err(e) = crate::operations::copy::copy_file(path, &dest, pw.as_deref()).await {
                    crate::utils::show_error("Paste Failed", &e);
                }
                done += 1;
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
        }

        {
            let sb = s.borrow();
            sb.progress_bar.set_visible(false);
            sb.status_label.set_label("");
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

fn ctx_add_to_archive(state: &SharedPanel) {
    let target_archive = {
        let selected = get_selected_path(state);
        selected.or_else(|| {
            let s = state.borrow();
            let cur = s.current_path.to_string_lossy().to_string();
            if cur.contains(" [archive]") {
                crate::archive::browse::parse_archive_path(&s.current_path)
                    .map(|(archive_path, _)| archive_path)
            } else {
                None
            }
        })
    };
    let target_archive = match target_archive {
        Some(p) if p.is_file() => p,
        _ => {
            crate::utils::show_error("Add to Archive", "Select an archive file first, or browse inside an archive.");
            return;
        }
    };

    let dialog = gtk::FileDialog::builder()
        .title("Select Files to Add")
        .accept_label("Add")
        .build();

    let s = state.clone();
    let archive = target_archive.clone();
    dialog.open_multiple(None::<&gtk::Window>, None::<&gio::Cancellable>, move |result| {
        if let Ok(files) = result {
            let n = files.n_items();
            let mut file_paths = Vec::new();
            for i in 0..n {
                if let Some(item) = files.item(i) {
                    if let Ok(f) = item.downcast::<gio::File>() {
                        if let Some(path) = f.path() {
                            file_paths.push(path);
                        }
                    }
                }
            }
            if file_paths.is_empty() {
                return;
            }
            let s2 = s.clone();
            let archive2 = archive.clone();
            let pw = { s2.borrow().current_password.clone() };
            glib::spawn_future_local(async move {
                {
                    let sb = s2.borrow();
                    sb.status_label.set_label("Adding files to archive...");
                    sb.progress_bar.set_visible(true);
                    sb.progress_bar.pulse();
                }
                let refs: Vec<&std::path::Path> = file_paths.iter().map(|pb| pb.as_path()).collect();
                let result = crate::archive::creator::add_to_archive(
                    &archive2, &refs, pw.as_deref(),
                ).await;
                {
                    let sb = s2.borrow();
                    sb.progress_bar.set_visible(false);
                }
                match result {
                    Ok(_) => {
                        let pw2 = pw.clone();
                        let archive3 = archive2.clone();
                        let s3 = s2.clone();
                        glib::spawn_future_local(async move {
                            match crate::archive::lister::list_archive_with_password(
                                &archive3, pw2.as_deref(),
                            ).await {
                                Ok(entries) => {
                                    s3.borrow_mut().archive_entries = entries;
                                }
                                Err(_) => {}
                            }
                            load_directory(&s3);
                        });
                    }
                    Err(e) => {
                        crate::utils::show_error("Add to Archive Failed", &e);
                    }
                }
            });
        }
    });
}

fn ctx_extract_here(state: &SharedPanel) {
    let password = state.borrow().current_password.clone();
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let s = state.clone();
    let archive_name = {
        let first = &paths[0];
        if let Some((archive_path, _)) = crate::archive::browse::parse_archive_path(first) {
            archive_path.file_name().unwrap_or_default().to_string_lossy().to_string()
        } else {
            first.file_name().unwrap_or_default().to_string_lossy().to_string()
        }
    };
    glib::spawn_future_local(async move {
        let mut entries_to_extract: Vec<(PathBuf, String)> = Vec::new();
        let mut top_level_archives: Vec<PathBuf> = Vec::new();
        for p in &paths {
            if let Some((archive_path, internal)) = crate::archive::browse::parse_archive_path(p) {
                entries_to_extract.push((archive_path, internal));
            } else if p.is_file() {
                top_level_archives.push(p.clone());
            }
        }
        for (archive, internal) in &entries_to_extract {
            let output_dir = archive.parent().unwrap_or(archive).to_path_buf();
            let mut result = crate::archive::extractor::extract_entry(
                archive, internal, &output_dir, password.as_deref(),
            ).await;
            if let Err(ref e) = result {
                if e == "__NEED_PASSWORD__" {
                    if let Some(pw) = crate::archive::browse::prompt_for_password(&archive_name).await {
                        result = crate::archive::extractor::extract_entry(
                            archive, internal, &output_dir, Some(&pw),
                        ).await;
                    }
                }
            }
            if let Err(e) = result {
                crate::utils::show_error("Extract Failed", &e);
            }
        }
        for archive in &top_level_archives {
            let output_dir = s.borrow().current_path.clone();
            let options = crate::archive::extractor::ExtractOptions {
                output_dir: output_dir.clone(),
                full_paths: true,
                overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                password: password.clone(),
            };
            let mut result = crate::archive::extractor::extract_archive(
                archive, &options, None, None, None,
            ).await;
            if let Err(ref e) = result {
                if e == "__NEED_PASSWORD__" {
                    if let Some(pw) = crate::archive::browse::prompt_for_password(&archive_name).await {
                        let options = crate::archive::extractor::ExtractOptions {
                            output_dir,
                            full_paths: true,
                            overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                            password: Some(pw),
                        };
                        result = crate::archive::extractor::extract_archive(
                            archive, &options, None, None, None,
                        ).await;
                    }
                }
            }
            if let Err(e) = result {
                crate::utils::show_error("Extract Failed", &e);
            }
        }
        load_directory(&s);
    });
}

fn ctx_extract_to(state: &SharedPanel) {
    let password = state.borrow().current_password.clone();
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let mut entries_to_extract: Vec<(PathBuf, String)> = Vec::new();
    let mut top_level_archives: Vec<PathBuf> = Vec::new();
    for p in &paths {
        if let Some((archive_path, internal)) = crate::archive::browse::parse_archive_path(p) {
            entries_to_extract.push((archive_path, internal));
        } else if p.is_file() {
            top_level_archives.push(p.clone());
        }
    }
    let s = state.clone();
    let archive_name = if let Some((ref a, _)) = entries_to_extract.first() {
        a.file_name().unwrap_or_default().to_string_lossy().to_string()
    } else if let Some(first) = top_level_archives.first() {
        first.file_name().unwrap_or_default().to_string_lossy().to_string()
    } else {
        return;
    };
    glib::idle_add_local_once(move || {
        let dialog = gtk::FileDialog::builder()
            .title("Extract To...")
            .accept_label("Extract")
            .build();
        dialog.select_folder(None::<&gtk::Window>, None::<&gio::Cancellable>, move |result| {
            if let Ok(dest_dir) = result {
                if let Some(dest_path) = dest_dir.path() {
                    let s2 = s.clone();
                    let archive_name = archive_name.clone();
                    let output = dest_path.to_path_buf();
                    let pw = password.clone();
                    let entries = entries_to_extract.clone();
                    let archives = top_level_archives.clone();
                    glib::spawn_future_local(async move {
                        for (archive, internal) in &entries {
                            let mut result = crate::archive::extractor::extract_entry(
                                archive, internal, &output, pw.as_deref(),
                            ).await;
                            if let Err(ref e) = result {
                                if e == "__NEED_PASSWORD__" {
                                    if let Some(pw) = crate::archive::browse::prompt_for_password(&archive_name).await {
                                        result = crate::archive::extractor::extract_entry(
                                            archive, internal, &output, Some(&pw),
                                        ).await;
                                    }
                                }
                            }
                            if let Err(e) = result {
                                crate::utils::show_error("Extract Failed", &e);
                            }
                        }
                        for archive in &archives {
                            let options = crate::archive::extractor::ExtractOptions {
                                output_dir: output.clone(),
                                full_paths: true,
                                overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                                password: pw.clone(),
                            };
                            let mut result = crate::archive::extractor::extract_archive(
                                archive, &options, None, None, None,
                            ).await;
                            if let Err(ref e) = result {
                                if e == "__NEED_PASSWORD__" {
                                    if let Some(pw) = crate::archive::browse::prompt_for_password(&archive_name).await {
                                        let options = crate::archive::extractor::ExtractOptions {
                                            output_dir: output.clone(),
                                            full_paths: true,
                                            overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                                            password: Some(pw),
                                        };
                                        result = crate::archive::extractor::extract_archive(
                                            archive, &options, None, None, None,
                                        ).await;
                                    }
                                }
                            }
                            if let Err(e) = result {
                                crate::utils::show_error("Extract Failed", &e);
                            }
                        }
                        load_directory(&s2);
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
    let (current, archive_info) = {
        let s = state.borrow();
        let cur = s.current_path.clone();
        let archive = crate::archive::browse::parse_archive_path(&cur)
            .map(|(p, _)| (p, s.current_password.clone()));
        (cur, archive)
    };
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
    let current2 = current.clone();
    let archive_info2 = archive_info.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "create" {
            let name = entry.text().to_string();
            if !name.is_empty() {
                if let Some((archive_path, password)) = &archive_info2 {
                    let s2 = s.clone();
                    let ap = archive_path.clone();
                    let pw = password.clone();
                    let n = name.clone();
                    glib::spawn_future_local(async move {
                        if let Err(e) = crate::archive::creator::add_directory_to_archive(
                            &ap, &n, pw.as_deref(),
                        ).await {
                            crate::utils::show_error("New Folder", &e);
                        }
                        match crate::archive::lister::list_archive_with_password(
                            &ap, pw.as_deref(),
                        ).await {
                            Ok(entries) => {
                                s2.borrow_mut().archive_entries = entries;
                            }
                            Err(_) => {}
                        }
                        load_directory(&s2);
                    });
                } else {
                    let path = current2.join(&name);
                    let s2 = s.clone();
                    glib::spawn_future_local(async move {
                        if let Err(e) = crate::operations::mkdir::create_directory(&path).await {
                            crate::utils::show_error("Create Folder Failed", &e);
                        }
                        load_directory(&s2);
                    });
                }
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
        drag_source.connect_prepare(move |ds, _x, _y| {
            let s = ps.borrow();

            // Build list of paths to drag: selected items, plus the item under cursor if not selected
            let mut drag_paths: Vec<String> = Vec::new();

            let bitset = s.selection_model.selection();
            let count = bitset.size() as u32;
            for i in 0..count {
                let pos = bitset.nth(i);
                if let Some(item) = s.sort_model.item(pos) {
                    if let Ok(fi) = item.downcast::<FileItem>() {
                        let p = fi.path();
                        if !p.is_empty() {
                            drag_paths.push(p);
                        }
                    }
                }
            }

            // Always ensure the item directly under the cursor is included
            let cursor_path = ds.widget()
                .as_ref()
                .and_then(|w| widget_item_data(w))
                .map(|(path, _)| path);
            if let Some(ref path_ref) = cursor_path {
                if !path_ref.is_empty() && !drag_paths.contains(path_ref) {
                    drag_paths.clear();
                    drag_paths.push(path_ref.clone());
                }
            }
            eprintln!("[DRAG] prepare: cursor={:?}, selection={:?}, drag={:?}", cursor_path, drag_paths.len(), drag_paths);

            if drag_paths.is_empty() {
                return None;
            }

            let mut uri_list = String::new();
            for path_str in &drag_paths {
                let path = PathBuf::from(path_str);
                let real_path = if let Some((archive, internal)) =
                    crate::archive::browse::parse_archive_path(&path)
                {
                    let pw = s.current_password.as_deref();
                    match extract_to_temp(&archive, &internal, pw) {
                        Some(tmp) => tmp,
                        None => path,
                    }
                } else {
                    path
                };
                if !uri_list.is_empty() {
                    uri_list.push_str("\r\n");
                }
                let gfile = gio::File::for_path(&real_path);
                uri_list.push_str(&gfile.uri().to_string());
            }
            if uri_list.is_empty() {
                return None;
            }
            eprintln!("[DRAG] prepare: uri_list={}", uri_list);
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
        let _ps_for_item_accept = panel_state.clone();
        drop_target_on_item.connect_accept(move |_, _drop| true);
        drop_target_on_item.connect_drop(move |ds, value, _x, _y| {
            let (in_archive, archive_path, archive_pw) = {
                let s = ps_for_item_drop.borrow();
                let in_arc = crate::archive::browse::parse_archive_path(&s.current_path).is_some();
                let arc = crate::archive::browse::parse_archive_path(&s.current_path)
                    .map(|(p, _)| p)
                    .unwrap_or(s.current_path.clone());
                let pw = s.current_password.clone();
                (in_arc, arc, pw)
            };

            let widget = match ds.widget() {
                Some(w) => w,
                None => return false,
            };
            let is_dir: bool = widget_item_data(&widget)
                .is_some_and(|(p, is_dir)| is_dir && p != "..");
            let item_path: String = widget_item_data(&widget)
                .map(|(p, _)| p)
                .unwrap_or_default();
            let item_dir = if is_dir { Some(std::path::PathBuf::from(item_path)) } else { None };

            let current = ps_for_item_drop.borrow().current_path.clone();
            let target_path = if in_archive { archive_path.clone() } else { item_dir.unwrap_or(current) };

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
                if in_archive {
                    let refs: Vec<&std::path::Path> = paths.iter().map(|pb| pb.as_path()).collect();
                    let internal_prefix = if is_dir {
                        let item_vpath = widget_item_data(&widget)
                            .map(|(p, _)| p)
                            .unwrap_or_default();
                        let virtual_root = {
                            let s = s3.borrow();
                            s.archive_virtual_root.clone()
                        };
                        if item_vpath.starts_with(&virtual_root) {
                            item_vpath[virtual_root.len()..].trim_start_matches('/').to_string()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    let (tx, rx) = async_channel::bounded::<u8>(32);
                    {
                        let sb = s3.borrow();
                        sb.progress_bar.set_visible(true);
                        sb.progress_bar.set_fraction(0.0);
                        sb.progress_bar.set_text(Some("0%"));
                        sb.status_label.set_label("Adding files...");
                    }
                    let s3_progress = s3.clone();
                    let add_fut = crate::archive::creator::add_files_into_archive_path(
                        &archive_path, &refs, &internal_prefix, archive_pw.as_deref(), Some(tx),
                    );
                    let progress_fut = async move {
                        while let Ok(pct) = rx.recv().await {
                            let sb = s3_progress.borrow();
                            sb.progress_bar.set_fraction(pct as f64 / 100.0);
                            sb.progress_bar.set_text(Some(&format!("{}%", pct)));
                            sb.status_label.set_label(&format!("Adding files... {}%", pct));
                        }
                    };

                    let (add_result, _) = tokio::join!(add_fut, progress_fut);

                    {
                        let sb = s3.borrow();
                        sb.progress_bar.set_visible(false);
                    }
                    if let Err(e) = add_result {
                        crate::utils::show_error("Drop Failed", &e);
                    }
                    if is_move {
                        let drag_tmp = std::env::temp_dir().join("sevenzip-gui-drag");
                        for path in &paths {
                            if let Ok(rel) = path.strip_prefix(&drag_tmp) {
                                let original = rel.to_string_lossy().to_string();
                                if let Some(name) = path.file_name() {
                                    let new_path = if internal_prefix.is_empty() {
                                        name.to_string_lossy().to_string()
                                    } else {
                                        format!("{}/{}", internal_prefix, name.to_string_lossy())
                                    };
                                    if original != new_path {
                                        if let Err(e) = crate::archive::creator::delete_entry_from_archive(
                                            &archive_path, &original, archive_pw.as_deref(),
                                        ).await {
                                            crate::utils::show_error("Delete Failed", &e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    match crate::archive::lister::list_archive_with_password(
                        &archive_path, archive_pw.as_deref(),
                    ).await {
                        Ok(entries) => {
                            s3.borrow_mut().archive_entries = entries;
                        }
                        Err(e) => {
                            crate::utils::show_error("Refresh Failed", &e);
                        }
                    }
                    crate::panels::load_directory(&s3);
                } else {
                    let mut skip_existing = false;
                    for path in &paths {
                        if let Some(name) = path.file_name() {
                            let dest = target_path.join(name);
                            if dest.exists() && !skip_existing {
                                match confirm_overwrite(&name.to_string_lossy()).await {
                                    0 => {
                                        if dest.is_dir() {
                                            let _ = std::fs::remove_dir_all(&dest);
                                        } else {
                                            let _ = std::fs::remove_file(&dest);
                                        }
                                    }
                                    1 => continue,
                                    2 => { skip_existing = true; continue; }
                                    _ => break,
                                }
                            }
                            if is_move {
                                if let Err(e) = crate::operations::move_move::move_file(path, &dest, None).await {
                                    crate::utils::show_error("Drop Failed", &e);
                                }
                            } else if let Err(e) = crate::operations::copy::copy_file(path, &dest, None).await {
                                crate::utils::show_error("Drop Failed", &e);
                            }
                        }
                    }
                    crate::panels::load_directory(&s3);
                }
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
        unsafe {
            hbox.set_data("item-path", file_item.path());
            hbox.set_data("item-is-dir", file_item.is_dir());
        }
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
                if crate::archive::browse::parse_archive_path(&cur).is_some() {
                    go_up(state);
                    return;
                }
            }
            navigate_to(state, &path);
        } else if let Some((archive, internal)) =
            crate::archive::browse::parse_archive_path(&path)
        {
            if internal.is_empty() {
                if is_archive_file(&path) {
                    crate::archive::browse::try_open_archive(state, &path);
                }
                return;
            }
            if is_archive_file(&path) {
                open_archive_inside_archive(state, &archive, &internal);
            } else {
                open_archive_entry(state, &archive, &internal);
            }
        } else if is_archive_file(&path) {
            crate::archive::browse::try_open_archive(state, &path);
        } else {
            let uri = format!("file://{}", path.display());
            let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
        }
    }
}

fn open_archive_inside_archive(state: &SharedPanel, archive: &Path, internal: &str) {
    let archive = archive.to_path_buf();
    let internal = internal.to_string();
    if internal.is_empty() {
        return;
    }
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
        eprintln!("[OPEN_NESTED] archive={}, internal={}, name={}, dest={}", archive.display(), internal, name, dest.display());

        loop {
            let _ = std::fs::remove_file(&dest);

            let pw = stored_password.as_deref();
            let result = crate::archive::extractor::extract_entry(
                &archive, &internal, &tmp, pw,
            ).await;

            match result {
                Ok(()) => break,
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

        eprintln!("[OPEN_NESTED] dest.exists={}", dest.exists());
        if !dest.exists() || dest.metadata().map_or(true, |m| m.len() == 0) {
            crate::utils::show_error("Open Failed", "Extracted archive not found or empty");
            return;
        }

        crate::archive::browse::try_open_archive(&s, &dest);
    });
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
        eprintln!("[OPEN] open_archive_entry: archive={}, internal={}, name={}, dest={}", archive.display(), internal, name, dest.display());

        loop {
            let _ = std::fs::remove_file(&dest);

            let pw = stored_password.as_deref();
            eprintln!("[OPEN] extract_entry: pw={:?}", pw.is_some());
            let result = crate::archive::extractor::extract_entry(
                &archive, &internal, &tmp, pw,
            ).await;

            match result {
                Ok(()) => {
                    eprintln!("[OPEN] extract_entry Ok, dest.exists={}", dest.exists());
                    break;
                }
                Err(e) if e == "__NEED_PASSWORD__" => {
                    eprintln!("[OPEN] extract_entry needs password");
                    match crate::archive::browse::prompt_for_password(&archive_name).await {
                        Some(password) => {
                            s.borrow_mut().current_password = Some(password.clone());
                            stored_password = Some(password);
                        }
                        None => return,
                    }
                }
                Err(e) => {
                    eprintln!("[OPEN] extract_entry error: {}", e);
                    crate::utils::show_error("Open Failed", &e);
                    return;
                }
            }
        }

        if !dest.exists() || dest.metadata().map_or(true, |m| m.len() == 0) {
            eprintln!("[OPEN] FAILED: dest.exists={}, dest={}", dest.exists(), dest.display());
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
        let virtual_root = s.archive_virtual_root.clone();
        let archive_entries = s.archive_entries.clone();
        let show_hidden = s.show_hidden.get();
        let raw_store = s.raw_store.clone();
        let path_entry = s.path_entry.clone();
        let status_label = s.status_label.clone();
        drop(s);

        let internal_prefix = current_path.to_string_lossy()
            [virtual_root.len()..]
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();

        raw_store.remove_all();

        let parent_item = FileItem::new("..", "..", true, 0, 0, 0, 0, "Directory");
        raw_store.append(&parent_item);

        let mut count = 0usize;
        for entry in &archive_entries {
            let entry_name = entry.name.trim_end_matches('/');
            if internal_prefix.is_empty() {
                if entry_name.contains('/') {
                    continue;
                }
            } else {
                if entry_name == internal_prefix {
                    continue;
                }
                let prefix_sep = format!("{}/", internal_prefix);
                if !entry_name.starts_with(&*prefix_sep) {
                    continue;
                }
                let rest = &entry_name[prefix_sep.len()..];
                if rest.contains('/') {
                    continue;
                }
            }

            let display_name = entry_name.rsplit('/').next().unwrap_or(entry_name).to_string();
            if !show_hidden && display_name.starts_with('.') {
                continue;
            }

            let full_virtual = format!("{}/{}", virtual_root, entry.name);
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
            raw_store.append(&item);
            count += 1;
        }

        path_entry.set_text(&format!("{}:{}/", current_path
            .to_string_lossy()
            .split(" [archive]")
            .next()
            .unwrap_or(""), internal_prefix));
        status_label.set_label(&format!("{} items (in archive)", count));
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

fn extract_to_temp(archive: &Path, internal: &str, password: Option<&str>) -> Option<PathBuf> {
    let tmp = std::env::temp_dir().join("sevenzip-gui-drag");
    let _ = std::fs::create_dir_all(&tmp);

    let dest = tmp.join(internal);
    if dest.exists() {
        return Some(dest);
    }

    let mut cmd = std::process::Command::new("7z");
    cmd.arg("x")
        .arg(archive)
        .arg(internal)
        .arg(format!("-o{}", tmp.display()))
        .arg("-y");
    if let Some(pw) = password {
        cmd.arg(format!("-p{}", pw));
    }
    let output = cmd.stdin(std::process::Stdio::null()).output().ok()?;
    if output.status.success() && dest.exists() {
        Some(dest)
    } else {
        None
    }
}

/// Retrieves (path, is_dir) metadata previously stored via WidgetExt::set_data.
/// The unsafe block is sound because data is only accessed while the widget is
/// alive (during drag/drop UI events), and the types match what was stored.
fn widget_item_data(widget: &gtk::Widget) -> Option<(String, bool)> {
    let path = unsafe { widget.data::<String>("item-path")? };
    let is_dir = unsafe { widget.data::<bool>("item-is-dir")? };
    Some((unsafe { path.as_ref() }.clone(), unsafe { *is_dir.as_ref() }))
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
