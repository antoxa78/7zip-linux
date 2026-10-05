use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio};

use crate::models::FileItem;

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
    /// Passwords the user entered, per archive file. `current_password` is always the
    /// entry for the archive at `archive_virtual_root` (or None), never a leftover.
    pub archive_passwords: HashMap<PathBuf, String>,
    /// Header bar title; shows the current folder (or archive) name and its location.
    pub window_title: Option<adw::WindowTitle>,
    /// Shown over the list when the folder is empty or the filter matches nothing.
    pub empty_page: adw::StatusPage,
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
            // Keep the ".." row so you can always navigate up while filtering.
            return fi.name() == ".." || glob_match(&pat, &fi.name());
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
    let status_label = gtk::Label::new(Some(""));
    status_label.set_xalign(0.0);
    status_label.set_hexpand(true);
    status_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    status_label.set_css_classes(&["dim-label", "status-label"]);

    // Search entry
    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Filter files..."));

    let progress_bar = gtk::ProgressBar::builder()
        .visible(false)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::End)
        .width_request(220)
        .build();
    progress_bar.add_css_class("status-progress");

    let empty_page = adw::StatusPage::builder()
        .icon_name("folder-symbolic")
        .title("This folder is empty")
        .can_target(false)
        .visible(false)
        .build();
    empty_page.add_css_class("compact");
    empty_page.add_css_class("dim-label");

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
        archive_passwords: HashMap::new(),
        window_title: None,
        empty_page: empty_page.clone(),
    }));

    setup_columns(&column_view, &state);

    // Sort: ".." always on top, then folders, then files. The clicked column only
    // orders items *within* those groups, so flipping a column never sinks folders
    // below files or moves ".." to the bottom.
    let group_sorter = gtk::CustomSorter::new(|a, b| {
        let rank = |o: &glib::Object| -> u8 {
            match o.downcast_ref::<FileItem>() {
                Some(fi) if fi.name() == ".." => 0,
                Some(fi) if fi.is_dir() => 1,
                _ => 2,
            }
        };
        rank(a).cmp(&rank(b)).into()
    });
    let multi_sorter = gtk::MultiSorter::new();
    multi_sorter.append(group_sorter);
    if let Some(cv_sorter) = column_view.sorter() {
        multi_sorter.append(cv_sorter);
    }
    sort_model.set_sorter(Some(&multi_sorter));
    if let Some(first) = column_view.columns().item(0).and_downcast::<gtk::ColumnViewColumn>() {
        column_view.sort_by_column(Some(&first), gtk::SortType::Ascending);
    }

    // Navigation bar
    let nav_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    nav_bar.add_css_class("path-bar");

    let back_button = gtk::Button::from_icon_name("go-previous-symbolic");
    back_button.set_tooltip_text(Some("Back (Alt+Left)"));
    back_button.add_css_class("flat");
    let s = state.clone();
    back_button.connect_clicked(move |_| go_back(&s));
    nav_bar.append(&back_button);

    let forward_button = gtk::Button::from_icon_name("go-next-symbolic");
    forward_button.set_tooltip_text(Some("Forward (Alt+Right)"));
    forward_button.add_css_class("flat");
    let s = state.clone();
    forward_button.connect_clicked(move |_| go_forward(&s));
    nav_bar.append(&forward_button);

    let up_button = gtk::Button::from_icon_name("go-up-symbolic");
    up_button.set_tooltip_text(Some("Parent folder (Alt+Up)"));
    up_button.add_css_class("flat");
    let s = state.clone();
    up_button.connect_clicked(move |_| go_up(&s));
    nav_bar.append(&up_button);

    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Reload (Ctrl+R)"));
    refresh_button.add_css_class("flat");
    let s = state.clone();
    refresh_button.connect_clicked(move |_| load_directory(&s));
    nav_bar.append(&refresh_button);

    let s = state.clone();
    path_entry.connect_activate(move |entry| {
        let text = entry.text().trim().to_string();
        let path = expand_home(&text);
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
    let list_overlay = gtk::Overlay::new();
    list_overlay.set_child(Some(&scrolled));
    list_overlay.add_overlay(&empty_page);
    container.append(&list_overlay);

    let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bottom_bar.add_css_class("statusbar");
    bottom_bar.append(&status_label);
    bottom_bar.append(&progress_bar);
    container.append(&bottom_bar);

    // Keep the status line and empty-state message in sync with filtering.
    let s = state.clone();
    sort_model.connect_items_changed(move |_, _, _, _| {
        if let Ok(sb) = s.try_borrow() {
            drop(sb);
            update_status(&s);
        }
    });

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
        let (archive_path, in_archive, archive_pw, internal_prefix) =
            match current_archive_location(&s_for_drop2) {
                Some((arc, prefix)) => {
                    let pw = password_for_archive(&s_for_drop2, &arc);
                    (arc, true, pw, prefix)
                }
                None => (s_for_drop2.borrow().current_path.clone(), false, None, String::new()),
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
            if in_archive && crate::archive::creator::is_read_only_archive(&archive_path) {
                crate::utils::show_error(
                    "Drop Failed",
                    &format!(
                        "This archive format is read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead."
                    ),
                );
                return;
            }
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
                } else if is_move {
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
                // Same engine as paste: no copying onto itself, no deleting the source
                // before it has been copied, overwrite only after confirmation.
                transfer_items(&s3, paths, None, is_move, TransferDest::Dir(archive_path)).await;
            }
        });
        true
    });
    container.add_controller(drop_target);

    // Selection changed -> update status bar
    let s = state.clone();
    selection.connect_selection_changed(move |_, _, _| {
        update_status(&s);
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

/// The ".." row is a navigation aid, not a real item. It must never be part of a
/// selection that gets deleted, copied, moved or archived (Ctrl+A selects it too).
fn is_parent_row(fi: &FileItem) -> bool {
    fi.name() == ".."
}

/// All selected items except the ".." row, in view order.
fn selected_items(state: &SharedPanel) -> Vec<FileItem> {
    let s = state.borrow();
    let bitset = s.selection_model.selection();
    let mut items = Vec::new();
    let count = bitset.size() as u32;
    for i in 0..count {
        let pos = bitset.nth(i);
        if let Some(item) = s.sort_model.item(pos) {
            if let Ok(fi) = item.downcast::<FileItem>() {
                if !is_parent_row(&fi) {
                    items.push(fi);
                }
            }
        }
    }
    items
}

pub fn get_selected_path(state: &SharedPanel) -> Option<PathBuf> {
    selected_items(state).first().map(|fi| PathBuf::from(fi.path()))
}

pub fn get_all_selected_paths(state: &SharedPanel) -> Vec<PathBuf> {
    selected_items(state).iter().map(|fi| PathBuf::from(fi.path())).collect()
}

pub fn get_selected_names(state: &SharedPanel) -> Vec<String> {
    selected_items(state).iter().map(|fi| fi.name()).collect()
}

// --- Archive location / password helpers ---

/// If the panel is showing the inside of an archive: (archive file, folder inside it).
pub fn current_archive_location(state: &SharedPanel) -> Option<(PathBuf, String)> {
    let cur = state.borrow().current_path.clone();
    crate::archive::browse::parse_archive_path(&cur)
        .map(|(a, internal)| (a, internal.trim_matches('/').to_string()))
}

/// The archive whose entries are currently loaded in `archive_entries`.
pub fn loaded_archive(state: &SharedPanel) -> Option<PathBuf> {
    let root = state.borrow().archive_virtual_root.clone();
    if root.is_empty() {
        return None;
    }
    crate::archive::browse::parse_archive_path(Path::new(&root)).map(|(a, _)| a)
}

/// The password we know for `archive` — never a password that belongs to another archive.
pub fn password_for_archive(state: &SharedPanel, archive: &Path) -> Option<String> {
    if let Some(pw) = state.borrow().archive_passwords.get(archive) {
        return Some(pw.clone());
    }
    if loaded_archive(state).as_deref() == Some(archive) {
        return state.borrow().current_password.clone();
    }
    None
}

/// Records a password the user entered for `archive`.
pub fn remember_password(state: &SharedPanel, archive: &Path, password: &str) {
    let is_loaded = loaded_archive(state).as_deref() == Some(archive);
    let mut s = state.borrow_mut();
    s.archive_passwords.insert(archive.to_path_buf(), password.to_string());
    if is_loaded {
        s.current_password = Some(password.to_string());
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

fn join_internal(prefix: &str, name: &str) -> String {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", prefix, name)
    }
}

/// Re-reads the archive shown in the panel (after it was modified) and redraws.
pub async fn refresh_current_archive_and_reload(state: &SharedPanel) {
    if let Some((archive, _)) = current_archive_location(state) {
        if loaded_archive(state).as_deref() == Some(archive.as_path()) {
            let pw = password_for_archive(state, &archive);
            match crate::archive::lister::list_archive_with_password(&archive, pw.as_deref()).await {
                Ok(entries) => state.borrow_mut().archive_entries = entries,
                Err(e) => crate::utils::show_error("Refresh Failed", &e),
            }
        }
    }
    load_directory(state);
}

/// Extracts an archive entry, asking for the password (again) if 7z needs one.
async fn extract_entry_with_prompt(
    state: &SharedPanel,
    archive: &Path,
    internal: &str,
    dest_dir: &Path,
    password: &mut Option<String>,
) -> Result<(), String> {
    let archive_name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    loop {
        match crate::archive::extractor::extract_entry(archive, internal, dest_dir, password.as_deref()).await {
            Err(e) if e == crate::utils::NEED_PASSWORD => {
                match crate::archive::browse::prompt_for_password_retry(&archive_name, password.is_some()).await {
                    Some(pw) => {
                        remember_password(state, archive, &pw);
                        *password = Some(pw);
                    }
                    None => return Err(e),
                }
            }
            other => return other,
        }
    }
}

// --- Copy / cut / paste / move (one implementation for toolbar, keys, menu and drops) ---

pub enum TransferDest {
    /// A real folder on disk.
    Dir(PathBuf),
    /// A folder inside an archive.
    Archive { archive: PathBuf, prefix: String, password: Option<String> },
}

fn set_progress(state: &SharedPanel, done: usize, total: usize, verb: &str) {
    let pct = if total == 0 { 0 } else { ((done as f64 / total as f64) * 100.0) as u32 };
    let sb = state.borrow();
    sb.progress_bar.set_fraction(pct as f64 / 100.0);
    sb.progress_bar.set_text(Some(&format!("{}%", pct)));
    sb.status_label.set_label(&format!("{} {} of {}...", verb, (done + 1).min(total), total));
}

/// Copies (or moves, if `is_move`) `sources` into `dest`.
///
/// Guarantees:
/// * an item is never copied onto itself (a copy in the same folder becomes "name (copy)"),
///   and moving an item to where it already is does nothing;
/// * a folder is never copied/moved into itself;
/// * an existing destination is only replaced after the user confirms;
/// * for a move, a source is deleted only after *its own* copy succeeded — skipped,
///   cancelled and failed items are left untouched.
///
/// Returns the sources that were not transferred.
pub async fn transfer_items(
    state: &SharedPanel,
    sources: Vec<PathBuf>,
    src_password: Option<String>,
    is_move: bool,
    dest: TransferDest,
) -> Vec<PathBuf> {
    let total = sources.len();
    let verb = if is_move { "Moving" } else { "Copying" };
    {
        let sb = state.borrow();
        sb.progress_bar.set_visible(true);
        sb.progress_bar.set_fraction(0.0);
        sb.progress_bar.set_text(Some("0%"));
        sb.status_label.set_label(&format!("{} files...", verb));
    }

    let mut src_password = src_password;
    let mut not_done: Vec<PathBuf> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut skip_existing = false;
    let mut cancelled = false;

    for (i, src) in sources.iter().enumerate() {
        if cancelled {
            not_done.push(src.clone());
            continue;
        }
        set_progress(state, i, total, verb);

        let name = match src.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => {
                not_done.push(src.clone());
                continue;
            }
        };
        let src_arc = match crate::archive::browse::parse_archive_path(src) {
            Some((a, internal)) => {
                let internal = internal.trim_matches('/').to_string();
                if internal.is_empty() {
                    errors.push(format!("{}: cannot copy an archive root", name));
                    not_done.push(src.clone());
                    continue;
                }
                Some((a, internal))
            }
            None => None,
        };

        let result: Result<(), String> = match &dest {
            TransferDest::Dir(dir) => {
                let mut target = dir.join(&name);
                if src_arc.is_none() {
                    if *src == target || same_file(src, &target) {
                        if is_move {
                            continue; // already there — nothing to do, nothing to delete
                        }
                        target = crate::utils::fsops::unique_copy_name(dir, &name, src.is_dir());
                    } else if src.is_dir() && crate::utils::fsops::is_same_or_inside(dir, src) {
                        errors.push(format!("{}: cannot {} a folder into itself", name, if is_move { "move" } else { "copy" }));
                        not_done.push(src.clone());
                        continue;
                    }
                }
                if std::fs::symlink_metadata(&target).is_ok() {
                    if skip_existing {
                        not_done.push(src.clone());
                        continue;
                    }
                    match confirm_overwrite(&name).await {
                        0 => {}
                        1 => { not_done.push(src.clone()); continue; }
                        2 => { skip_existing = true; not_done.push(src.clone()); continue; }
                        _ => { cancelled = true; not_done.push(src.clone()); continue; }
                    }
                }
                match &src_arc {
                    Some((arc, internal)) => {
                        let r = extract_entry_with_prompt(state, arc, internal, dir, &mut src_password).await;
                        if r.is_ok() && is_move {
                            if let Err(e) = crate::archive::creator::delete_entry_from_archive(
                                arc, internal, src_password.as_deref(),
                            ).await {
                                errors.push(format!("{}: copied, but could not be removed from the archive: {}", name, crate::utils::humanize_error(&e)));
                            }
                        }
                        r
                    }
                    None if is_move => crate::operations::move_move::move_file(src, &target, None).await,
                    None => crate::operations::copy::copy_file(src, &target, None).await,
                }
            }
            TransferDest::Archive { archive, prefix, password } => {
                let dest_internal = join_internal(prefix, &name);
                let same_archive_src = src_arc
                    .as_ref()
                    .filter(|(a, _)| same_file(a, archive))
                    .map(|(_, internal)| internal.clone());
                if let Some(ref si) = same_archive_src {
                    if *si == dest_internal {
                        continue; // pasted onto itself — nothing to do
                    }
                    if dest_internal.starts_with(&format!("{}/", si)) {
                        errors.push(format!("{}: cannot {} a folder into itself", name, if is_move { "move" } else { "copy" }));
                        not_done.push(src.clone());
                        continue;
                    }
                }
                let exists = loaded_archive(state).is_some_and(|a| same_file(&a, archive))
                    && state.borrow().archive_entries.iter()
                        .any(|e| e.name.trim_matches('/') == dest_internal);
                if exists {
                    if skip_existing {
                        not_done.push(src.clone());
                        continue;
                    }
                    match confirm_overwrite(&name).await {
                        0 => {}
                        1 => { not_done.push(src.clone()); continue; }
                        2 => { skip_existing = true; not_done.push(src.clone()); continue; }
                        _ => { cancelled = true; not_done.push(src.clone()); continue; }
                    }
                }
                if let (Some(si), true) = (&same_archive_src, is_move) {
                    // Move inside one archive: a rename, no extract/re-add needed.
                    crate::archive::creator::move_entry_in_archive(archive, si, &dest_internal, password.as_deref()).await
                } else if let Some((arc, internal)) = &src_arc {
                    match crate::utils::unique_temp_dir("paste") {
                        Err(e) => Err(format!("Failed to create temp dir: {}", e)),
                        Ok(tmp) => {
                            let mut r = extract_entry_with_prompt(state, arc, internal, &tmp, &mut src_password).await;
                            if r.is_ok() {
                                let staged = tmp.join(&name);
                                r = crate::archive::creator::add_files_into_archive_path(
                                    archive, &[staged.as_path()], prefix, password.as_deref(), None,
                                ).await;
                            }
                            let _ = std::fs::remove_dir_all(&tmp);
                            if r.is_ok() && is_move {
                                if let Err(e) = crate::archive::creator::delete_entry_from_archive(
                                    arc, internal, src_password.as_deref(),
                                ).await {
                                    errors.push(format!("{}: copied, but could not be removed from the source archive: {}", name, crate::utils::humanize_error(&e)));
                                }
                            }
                            r
                        }
                    }
                } else {
                    let r = crate::archive::creator::add_files_into_archive_path(
                        archive, &[src.as_path()], prefix, password.as_deref(), None,
                    ).await;
                    if r.is_ok() && is_move {
                        if let Err(e) = crate::utils::fsops::remove_path(src) {
                            errors.push(format!("{}: added to the archive, but the original could not be deleted: {}", name, e));
                        }
                    }
                    r
                }
            }
        };

        if let Err(e) = result {
            errors.push(format!("{}: {}", name, crate::utils::humanize_error(&e)));
            not_done.push(src.clone());
        }
    }

    {
        let sb = state.borrow();
        sb.progress_bar.set_visible(false);
        sb.status_label.set_label("");
    }
    if !errors.is_empty() {
        crate::utils::show_error(
            if is_move { "Move Failed" } else { "Copy Failed" },
            &errors.join("\n"),
        );
    }
    refresh_current_archive_and_reload(state).await;
    not_done
}

/// Puts the selection on the app clipboard (Copy / Cut).
pub fn copy_selection(state: &SharedPanel, is_cut: bool) {
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let password = paths
        .iter()
        .find_map(|p| crate::archive::browse::parse_archive_path(p).map(|(a, _)| a))
        .and_then(|a| password_for_archive(state, &a));
    let count = paths.len();
    let first_name = paths[0]
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    crate::clipboard::set(crate::clipboard::ClipboardData { paths, is_cut, password });
    let verb = if is_cut { "cut" } else { "copied" };
    let msg = if count == 1 {
        format!("{} {} to clipboard", first_name, verb)
    } else {
        format!("{} items {} to clipboard", count, verb)
    };
    state.borrow().status_label.set_label(&msg);
}

/// Pastes the app clipboard into the current folder (on disk or inside an archive).
pub fn paste_clipboard(state: &SharedPanel, spinner: Option<gtk::Spinner>) {
    let cb = crate::clipboard::get();
    if cb.paths.is_empty() {
        return;
    }
    let dest = match current_archive_location(state) {
        Some((archive, prefix)) => {
            let password = password_for_archive(state, &archive);
            TransferDest::Archive { archive, prefix, password }
        }
        None => TransferDest::Dir(state.borrow().current_path.clone()),
    };
    let s = state.clone();
    glib::spawn_future_local(async move {
        if let Some(ref sp) = spinner {
            sp.set_spinning(true);
        }
        let remaining = transfer_items(&s, cb.paths.clone(), cb.password.clone(), cb.is_cut, dest).await;
        if cb.is_cut {
            if remaining.is_empty() {
                crate::clipboard::clear();
            } else {
                // Keep what could not be moved so the user can retry.
                crate::clipboard::set(crate::clipboard::ClipboardData {
                    paths: remaining,
                    is_cut: true,
                    password: cb.password.clone(),
                });
            }
        }
        if let Some(ref sp) = spinner {
            sp.set_spinning(false);
        }
    });
}

/// Asks for confirmation and deletes the selection (on disk or inside an archive).
pub fn delete_selection(state: &SharedPanel, spinner: Option<gtk::Spinner>) {
    let names = get_selected_names(state);
    if names.is_empty() {
        return;
    }
    let current = state.borrow().current_path.clone();
    let archive_info = current_archive_location(state)
        .map(|(archive, prefix)| {
            let pw = password_for_archive(state, &archive);
            (archive, prefix, pw)
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
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        let s2 = s.clone();
        let names = names.clone();
        let current = current.clone();
        let archive_info = archive_info.clone();
        let spinner = spinner.clone();
        glib::spawn_future_local(async move {
            if let Some(ref sp) = spinner {
                sp.set_spinning(true);
            }
            let mut errors = Vec::new();
            if let Some((archive, prefix, pw)) = &archive_info {
                for name in &names {
                    let internal = join_internal(prefix, name);
                    if let Err(e) = crate::archive::creator::delete_entry_from_archive(
                        archive, &internal, pw.as_deref(),
                    ).await {
                        errors.push(format!("{}: {}", name, crate::utils::humanize_error(&e)));
                    }
                }
            } else {
                for name in &names {
                    if let Err(e) = crate::operations::delete::delete_entry(&current.join(name)).await {
                        errors.push(format!("{}: {}", name, e));
                    }
                }
            }
            if !errors.is_empty() {
                crate::utils::show_error("Delete Failed", &errors.join("\n"));
            }
            refresh_current_archive_and_reload(&s2).await;
            if let Some(ref sp) = spinner {
                sp.set_spinning(false);
            }
        });
    });
    dialog.present(crate::utils::parent_window().as_ref());
}

/// Asks for a name and creates a folder in the current location
/// (inside the current archive folder when browsing an archive).
pub fn new_folder(state: &SharedPanel, spinner: Option<gtk::Spinner>) {
    let current = state.borrow().current_path.clone();
    let archive_info = current_archive_location(state)
        .map(|(archive, prefix)| {
            let pw = password_for_archive(state, &archive);
            (archive, prefix, pw)
        });
    let dialog = adw::AlertDialog::builder()
        .heading("New Folder")
        .body("Enter folder name:")
        .build();
    let entry = gtk::Entry::builder()
        .placeholder_text("New Folder")
        .hexpand(true)
        .activates_default(true)
        .build();
    entry.set_text("New Folder");
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_default_response(Some("create"));

    let s = state.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "create" {
            return;
        }
        let name = entry.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        if name.contains('/') || name == "." || name == ".." {
            crate::utils::show_error("New Folder", &format!("\"{}\" is not a valid folder name.", name));
            return;
        }
        let s2 = s.clone();
        let current = current.clone();
        let archive_info = archive_info.clone();
        let spinner = spinner.clone();
        glib::spawn_future_local(async move {
            if let Some(ref sp) = spinner {
                sp.set_spinning(true);
            }
            if let Some((archive, prefix, pw)) = &archive_info {
                let internal = join_internal(prefix, &name);
                let exists = s2.borrow().archive_entries.iter()
                    .any(|e| e.name.trim_matches('/') == internal);
                if exists {
                    crate::utils::show_error("New Folder", &format!("\"{}\" already exists.", name));
                } else if let Err(e) = crate::archive::creator::add_directory_to_archive(
                    archive, prefix, &name, pw.as_deref(),
                ).await {
                    crate::utils::show_error("New Folder", &e);
                }
            } else if let Err(e) = crate::operations::mkdir::create_directory(&current.join(&name)).await {
                crate::utils::show_error("Create Folder Failed", &e);
            }
            refresh_current_archive_and_reload(&s2).await;
            if let Some(ref sp) = spinner {
                sp.set_spinning(false);
            }
        });
    });
    dialog.present(crate::utils::parent_window().as_ref());
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
    copy_selection(state, false);
}

