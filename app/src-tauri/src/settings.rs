// SPDX-License-Identifier: GPL-3.0-or-later

//! The Settings page: every setting is saved in the database and applied at once.

use std::path::PathBuf;

use omnidm_ipc::protocol::CategoryEdit;
use omnidm_service::manager::Manager;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_autostart::ManagerExt;

use crate::{capture, clipboard, notify};

const DOWNLOADS_SETTING: &str = "downloads_dir";
const MAX_ACTIVE_SETTING: &str = "max_active";
const MAX_CONNECTIONS_SETTING: &str = "max_connections";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    downloads_folder: String,
    /// Downloads outside queues that run at once.
    max_active: usize,
    /// Connections one download may use.
    max_connections: u32,
    ask_before_download: bool,
    notify_finished: bool,
    clipboard_monitor: bool,
    start_with_windows: bool,
}

/// Applies the saved settings when OmniDm starts.
pub fn apply_saved(app: &AppHandle) {
    let manager = app.state::<Manager>();
    if let Some(folder) = manager.setting(DOWNLOADS_SETTING) {
        let _ = manager.set_downloads_folder(PathBuf::from(folder));
    }
    if let Some(max) = number(&manager, MAX_ACTIVE_SETTING) {
        manager.set_max_active(max as usize);
    }
    if let Some(max) = number(&manager, MAX_CONNECTIONS_SETTING) {
        manager.set_max_connections(max);
    }
    manager.set_ask_before_download(capture::saved_choice(&manager));
    clipboard::set_watching(
        app,
        manager.setting(clipboard::CLIPBOARD_SETTING).as_deref() == Some("true"),
    );
}

fn number(manager: &Manager, key: &str) -> Option<u32> {
    manager.setting(key)?.parse().ok()
}

fn current(app: &AppHandle) -> AppSettings {
    let manager = app.state::<Manager>();
    AppSettings {
        downloads_folder: manager.downloads_folder().to_string_lossy().into_owned(),
        max_active: manager.max_active(),
        max_connections: manager.max_connections(),
        ask_before_download: manager.asks_before_download(),
        notify_finished: manager.setting(notify::NOTIFY_SETTING).as_deref() != Some("false"),
        clipboard_monitor: clipboard::is_watching(app),
        start_with_windows: app.autolaunch().is_enabled().unwrap_or(false),
    }
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> AppSettings {
    current(&app)
}

/// Saves and applies every setting; returns them as they now are (limits are clamped).
#[tauri::command]
pub fn save_settings(app: AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    let manager = app.state::<Manager>();
    let folder = PathBuf::from(settings.downloads_folder.trim());
    manager.set_downloads_folder(folder.clone())?;
    manager.set_setting(DOWNLOADS_SETTING, &folder.to_string_lossy())?;
    manager.set_max_active(settings.max_active);
    manager.set_setting(MAX_ACTIVE_SETTING, &manager.max_active().to_string())?;
    manager.set_max_connections(settings.max_connections);
    manager.set_setting(
        MAX_CONNECTIONS_SETTING,
        &manager.max_connections().to_string(),
    )?;
    capture::apply(&app, settings.ask_before_download)?;
    manager.set_setting(notify::NOTIFY_SETTING, flag(settings.notify_finished))?;
    clipboard::set_watching(&app, settings.clipboard_monitor);
    manager.set_setting(
        clipboard::CLIPBOARD_SETTING,
        flag(settings.clipboard_monitor),
    )?;
    let autostart = app.autolaunch();
    if autostart.is_enabled().unwrap_or(false) != settings.start_with_windows {
        let changed = if settings.start_with_windows {
            autostart.enable()
        } else {
            autostart.disable()
        };
        changed.map_err(|e| format!("can't change starting with Windows: {e}"))?;
    }
    let _ = app.emit("settings-changed", ());
    Ok(current(&app))
}

fn flag(on: bool) -> &'static str {
    if on { "true" } else { "false" }
}

/// Category changes also change the sidebar.
fn categories_changed(app: &AppHandle) {
    let _ = app.emit("downloads-changed", ());
}

#[tauri::command]
pub fn create_category(app: AppHandle, edit: CategoryEdit) -> Result<i64, String> {
    let id = app.state::<Manager>().create_category(edit)?;
    categories_changed(&app);
    Ok(id)
}

#[tauri::command]
pub fn edit_category(app: AppHandle, id: i64, edit: CategoryEdit) -> Result<(), String> {
    app.state::<Manager>().edit_category(id, edit)?;
    categories_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_category(app: AppHandle, id: i64) -> Result<(), String> {
    app.state::<Manager>().delete_category(id)?;
    categories_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn move_category(app: AppHandle, id: i64, index: usize) -> Result<(), String> {
    app.state::<Manager>().move_category(id, index)?;
    categories_changed(&app);
    Ok(())
}
