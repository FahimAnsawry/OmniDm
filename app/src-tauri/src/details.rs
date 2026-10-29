// SPDX-License-Identifier: GPL-3.0-or-later

//! The Details window: one per download, showing its pieces, connections, what the
//! server said, and its log.

use omnidm_ipc::protocol::DownloadDetails;
use omnidm_service::manager::Manager;
use tauri::{AppHandle, Manager as _, WebviewUrl, WebviewWindowBuilder};

const LABEL_PREFIX: &str = "details-";

// Async: creating a window from a synchronous command can hang on Windows.
#[tauri::command]
pub async fn open_details(app: AppHandle, id: i64) -> Result<(), String> {
    let label = format!("{LABEL_PREFIX}{id}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("Details")
        .inner_size(760.0, 560.0)
        .min_inner_size(560.0, 420.0)
        .center()
        .focused(true)
        .build()
        .map(drop)
        .map_err(|e| format!("can't open the details window: {e}"))
}

#[tauri::command]
pub fn download_details(manager: tauri::State<'_, Manager>, id: i64) -> Option<DownloadDetails> {
    manager.details(id)
}
