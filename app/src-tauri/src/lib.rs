// SPDX-License-Identifier: GPL-3.0-or-later

//! OmniDm desktop app: runs the download service, serves the extension and the CLI over
//! the named pipe, and shows the download list.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use omnidm_core::engine::EngineConfig;
use omnidm_ipc::protocol::DownloadSummary;
use omnidm_service::manager::{Manager, ManagerConfig, Update};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager as _, WindowEvent};

/// Downloads that run at once; more wait their turn.
const MAX_ACTIVE: usize = 3;

pub fn run() {
    // Started by the browser's native messaging host: no window until asked.
    let background = std::env::args().any(|arg| arg == "--background");
    // OMNIDM_PIPE is for development and tests.
    let pipe_name =
        std::env::var("OMNIDM_PIPE").unwrap_or_else(|_| omnidm_ipc::pipe::default_pipe_name());

    #[cfg(windows)]
    let listener = {
        use omnidm_ipc::pipe::{PipeError, PipeListener};
        match tauri::async_runtime::block_on(async { PipeListener::bind(&pipe_name) }) {
            Ok(listener) => listener,
            // Already running: bring that window forward instead of starting twice.
            Err(PipeError::AlreadyRunning) => {
                if !background {
                    tauri::async_runtime::block_on(ask_running_app_to_show(&pipe_name));
                }
                return;
            }
            Err(e) => {
                eprintln!("OmniDm: can't open its pipe: {e}");
                std::process::exit(1);
            }
        }
    };

    tauri::Builder::default()
        .setup(move |app| {
            let manager = Manager::start(ManagerConfig {
                data_dir: data_dir(),
                downloads_dir: downloads_dir(app.handle()),
                max_active: MAX_ACTIVE,
                engine: EngineConfig::default(),
                // Below the extension's 10 s wait, so a slow server's file goes back to
                // the browser in time.
                probe_timeout: Duration::from_secs(8),
            })?;
            let handle = app.handle().clone();
            manager.set_show_handler(Arc::new(move || show_window(&handle)));
            app.manage(manager.clone());

            #[cfg(windows)]
            tauri::async_runtime::spawn(omnidm_service::server::serve(manager.clone(), listener));
            forward_updates(app.handle().clone(), &manager);
            build_tray(app)?;
            if !background {
                show_window(app.handle());
            }
            Ok(())
        })
        // Closing the window keeps OmniDm (and its downloads) running in the tray.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_downloads,
            pause_download,
            resume_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running OmniDm");
}

#[tauri::command]
fn list_downloads(manager: tauri::State<'_, Manager>) -> Vec<DownloadSummary> {
    manager.list()
}

#[tauri::command]
fn pause_download(manager: tauri::State<'_, Manager>, id: i64) -> bool {
    manager.pause(id)
}

#[tauri::command]
async fn resume_download(manager: tauri::State<'_, Manager>, id: i64) -> Result<(), String> {
    manager.resume(id).await
}

/// Sends the service's progress and finished updates to the window.
fn forward_updates(app: AppHandle, manager: &Manager) {
    let mut updates = manager.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match updates.recv().await {
                Ok(Update::Progress(progress)) => {
                    let _ = app.emit("download-progress", progress);
                }
                Ok(Update::Finished(finished)) => {
                    let _ = app.emit("download-finished", finished);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open OmniDm", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("OmniDm")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Where OmniDm keeps its database: `OMNIDM_DATA_DIR` if set, else `%APPDATA%\OmniDm`.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OMNIDM_DATA_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OmniDm")
}

/// Where captured downloads go: `OMNIDM_DOWNLOAD_DIR` if set, else the user's Downloads.
fn downloads_dir(app: &AppHandle) -> PathBuf {
    if let Some(dir) = std::env::var_os("OMNIDM_DOWNLOAD_DIR") {
        return PathBuf::from(dir);
    }
    app.path()
        .download_dir()
        .unwrap_or_else(|_| data_dir().join("Downloads"))
}

/// Tells the running OmniDm to open its window.
#[cfg(windows)]
async fn ask_running_app_to_show(pipe_name: &str) {
    use omnidm_ipc::frame::write_frame_async;
    use omnidm_ipc::protocol::{Envelope, Message};

    // Only another copy of this same program may receive the request.
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let Ok(mut pipe) = omnidm_ipc::pipe::connect(pipe_name, &[me]).await else {
        return;
    };
    let show = Envelope::new("show", Message::Show);
    if let Ok(bytes) = serde_json::to_vec(&show) {
        let _ = write_frame_async(&mut pipe, &bytes).await;
    }
}
