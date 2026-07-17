use adw::prelude::*;

use crate::config::settings::SharedSettings;

pub fn show(settings: &SharedSettings) {
    let s = settings.borrow();

    let dialog = adw::Dialog::builder()
        .title("Settings")
        .content_width(450)
        .content_height(520)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // ... all content appended to `content` ...

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&content));
    toolbar_view.set_content(Some(&scrolled));

    // Default format
    let fmt_label = gtk::Label::builder().label("Default archive format:").xalign(0.0).build();
    content.append(&fmt_label);
    let fmt_combo = gtk::DropDown::from_strings(&["7z", "zip", "tar", "tar.gz", "tar.bz2", "tar.xz"]);
    fmt_combo.set_selected(match s.default_format.as_str() {
        "zip" => 1,
        "tar" => 2,
        "tar.gz" => 3,
        "tar.bz2" => 4,
        "tar.xz" => 5,
        _ => 0,
    });
    content.append(&fmt_combo);

    // Default compression level
    let level_label = gtk::Label::builder().label("Default compression level:").xalign(0.0).build();
    content.append(&level_label);
    let level_combo = gtk::DropDown::from_strings(&[
        "Store (no compression)",
        "Fastest",
        "Fast",
        "Normal",
        "Maximum",
        "Ultra",
    ]);
    level_combo.set_selected(s.default_compression_level);
    content.append(&level_combo);

    // Default overwrite
    let ow_label = gtk::Label::builder().label("Default overwrite mode:").xalign(0.0).build();
    content.append(&ow_label);
    let ow_combo = gtk::DropDown::from_strings(&["Overwrite all", "Skip existing", "Auto-rename"]);
    ow_combo.set_selected(match s.default_overwrite.as_str() {
        "skip" => 1,
        "rename" => 2,
        _ => 0,
    });
    content.append(&ow_combo);

    // Show hidden by default
    let hidden_check = gtk::CheckButton::builder()
        .label("Show hidden files by default")
        .active(s.show_hidden_by_default)
        .build();
    content.append(&hidden_check);

    // Application theme
    let theme_label = gtk::Label::builder().label("Application theme:").xalign(0.0).build();
    content.append(&theme_label);
    let theme_combo = gtk::DropDown::from_strings(&["System", "Light", "Dark"]);
    theme_combo.set_selected(s.color_scheme);
    content.append(&theme_combo);

    // Remember window size
    let window_check = gtk::CheckButton::builder()
        .label("Remember window size")
        .active(s.remember_window_size)
        .build();
    content.append(&window_check);

    // Config dir info
    let config_path = crate::config::config_dir();
    let info_label = gtk::Label::builder()
        .label(&format!("Config: {}", config_path.display()))
        .xalign(0.0)
        .wrap(true)
        .build();
    info_label.add_css_class("dim-label");
    content.append(&info_label);

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&content));
    toolbar_view.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar_view));

    // Save button
    let save_button = gtk::Button::builder().label("Save").build();
    save_button.add_css_class("suggested-action");
    header.pack_end(&save_button);

    let cancel_button = gtk::Button::builder().label("Cancel").build();
    header.pack_end(&cancel_button);

    let dialog_cancel = dialog.clone();
    cancel_button.connect_clicked(move |_| { dialog_cancel.close(); });

    let settings = settings.clone();
    let dialog_save = dialog.clone();
    save_button.connect_clicked(move |_| {
        let fmt_idx = fmt_combo.selected();
        let formats = ["7z", "zip", "tar", "tar.gz", "tar.bz2", "tar.xz"];
        let ow_idx = ow_combo.selected();
        let overwrites = ["overwrite", "skip", "rename"];

        let mut s = settings.borrow_mut();
        s.default_format = formats[fmt_idx as usize].to_string();
        s.default_compression_level = level_combo.selected();
        s.default_overwrite = overwrites[ow_idx as usize].to_string();
        s.show_hidden_by_default = hidden_check.is_active();
        s.color_scheme = theme_combo.selected();
        s.remember_window_size = window_check.is_active();
        s.save();
        drop(s);

        let style_manager = adw::StyleManager::default();
        let color_scheme = match theme_combo.selected() {
            1 => adw::ColorScheme::ForceLight,
            2 => adw::ColorScheme::ForceDark,
            _ => adw::ColorScheme::Default,
        };
        style_manager.set_color_scheme(color_scheme);

        dialog_save.close();
    });

    dialog.present(crate::utils::parent_window().as_ref());
}
