// SPDX-License-Identifier: GPL-3.0-or-later

//! The capture prompt: a small window for each download taken from the browser, asking
//! where to save it and whether to start now or later in a queue.

use omnidm_ipc::protocol::{AskAbout, CaptureAnswer};
use omnidm_service::manager::Manager;
use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, Manager as _, WebviewUrl, WebviewWindowBuilder, Wry};

/// The saved setting: `"false"` when captures start without asking.
pub const ASK_SETTING: &str = "ask_before_download";

/// The window of the capture with this id.
const LABEL_PREFIX: &str = "capture-";

/// The tray's "Ask before downloading" item, kept in step with the setting.
pub struct AskMenuItem(pub CheckMenuItem<Wry>);

/// Asking is on unless the user turned it off.
pub fn saved_choice(manager: &Manager) -> bool {
    manager.setting(ASK_SETTING).as_deref() != Some("false")
}

/// Opens the prompt for a held capture, or brings it forward if it is open.
pub fn open(app: &AppHandle, about: &AskAbout) {
    let label = format!("{LABEL_PREFIX}{}", about.download_id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("Download file")
        .inner_size(560.0, 360.0)
        .resizable(false)
        .maximizable(false)
        .always_on_top(true)
        .center()
        .focused(true)
        .build();
    if let Err(e) = built {
        eprintln!("OmniDm: can't open the download window: {e}");
    }
}

/// The capture a prompt window belongs to.
pub fn download_of(label: &str) -> Option<i64> {
    label.strip_prefix(LABEL_PREFIX)?.parse().ok()
}

/// Closing the prompt without answering cancels the download (as its Cancel button does).
pub fn closed_unanswered(app: &AppHandle, id: i64) {
    let manager = app.state::<Manager>();
    if manager.cancel_capture(id).is_ok() {
        let _ = app.emit("downloads-changed", ());
    }
}

#[tauri::command]
pub fn capture_prompt(manager: tauri::State<'_, Manager>, id: i64) -> Option<AskAbout> {
    manager.asking().into_iter().find(|a| a.download_id == id)
}

// Async, so it runs on the runtime: starting a download spawns its tasks there.
#[tauri::command]
pub async fn answer_capture(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    answer: CaptureAnswer,
) -> Result<(), String> {
    manager.answer_capture(answer)?;
    let _ = app.emit("downloads-changed", ());
    Ok(())
}

#[tauri::command]
pub fn cancel_capture(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    id: i64,
) -> Result<(), String> {
    manager.cancel_capture(id)?;
    let _ = app.emit("downloads-changed", ());
    Ok(())
}

#[tauri::command]
pub fn set_ask_before_download(app: AppHandle, ask: bool) -> Result<(), String> {
    apply(&app, ask)
}

/// Saves and applies the choice, and ticks or unticks the tray item.
pub fn apply(app: &AppHandle, ask: bool) -> Result<(), String> {
    let manager = app.state::<Manager>();
    manager.set_setting(ASK_SETTING, if ask { "true" } else { "false" })?;
    manager.set_ask_before_download(ask);
    if let Some(item) = app.try_state::<AskMenuItem>() {
        let _ = item.0.set_checked(ask);
    }
    // An open Settings page shows the change.
    let _ = app.emit("settings-changed", ());
    Ok(())
}
