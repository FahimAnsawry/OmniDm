// SPDX-License-Identifier: GPL-3.0-or-later

//! Windows notifications when a download completes or fails.

use omnidm_ipc::protocol::{FinishStatus, Finished};
use omnidm_service::manager::Manager;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_notification::NotificationExt;

/// The saved setting: `"false"` turns the notifications off.
pub const NOTIFY_SETTING: &str = "notify_finished";

const PART_SUFFIX: &str = ".omnidm-part";

/// Tells the user how a download ended, unless they are looking at it already.
pub fn finished(app: &AppHandle, manager: &Manager, done: &Finished) {
    let title = match done.status {
        FinishStatus::Completed => "Download complete",
        FinishStatus::Failed => "Download failed",
        FinishStatus::Paused => return,
    };
    if manager.setting(NOTIFY_SETTING).as_deref() == Some("false") {
        return;
    }
    // The list, or this download's own progress window.
    let watching = ["main".to_string(), format!("capture-{}", done.download_id)]
        .iter()
        .filter_map(|label| app.get_webview_window(label))
        .any(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
    if watching {
        return;
    }
    let name = done
        .path
        .clone()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            manager
                .file_of(done.download_id)
                .ok()
                .filter(|p| !p.is_dir())
        })
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .map(|n| n.strip_suffix(PART_SUFFIX).map(String::from).unwrap_or(n))
        .unwrap_or_else(|| "A download".into());
    let body = match &done.error {
        Some(error) => format!("{name}\n{error}"),
        None => name,
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("OmniDm: can't show a notification: {e}");
    }
}
