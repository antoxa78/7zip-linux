use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adw::prelude::*;

pub struct ProgressDialog {
    pub dialog: adw::Dialog,
    pub progress_bar: gtk::ProgressBar,
    status_label: gtk::Label,
    pub cancel_flag: Arc<AtomicBool>,
    pub pause_flag: Arc<AtomicBool>,
    pub is_background: Arc<AtomicBool>,
    pub background_button: gtk::Button,
    pub cancel_button: gtk::Button,
}

impl ProgressDialog {
    pub fn new(status: &str) -> Self {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let pause_flag = Arc::new(AtomicBool::new(false));
        let is_background = Arc::new(AtomicBool::new(false));

        let dialog = adw::Dialog::builder()
            .title(status)
            .content_width(400)
            .content_height(160)
            .build();

        let toolbar_view = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        toolbar_view.add_top_bar(&header);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_margin_top(12);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let status_label = gtk::Label::builder()
            .label(status)
            .xalign(0.0)
            .wrap(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        content.append(&status_label);

        let progress_bar = gtk::ProgressBar::builder()
            .show_text(true)
            .text("0%")
            .hexpand(true)
            .build();
        content.append(&progress_bar);

        toolbar_view.set_content(Some(&content));

        let background_button = gtk::Button::with_label("Background");

        let pause_button = gtk::Button::with_label("Pause");

        let cancel_button = gtk::Button::with_label("Cancel");
        cancel_button.add_css_class("destructive-action");

        let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bottom_bar.set_margin_top(8);
        bottom_bar.set_margin_bottom(8);
        bottom_bar.set_margin_start(12);
        bottom_bar.set_margin_end(12);
        bottom_bar.append(&background_button);
        bottom_bar.append(&pause_button);
        bottom_bar.set_halign(gtk::Align::End);
        bottom_bar.set_hexpand(true);
        bottom_bar.append(&cancel_button);
        toolbar_view.add_bottom_bar(&bottom_bar);

        dialog.set_child(Some(&toolbar_view));

        let pf = pause_flag.clone();
        let pb = pause_button.clone();
        pause_button.connect_clicked(move |_| {
            let was_paused = pf.fetch_xor(true, Ordering::Relaxed);
            pb.set_label(if was_paused { "Pause" } else { "Resume" });
        });

        Self {
            dialog,
            progress_bar,
            status_label,
            cancel_flag,
            pause_flag,
            is_background,
            background_button,
            cancel_button,
        }
    }

    pub fn present(&self) {
        self.dialog.present(crate::utils::parent_window().as_ref());
    }

    pub fn close(&self) {
        self.dialog.close();
    }

    pub fn set_status(&self, text: &str) {
        self.status_label.set_text(text);
    }

    pub fn set_progress(&self, fraction: f64, percent: u8) {
        self.progress_bar.set_fraction(fraction);
        self.progress_bar.set_text(Some(&format!("{}%", percent)));
    }
}