fn ctx_move(state: &SharedPanel) {
    copy_selection(state, true);
}

fn ctx_delete(state: &SharedPanel) {
    delete_selection(state, None);
}

pub fn ctx_rename(state: &SharedPanel) {
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
    paste_clipboard(state, None);
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
    crate::dialogs::create_archive::show(state, &paths, false);
}

fn ctx_add_to_archive(state: &SharedPanel) {
    add_to_archive_dialog(state, None);
}

/// "Add to Archive": pick files and add them to the archive being browsed
/// (into the current folder), or to the selected archive file.
pub fn add_to_archive_dialog(state: &SharedPanel, spinner: Option<gtk::Spinner>) {
    let (target_archive, internal_prefix) = match current_archive_location(state) {
        Some((archive, prefix)) => (Some(archive), prefix),
        None => (get_selected_path(state), String::new()),
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
    dialog.open_multiple(crate::utils::parent_window().as_ref(), None::<&gio::Cancellable>, move |result| {
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
            let prefix = internal_prefix.clone();
            let pw = password_for_archive(&s2, &archive2);
            let spinner = spinner.clone();
            glib::spawn_future_local(async move {
                if let Some(ref sp) = spinner {
                    sp.set_spinning(true);
                }
                {
                    let sb = s2.borrow();
                    sb.status_label.set_label("Adding files to archive...");
                    sb.progress_bar.set_visible(true);
                    sb.progress_bar.pulse();
                }
                let refs: Vec<&std::path::Path> = file_paths.iter().map(|pb| pb.as_path()).collect();
                let result = crate::archive::creator::add_files_into_archive_path(
                    &archive2, &refs, &prefix, pw.as_deref(), None,
                ).await;
                {
                    let sb = s2.borrow();
                    sb.progress_bar.set_visible(false);
                    sb.status_label.set_label("");
                }
                if let Err(e) = result {
                    crate::utils::show_error("Add to Archive Failed", &e);
                }
                refresh_current_archive_and_reload(&s2).await;
                if let Some(ref sp) = spinner {
                    sp.set_spinning(false);
                }
            });
        }
    });
}

