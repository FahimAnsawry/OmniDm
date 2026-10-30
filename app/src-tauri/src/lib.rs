// SPDX-License-Identifier: GPL-3.0-or-later

//! OmniDM desktop app: runs the download service, serves the extension and the CLI over
//! the named pipe, and shows the download list.

mod capture;
mod clipboard;
mod details;
mod media;
mod notify;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use omnidm_core::engine::EngineConfig;
use omnidm_ipc::protocol::{
    AddRequest, CategorySummary, DownloadSummary, MediaInfo, Progress, QueueSummary,
};
#[cfg(windows)]
use omnidm_queue::power::SystemPower;
use omnidm_queue::power::{NoPower, Power};
use omnidm_service::manager::{Manager, ManagerConfig, MediaAdd, Update};
use omnidm_service::media::{AudioChoice, Choice, QualityChoice};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager as _, WindowEvent};

/// Downloads that run at once; more wait their turn.
const MAX_ACTIVE: usize = 3;

/// Progress goes to the window at most this often (5 times a second), every download in
/// one event, so a long list stays smooth.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

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
                eprintln!("OmniDM: can't open its pipe: {e}");
                std::process::exit(1);
            }
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // Started with Windows: into the tray, like a start from the browser.
            Some(vec!["--background"]),
        ))
        .manage(clipboard::ClipboardWatch::default())
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
            settings::apply_saved(app.handle());
            clipboard::start(app.handle().clone());

            #[cfg(windows)]
            tauri::async_runtime::spawn(omnidm_service::server::serve(manager.clone(), listener));
            let power = power();
            tauri::async_runtime::spawn(omnidm_service::scheduler::run(
                manager.clone(),
                power.clone(),
            ));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(omnidm_service::after_queue::run(
                manager.clone(),
                power,
                Arc::new(move || handle.exit(0)),
                omnidm_service::after_queue::COUNTDOWN,
            ));
            forward_updates(app.handle().clone(), &manager);
            build_tray(app)?;
            if !background {
                show_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            if window.label() == "main" {
                // Closing the window keeps OmniDM (and its downloads) running in the tray.
                // The page hides it once its closing animation has played.
                api.prevent_close();
            } else if let Some(id) = capture::download_of(window.label()) {
                capture::closed_unanswered(window.app_handle(), id);
            } else if let Some(offer) = media::offer_of(window.label()) {
                window.app_handle().state::<Manager>().forget_offer(offer);
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_downloads,
            list_queues,
            list_categories,
            pause_downloads,
            resume_downloads,
            delete_downloads,
            pause_all,
            resume_all,
            start_queue,
            stop_queue,
            open_file,
            show_in_folder,
            open_extension_guide,
            cancel_after_queue,
            add_download,
            look_at_media,
            add_media,
            suggest_place,
            capture::capture_prompt,
            capture::answer_capture,
            capture::cancel_capture,
            capture::set_ask_before_download,
            settings::get_settings,
            settings::save_settings,
            settings::create_category,
            settings::edit_category,
            settings::delete_category,
            settings::move_category,
            details::open_details,
            details::download_details,
            media::media_offer,
            media::accept_media
        ])
        .run(tauri::generate_context!())
        .expect("error while running OmniDM");
}

#[tauri::command]
fn list_downloads(manager: tauri::State<'_, Manager>) -> Vec<DownloadSummary> {
    manager.list()
}

#[tauri::command]
fn list_queues(manager: tauri::State<'_, Manager>) -> Vec<QueueSummary> {
    manager.queues()
}

#[tauri::command]
fn list_categories(manager: tauri::State<'_, Manager>) -> Vec<CategorySummary> {
    manager.categories()
}

// Async, so it runs on the runtime: pausing may start a queue's next download.
#[tauri::command]
async fn pause_downloads(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    ids: Vec<i64>,
) -> Result<(), String> {
    for id in ids {
        manager.pause(id);
    }
    let _ = app.emit("downloads-changed", ());
    Ok(())
}

/// Continues downloads; the first error is reported once all have been tried.
#[tauri::command]
async fn resume_downloads(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    ids: Vec<i64>,
) -> Result<(), String> {
    let mut first_error = None;
    for id in ids {
        if let Err(e) = manager.resume(id).await {
            first_error.get_or_insert(e);
        }
    }
    let _ = app.emit("downloads-changed", ());
    first_error.map_or(Ok(()), Err)
}

