pub mod format;
pub mod icons;

pub use icons::icon_for_file;

use adw::prelude::*;
use std::cell::RefCell;

thread_local! {
    static APP_WINDOW: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

pub fn set_app_window(window: &impl IsA<gtk::Window>) {
    APP_WINDOW.with(|w| *w.borrow_mut() = Some(window.clone().upcast()));
}

pub fn parent_window() -> Option<gtk::Window> {
    APP_WINDOW.with(|w| w.borrow().clone())
}

pub fn show_error(title: &str, detail: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading(title)
        .body(detail)
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(parent_window().as_ref());
}

pub fn show_info(title: &str, detail: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading(title)
        .body(detail)
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(parent_window().as_ref());
}