fn ctx_extract_here(state: &SharedPanel) {
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let s = state.clone();
    glib::spawn_future_local(async move {
        extract_paths(&s, &paths, None).await;
    });
}

fn ctx_extract_to(state: &SharedPanel) {
    let paths = get_all_selected_paths(state);
    if paths.is_empty() {
        return;
    }
    let s = state.clone();
    glib::idle_add_local_once(move || {
        let dialog = gtk::FileDialog::builder()
            .title("Extract To...")
            .accept_label("Extract")
            .build();
        dialog.select_folder(crate::utils::parent_window().as_ref(), None::<&gio::Cancellable>, move |result| {
            if let Ok(dest_dir) = result {
                if let Some(dest_path) = dest_dir.path() {
                    let s2 = s.clone();
                    let paths = paths.clone();
                    glib::spawn_future_local(async move {
                        extract_paths(&s2, &paths, Some(dest_path)).await;
                    });
                }
            }
        });
    });
}

/// Extracts the given items. Entries inside an archive are extracted (folders with their
/// contents) next to the archive, archive files on disk into the current folder —
/// or everything into `output` when given. Each archive uses its own password.
async fn extract_paths(state: &SharedPanel, paths: &[PathBuf], output: Option<PathBuf>) {
    let mut errors = Vec::new();
    for p in paths {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Some((archive, internal)) = crate::archive::browse::parse_archive_path(p) {
            if internal.trim_matches('/').is_empty() {
                continue;
            }
            let out_dir = output.clone().unwrap_or_else(|| {
                archive.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
            });
            let mut pw = password_for_archive(state, &archive);
            if let Err(e) = extract_entry_with_prompt(state, &archive, &internal, &out_dir, &mut pw).await {
                errors.push(format!("{}: {}", name, crate::utils::humanize_error(&e)));
            }
        } else if p.is_file() {
            let out_dir = output.clone().unwrap_or_else(|| state.borrow().current_path.clone());
            let mut pw = password_for_archive(state, p);
            loop {
                let options = crate::archive::extractor::ExtractOptions {
                    output_dir: out_dir.clone(),
                    full_paths: true,
                    overwrite: crate::archive::extractor::OverwriteMode::Overwrite,
                    password: pw.clone(),
                };
                match crate::archive::extractor::extract_archive(p, &options, None, None, None).await {
                    Ok(_) => break,
                    Err(e) if e == crate::utils::NEED_PASSWORD => {
                        match crate::archive::browse::prompt_for_password_retry(&name, pw.is_some()).await {
                            Some(new_pw) => {
                                remember_password(state, p, &new_pw);
                                pw = Some(new_pw);
                            }
                            None => break,
                        }
                    }
                    Err(e) => {
                        errors.push(format!("{}: {}", name, crate::utils::humanize_error(&e)));
                        break;
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        crate::utils::show_error("Extract Failed", &errors.join("\n"));
    }
    load_directory(state);
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
    new_folder(state, None);
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
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
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
                        if !p.is_empty() && !is_parent_row(&fi) {
                            drag_paths.push(p);
                        }
                    }
                }
            }

            // Always ensure the item directly under the cursor is included
            let cursor_path = ds.widget()
                .as_ref()
                .and_then(|w| widget_item_data(w))
                .map(|(path, _)| path)
                .filter(|path| {
                    // The ".." row stores the parent folder (or ".." in archives); never drag it.
                    path != ".." && Some(Path::new(path)) != s.current_path.parent()
                });
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
                        None => {
                            eprintln!("[DRAG] failed to extract {} from archive, skipping", path_str);
                            continue;
                        }
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
            let (in_archive, archive_path, archive_pw) =
                match current_archive_location(&ps_for_item_drop) {
                    Some((arc, _)) => {
                        let pw = password_for_archive(&ps_for_item_drop, &arc);
                        (true, arc, pw)
                    }
                    None => (false, ps_for_item_drop.borrow().current_path.clone(), None),
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
                if in_archive && crate::archive::creator::is_read_only_archive(&archive_path) {
                    crate::utils::show_error(
                        "Drop Failed",
                        "This archive format is read-only and cannot be modified.\nExtract the files, make changes, and repack the archive instead.",
                    );
                    return;
                }
                if in_archive {
                    let refs: Vec<&std::path::Path> = paths.iter().map(|pb| pb.as_path()).collect();
                    let internal_prefix = if is_dir {
                        let item_vpath = widget_item_data(&widget)
                            .map(|(p, _)| p)
                            .unwrap_or_default();
                        crate::archive::browse::parse_archive_path(Path::new(&item_vpath))
                            .map(|(_, internal)| internal.trim_matches('/').to_string())
                            .unwrap_or_default()
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
                    } else if is_move {
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
                    transfer_items(&s3, paths, None, is_move, TransferDest::Dir(target_path)).await;
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
        let name = file_item.name();
        if name == ".." {
            icon.set_icon_name(Some("go-up-symbolic"));
            label.set_tooltip_text(Some("Parent folder"));
            label.add_css_class("dim-label");
        } else {
            icon.set_from_gicon(&item_icon(&name, file_item.is_dir()));
            label.set_tooltip_text(None);
            label.remove_css_class("dim-label");
        }
        label.set_label(&name);
        unsafe {
            hbox.set_data("item-path", file_item.path());
            hbox.set_data("item-is-dir", file_item.is_dir());
        }
    });
    let name_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.name().to_lowercase().cmp(&b.name().to_lowercase()).into()
    });
    let name_col = gtk::ColumnViewColumn::new(Some("Name"), Some(name_factory));
    name_col.set_expand(true);
    name_col.set_resizable(true);
    name_col.set_sorter(Some(&name_sorter));
    column_view.append_column(&name_col);

    // Size column
    let size_factory = gtk::SignalListItemFactory::new();
    size_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(1.0).css_classes(["numeric", "dim-label"]).build();
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
    size_col.set_fixed_width(96);
    size_col.set_resizable(true);
    size_col.set_sorter(Some(&size_sorter));
    column_view.append_column(&size_col);

    // Modified column
    let date_factory = gtk::SignalListItemFactory::new();
    date_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).css_classes(["numeric", "dim-label"]).build();
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
    date_col.set_fixed_width(150);
    date_col.set_resizable(true);
    date_col.set_sorter(Some(&date_sorter));
    column_view.append_column(&date_col);

    // Created column
    let created_factory = gtk::SignalListItemFactory::new();
    created_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).css_classes(["numeric", "dim-label"]).build();
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
    created_col.set_fixed_width(150);
    created_col.set_visible(false);
    created_col.set_resizable(true);
    created_col.set_sorter(Some(&created_sorter));
    column_view.append_column(&created_col);

    // Accessed column
    let accessed_factory = gtk::SignalListItemFactory::new();
    accessed_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder().xalign(0.0).css_classes(["numeric", "dim-label"]).build();
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
    accessed_col.set_fixed_width(150);
    accessed_col.set_visible(false);
    accessed_col.set_resizable(true);
    accessed_col.set_sorter(Some(&accessed_sorter));
    column_view.append_column(&accessed_col);

    // Type column
    let type_factory = gtk::SignalListItemFactory::new();
    type_factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build();
        item.set_child(Some(&label));
    });
    type_factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let file_item = item.item().and_downcast::<FileItem>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        let file_type = file_item.file_type();
        label.set_tooltip_text(if file_type.is_empty() { None } else { Some(file_type.as_str()) });
        label.set_label(&file_type);
    });
    let type_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a.downcast_ref::<FileItem>().unwrap();
        let b = b.downcast_ref::<FileItem>().unwrap();
        a.file_type().to_lowercase().cmp(&b.file_type().to_lowercase()).into()
    });
    let type_col = gtk::ColumnViewColumn::new(Some("Type"), Some(type_factory));
    type_col.set_fixed_width(180);
    type_col.set_resizable(true);
    type_col.set_sorter(Some(&type_sorter));
    column_view.append_column(&type_col);

    // Right-click any column header to show or hide the optional columns.
    let cols = gio::SimpleActionGroup::new();
    let header_menu = gio::Menu::new();
    for (key, label, col) in [
        ("modified", "Modified", &date_col),
        ("created", "Created", &created_col),
        ("accessed", "Accessed", &accessed_col),
        ("type", "Type", &type_col),
        ("size", "Size", &size_col),
    ] {
        let action = gio::SimpleAction::new_stateful(key, None, &col.is_visible().to_variant());
        let c = col.clone();
        action.connect_activate(move |a, _| {
            let visible = !c.is_visible();
            c.set_visible(visible);
            a.set_state(&visible.to_variant());
        });
        cols.add_action(&action);
        header_menu.append(Some(label), Some(&format!("cols.{}", key)));
    }
    column_view.insert_action_group("cols", Some(&cols));
    for col in [&name_col, &size_col, &date_col, &created_col, &accessed_col, &type_col] {
        col.set_header_menu(Some(&header_menu));
    }
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