/// Removes downloads from the list, and their files with `delete_files`.
#[tauri::command]
async fn delete_downloads(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    ids: Vec<i64>,
    delete_files: bool,
) -> Result<(), String> {
    let mut first_error = None;
    for id in ids {
        if let Err(e) = manager.delete(id, delete_files).await {
            first_error.get_or_insert(e);
        }
    }
    let _ = app.emit("downloads-changed", ());
    first_error.map_or(Ok(()), Err)
}

#[tauri::command]
async fn pause_all(manager: tauri::State<'_, Manager>) -> Result<(), String> {
    manager.pause_all();
    Ok(())
}

#[tauri::command]
async fn resume_all(manager: tauri::State<'_, Manager>) -> Result<usize, String> {
    Ok(manager.resume_all().await)
}

#[tauri::command]
async fn start_queue(manager: tauri::State<'_, Manager>, name: String) -> Result<(), String> {
    manager.start_queue(&name)
}

#[tauri::command]
async fn stop_queue(manager: tauri::State<'_, Manager>, name: String) -> Result<(), String> {
    manager.stop_queue(&name)
}

/// What the Add download dialog sends.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewDownload {
    url: String,
    /// Blank: the server's name.
    file_name: Option<String>,
    folder: String,
    connections: Option<u32>,
    /// Put in this queue instead of starting now.
    queue: Option<String>,
}

#[tauri::command]
async fn add_download(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    download: NewDownload,
) -> Result<i64, String> {
    let id = manager.add(AddRequest {
        url: download.url.trim().to_string(),
        dir: download.folder.trim().to_string(),
        file_name: download
            .file_name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty()),
        connections: download.connections,
        headers: vec![],
        cookies: vec![],
        data: None,
        queue: download.queue,
    })?;
    let _ = app.emit("downloads-changed", ());
    Ok(id)
}

/// What a video link offers, for the quality window.
#[tauri::command]
async fn look_at_media(
    manager: tauri::State<'_, Manager>,
    url: String,
) -> Result<MediaInfo, String> {
    manager
        .look_at_media(MediaAdd {
            url,
            ..MediaAdd::default()
        })
        .await
}

/// A video the quality window chose.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewMedia {
    url: String,
    /// The file's name, without an extension.
    title: String,
    folder: String,
    quality: Option<QualityChoice>,
    audio: Option<AudioChoice>,
    /// Put in this queue instead of starting now.
    queue: Option<String>,
}

