// SPDX-License-Identifier: GPL-3.0-or-later

//! The clipboard monitor (off by default): when a link to a file is copied, the window
//! opens the Add download dialog with it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use omnidm_service::manager::Manager;
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// The saved setting: `"true"` when the clipboard is watched.
pub const CLIPBOARD_SETTING: &str = "clipboard_monitor";

/// How often the clipboard is looked at.
const CHECK_EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct ClipboardWatch {
    on: AtomicBool,
    /// The clipboard's text when last looked at; only a change is offered.
    last: Mutex<Option<String>>,
}

/// Turns watching on or off. What is on the clipboard when it is turned on isn't offered.
pub fn set_watching(app: &AppHandle, on: bool) {
    let watch = app.state::<ClipboardWatch>();
    if on && !watch.on.load(Ordering::Relaxed) {
        *watch.last.lock().unwrap() = app.clipboard().read_text().ok();
    }
    watch.on.store(on, Ordering::Relaxed);
}

pub fn is_watching(app: &AppHandle) -> bool {
    app.state::<ClipboardWatch>().on.load(Ordering::Relaxed)
}

/// Watches the clipboard for as long as the app runs.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(CHECK_EVERY);
        loop {
            tick.tick().await;
            if !is_watching(&app) {
                continue;
            }
            let Ok(text) = app.clipboard().read_text() else {
                continue;
            };
            let watch = app.state::<ClipboardWatch>();
            {
                let mut last = watch.last.lock().unwrap();
                if last.as_deref() == Some(text.as_str()) {
                    continue;
                }
                *last = Some(text.clone());
            }
            let link = text.trim();
            let one_link = !link.is_empty() && !link.contains(char::is_whitespace);
            if one_link && app.state::<Manager>().looks_like_file(link) {
                crate::show_window(&app);
                let _ = app.emit("clipboard-link", link.to_string());
            }
        }
    });
}