pub fn go_back(state: &SharedPanel) {
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

pub fn go_forward(state: &SharedPanel) {
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

pub fn go_up(state: &SharedPanel) {
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
                            remember_password(&s, &archive, &password);
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
                            remember_password(&s, &archive, &password);
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
            }
        } else {
            let uri = format!("file://{}", dest.display());
            if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
                crate::utils::show_error("Open Failed", &e.to_string());
            }
        }
    });
}

pub fn load_directory(state: &SharedPanel) {
    let s = state.borrow();
    let current_path = s.current_path.clone();

    if let Some((current_archive, internal_prefix)) =
        crate::archive::browse::parse_archive_path(&current_path)
    {
        let virtual_root = s.archive_virtual_root.clone();
        let busy = s.pulse_source.is_some();
        drop(s);

        // The loaded entries may belong to a different archive (e.g. after Back/Forward
        // from archive B into archive A). Re-read the right archive instead of showing
        // the wrong contents.
        let loaded = if virtual_root.is_empty() {
            None
        } else {
            crate::archive::browse::parse_archive_path(Path::new(&virtual_root)).map(|(a, _)| a)
        };
        if loaded.as_deref() != Some(current_archive.as_path()) {
            if !busy {
                crate::archive::browse::reopen_archive(state, &current_archive);
            }
            return;
        }

        let s = state.borrow();
        let archive_entries = s.archive_entries.clone();
        let show_hidden = s.show_hidden.get();
        let raw_store = s.raw_store.clone();
        let path_entry = s.path_entry.clone();
        let status_label = s.status_label.clone();
        drop(s);

        let internal_prefix = internal_prefix.trim_matches('/').to_string();

        // Build the whole list first and insert it in one go: one change notification
        // instead of one per file keeps big folders fast.
        let mut items: Vec<FileItem> = vec![FileItem::new("..", "..", true, 0, 0, 0, 0, "")];

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
            let file_type = type_description(&display_name, entry.is_dir);
            let item = FileItem::new(
                &display_name,
                &full_virtual,
                entry.is_dir,
                entry.size,
                entry.modified,
                0,
                0,
                &file_type,
            );
            items.push(item);
            count += 1;
        }
        raw_store.splice(0, raw_store.n_items(), &items);

        let _ = (path_entry, status_label, count);
        update_location_ui(state);
        update_status(state);
        return;
    }

    let raw_store = s.raw_store.clone();
    let show_hidden = s.show_hidden.get();

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
            "",
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
            let file_type = type_description(&name, is_dir);

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

    raw_store.splice(0, raw_store.n_items(), &entries);

    drop(s);
    update_location_ui(state);
    update_status(state);
}