#[tauri::command]
async fn add_media(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    media: NewMedia,
) -> Result<i64, String> {
    let id = manager
        .add_media(MediaAdd {
            url: media.url,
            title: Some(media.title),
            dir: Some(PathBuf::from(media.folder.trim())),
            choice: Choice {
                quality: media.quality,
                audio: media.audio,
            },
            queue: media.queue,
            ..MediaAdd::default()
        })
        .await?;
    let _ = app.emit("downloads-changed", ());
    Ok(id)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Place {
    category: Option<String>,
    folder: String,
}

/// The category and folder a link would get.
#[tauri::command]
fn suggest_place(
    manager: tauri::State<'_, Manager>,
    url: String,
    file_name: Option<String>,
) -> Place {
    let (category, folder) = manager.suggest_place(url.trim(), file_name.as_deref());
    Place {
        category,
        folder: folder.to_string_lossy().into_owned(),
    }
}

/// Opens a completed download with its usual program.
#[tauri::command]
fn open_file(manager: tauri::State<'_, Manager>, id: i64) -> Result<(), String> {
    let file = manager.file_of(id)?;
    tauri_plugin_opener::open_path(&file, None::<&str>)
        .map_err(|e| format!("can't open {}: {e}", file.display()))
}

/// Shows a download in Explorer: its file selected if there is one yet, else its folder.
#[tauri::command]
fn show_in_folder(manager: tauri::State<'_, Manager>, id: i64) -> Result<(), String> {
    let file = manager.file_of(id)?;
    let shown = if file.is_file() {
        tauri_plugin_opener::reveal_item_in_dir(&file)
    } else {
        let dir = if file.is_dir() {
            file.as_path()
        } else {
            file.parent().unwrap_or(&file)
        };
        tauri_plugin_opener::open_path(dir, None::<&str>)
    };
    shown.map_err(|e| format!("can't show {}: {e}", file.display()))
}

/// Opens the page that tells how to install the browser extension (it isn't in the stores).
#[tauri::command]
fn open_extension_guide() -> Result<(), String> {
    let page = concat!(env!("CARGO_PKG_HOMEPAGE"), "#readme");
    tauri_plugin_opener::open_url(page, None::<&str>).map_err(|e| format!("can't open {page}: {e}"))
}

/// Wake timers and keep-awake for the scheduler; without them, schedules still run while
/// the PC is on.
fn power() -> Arc<dyn Power> {
    #[cfg(windows)]
    match SystemPower::new() {
        Ok(power) => return Arc::new(power),
        Err(e) => eprintln!("OmniDM: can't use power management: {e}"),
    }
    Arc::new(NoPower)
}

#[tauri::command]
fn cancel_after_queue(manager: tauri::State<'_, Manager>) -> bool {
    manager.cancel_after_queue()
}

/// Sends the service's updates to the window. Progress is gathered and sent as one
/// `download-progress` event (a list) every [`PROGRESS_EVERY`]. An after-queue countdown
/// also brings the window forward once, so whoever is at the PC can cancel it.
fn forward_updates(app: AppHandle, manager: &Manager) {
    let mut updates = manager.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut shown_countdown = false;
        let mut progress: HashMap<i64, Progress> = HashMap::new();
        let mut tick = tokio::time::interval(PROGRESS_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let update = tokio::select! {
                update = updates.recv() => update,
                _ = tick.tick() => {
                    if !progress.is_empty() {
                        let batch: Vec<Progress> = progress.drain().map(|(_, p)| p).collect();
                        let _ = app.emit("download-progress", batch);
                    }
                    continue;
                }
            };
            match update {
                Ok(Update::Progress(p)) => {
                    progress.insert(p.download_id, p);
                }
                Ok(Update::Finished(finished)) => {
                    // Its last progress is old news now.
                    progress.remove(&finished.download_id);
                    notify::finished(&app, &app.state::<Manager>(), &finished);
                    let _ = app.emit("download-finished", finished);
                }
                Ok(Update::Ask(about)) => {
                    capture::open(&app, &about);
                    let _ = app.emit("downloads-changed", ());
                }
                Ok(Update::MediaOffer { offer }) => media::open(&app, offer),
                Ok(Update::QueueFinished { queue_id, tried }) => {
                    let finished = serde_json::json!({ "queueId": queue_id, "tried": tried });
                    let _ = app.emit("queue-finished", finished);
                }
                Ok(Update::AfterQueue(notice)) => {
                    let counting = notice.outcome.is_none() && !notice.waiting_for_downloads;
                    if counting && !shown_countdown {
                        show_window(&app);
                    }
                    shown_countdown = notice.outcome.is_none() && (shown_countdown || counting);
                    let _ = app.emit("after-queue", notice);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open OmniDM", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause-all", "Pause all", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume-all", "Resume all", true, None::<&str>)?;
    let asks = capture::saved_choice(&app.state::<Manager>());
    let ask = CheckMenuItem::with_id(
        app,
        "ask",
        "Ask before downloading",
        true,
        asks,
        None::<&str>,
    )?;
    app.manage(capture::AskMenuItem(ask.clone()));
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &resume,
            &PredefinedMenuItem::separator(app)?,
            &ask,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("OmniDM")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "pause-all" => {
                let manager = app.state::<Manager>().inner().clone();
                tauri::async_runtime::spawn(async move { manager.pause_all() });
            }
            "resume-all" => {
                let manager = app.state::<Manager>().inner().clone();
                tauri::async_runtime::spawn(async move { manager.resume_all().await });
            }
            "ask" => {
                // The item ticks itself; save what it now shows.
                let ask = app
                    .try_state::<capture::AskMenuItem>()
                    .and_then(|item| item.0.is_checked().ok())
                    .unwrap_or(true);
                let _ = capture::apply(app, ask);
            }
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

pub(crate) fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Where OmniDM keeps its database: `OMNIDM_DATA_DIR` if set, else `%APPDATA%\OmniDm`.
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

/// Tells the running OmniDM to open its window.
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