/// "~/Documents" for paths under the home folder; the full path otherwise.
pub fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if path == home {
            return "~".to_string();
        }
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// Inverse of `display_path`: expands a leading "~".
fn expand_home(text: &str) -> PathBuf {
    if text == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(text));
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(text)
}

/// Full-colour themed icon for a file name, like a file manager shows.
fn item_icon(name: &str, is_dir: bool) -> gio::Icon {
    if is_dir {
        return gio::ThemedIcon::from_names(&["folder", "folder-symbolic"]).upcast();
    }
    let (content_type, _) = gio::content_type_guess(Some(name), None::<&[u8]>);
    gio::content_type_get_icon(&content_type)
}

/// Human-readable type ("Zip archive", "PDF document") instead of a bare extension.
pub fn type_description(name: &str, is_dir: bool) -> String {
    if is_dir {
        return "Folder".to_string();
    }
    let (content_type, uncertain) = gio::content_type_guess(Some(name), None::<&[u8]>);
    if uncertain || gio::content_type_is_unknown(&content_type) {
        return match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => format!("{} file", ext.to_uppercase()),
            _ => "File".to_string(),
        };
    }
    gio::content_type_get_description(&content_type).to_string()
}

/// Updates the header title, path bar text/icon and the "inside an archive" styling.
fn update_location_ui(state: &SharedPanel) {
    let s = state.borrow();
    let path_entry = s.path_entry.clone();
    if let Some((archive, internal)) = crate::archive::browse::parse_archive_path(&s.current_path) {
        let internal = internal.trim_matches('/').to_string();
        let archive_name = archive
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let encrypted = s.current_password.is_some();
        path_entry.set_text(&if internal.is_empty() {
            format!("{}/", display_path(&archive))
        } else {
            format!("{}/{}/", display_path(&archive), internal)
        });
        path_entry.add_css_class("in-archive");
        path_entry.set_primary_icon_name(Some("package-x-generic-symbolic"));
        path_entry.set_primary_icon_tooltip_text(Some("Browsing inside an archive"));
        if encrypted {
            path_entry.set_secondary_icon_name(Some("changes-prevent-symbolic"));
            path_entry.set_secondary_icon_tooltip_text(Some("Password-protected archive"));
        } else {
            path_entry.set_secondary_icon_name(None);
        }
        if let Some(t) = &s.window_title {
            let title = if internal.is_empty() {
                archive_name.clone()
            } else {
                internal.rsplit('/').next().unwrap_or(&internal).to_string()
            };
            t.set_title(&title);
            t.set_subtitle(&if internal.is_empty() {
                display_path(archive.parent().unwrap_or(Path::new("/")))
            } else {
                format!("in {}", archive_name)
            });
        }
    } else {
        path_entry.set_text(&display_path(&s.current_path));
        path_entry.remove_css_class("in-archive");
        path_entry.set_primary_icon_name(Some("folder-symbolic"));
        path_entry.set_primary_icon_tooltip_text(None);
        path_entry.set_secondary_icon_name(None);
        if let Some(t) = &s.window_title {
            let title = match s.current_path.file_name() {
                Some(n) if s.current_path.as_path() != dirs::home_dir().unwrap_or_default().as_path() => {
                    n.to_string_lossy().to_string()
                }
                Some(_) => "Home".to_string(),
                None => "/".to_string(),
            };
            t.set_title(&title);
            t.set_subtitle(&s.current_path.parent().map(display_path).unwrap_or_default());
        }
    }
}

/// Status line: item count, or how many are selected and their total size.
/// Also shows/hides the empty-folder message.
pub fn update_status(state: &SharedPanel) {
    let s = state.borrow();
    if s.progress_bar.is_visible() {
        return; // an operation is reporting progress
    }
    let n = s.sort_model.n_items();
    let mut items = 0u32;
    let mut selected = 0u32;
    let mut selected_bytes = 0u64;
    let mut selected_dirs = false;
    let selection = s.selection_model.selection();
    for pos in 0..n {
        if let Some(fi) = s.sort_model.item(pos).and_downcast::<FileItem>() {
            if fi.name() == ".." {
                continue;
            }
            items += 1;
            if selection.contains(pos) {
                selected += 1;
                if fi.is_dir() {
                    selected_dirs = true;
                } else {
                    selected_bytes += fi.size();
                }
            }
        }
    }
    let noun = |n: u32| if n == 1 { "item" } else { "items" };
    let text = if selected > 0 {
        let size = if selected_bytes > 0 || !selected_dirs {
            format!(" ({})", crate::utils::format::format_size(selected_bytes))
        } else {
            String::new()
        };
        format!("{} of {} {} selected{}", selected, items, noun(items), size)
    } else {
        format!("{} {}", items, noun(items))
    };
    s.status_label.set_label(&text);

    let filtering = !s.search_pattern.borrow().is_empty();
    s.empty_page.set_visible(items == 0 && s.pulse_source.is_none());
    if filtering {
        s.empty_page.set_icon_name(Some("edit-find-symbolic"));
        s.empty_page.set_title("No matching files");
        s.empty_page.set_description(Some(&format!("Nothing here matches \u{201c}{}\u{201d}", s.search_pattern.borrow())));
    } else if crate::archive::browse::parse_archive_path(&s.current_path).is_some() {
        s.empty_page.set_icon_name(Some("package-x-generic-symbolic"));
        s.empty_page.set_title("This folder is empty");
        s.empty_page.set_description(Some("Drop files here to add them to the archive"));
    } else {
        s.empty_page.set_icon_name(Some("folder-symbolic"));
        s.empty_page.set_title("This folder is empty");
        s.empty_page.set_description(None);
    }
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
