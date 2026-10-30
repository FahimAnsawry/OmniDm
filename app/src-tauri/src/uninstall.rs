// SPDX-License-Identifier: GPL-3.0-or-later

//! `OmniDm.exe --uninstall`: the window Windows' Apps list opens to remove OmniDM (the
//! installer points the uninstall entry here, app/src-tauri/windows/hooks.nsh). It asks,
//! then runs the installer's own `uninstall.exe` silently and shows its progress.
//!
//! The uninstaller closes every program started from OmniDM's folder, this window
//! included, so the window first copies itself and the uninstaller into a temporary
//! folder and runs from there (`--uninstall --from <folder>`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::webview::PageLoadEvent;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// Keeps a console window from flashing up for the clean-up command.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The temporary folder's name, before the process id.
const TEMP_PREFIX: &str = "OmniDM-uninstall-";

/// The OmniDM folder being removed, and the temporary folder this copy runs from.
struct Place {
    folder: PathBuf,
    temp: PathBuf,
}

/// Shows the uninstall window. With other arguments than `--uninstall` (a silent or
/// updating uninstall started by an installer), they go straight to `uninstall.exe`.
pub fn run() {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| arg != "--uninstall")
        .collect();
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(here) = exe.parent().map(Path::to_path_buf) else {
        return;
    };
    let from = match args.as_slice() {
        [flag, folder] if flag == "--from" => PathBuf::from(folder),
        [] => {
            move_out_and_restart(&exe, &here);
            return;
        }
        passed => {
            let _ = Command::new(here.join("uninstall.exe"))
                .args(passed)
                .spawn();
            return;
        }
    };

    let place = Place {
        folder: from,
        temp: here,
    };
    let webview_data = place.temp.join("webview");
    let app = tauri::Builder::default()
        .manage(place)
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "uninstall", WebviewUrl::App("index.html".into()))
                .title("Uninstall OmniDM")
                .inner_size(680.0, 460.0)
                .visible(false)
                // Drawn like the app's other small windows: a card on a see-through window.
                .decorations(false)
                .transparent(true)
                .background_color(Color(0, 0, 0, 0))
                .shadow(false)
                .resizable(false)
                .maximizable(false)
                .center()
                .focused(true)
                // Not the app's own browser data: "delete my data" removes that.
                .data_directory(webview_data)
                .on_page_load(|window, payload| {
                    if payload.event() == PageLoadEvent::Finished {
                        crate::reveal(&window);
                    }
                })
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            uninstall_info,
            uninstall,
            crate::wake_page
        ])
        .build(tauri::generate_context!())
        .expect("error while running OmniDM's uninstaller");
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            clean_up_later(&app.state::<Place>().temp);
        }
    });
}

/// Copies the app and the uninstaller into a new temporary folder and starts the window
/// from there. If that fails, the uninstaller's own window opens instead.
fn move_out_and_restart(exe: &Path, folder: &Path) {
    let uninstaller = folder.join("uninstall.exe");
    let temp = std::env::temp_dir().join(format!("{TEMP_PREFIX}{}", std::process::id()));
    let copied = std::fs::create_dir_all(&temp)
        .and_then(|()| std::fs::copy(exe, temp.join("OmniDm.exe")))
        .and_then(|_| std::fs::copy(&uninstaller, temp.join("uninstall.exe")));
    let started = copied.and_then(|_| {
        Command::new(temp.join("OmniDm.exe"))
            .arg("--uninstall")
            .arg("--from")
            .arg(folder)
            .spawn()
    });
    if started.is_err() {
        let _ = std::fs::remove_dir_all(&temp);
        let _ = Command::new(uninstaller).spawn();
    }
}

/// Removes the temporary folder once this program has quit; only a folder made by
/// [`move_out_and_restart`], never one started some other way.
fn clean_up_later(temp: &Path) {
    let ours = temp.parent() == Some(std::env::temp_dir().as_path())
        && temp
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(TEMP_PREFIX));
    if !ours {
        return;
    }
    let mut command = Command::new("cmd");
    command
        .args(["/c", "ping -n 3 127.0.0.1 >nul & rmdir /s /q"])
        .arg(temp);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command.spawn();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UninstallInfo {
    folder: String,
    /// What removing the app frees, in bytes.
    size: u64,
}

#[tauri::command]
fn uninstall_info(place: tauri::State<'_, Place>) -> UninstallInfo {
    UninstallInfo {
        size: folder_size(&place.folder),
        folder: place.folder.display().to_string(),
    }
}

#[derive(Clone, Serialize)]
struct Progress {
    /// `close` while OmniDM is being closed, then `files`.
    phase: &'static str,
    fraction: f64,
}

/// Runs the uninstaller silently and reports `uninstall-progress` until it is done.
#[tauri::command]
async fn uninstall(app: AppHandle, delete_data: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let place = app.state::<Place>();
        run_uninstaller(
            &app,
            &place.folder,
            &place.temp.join("uninstall.exe"),
            delete_data,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

fn run_uninstaller(
    app: &AppHandle,
    folder: &Path,
    uninstaller: &Path,
    delete_data: bool,
) -> Result<(), String> {
    let mut command = Command::new(uninstaller);
    command.arg("/S");
    if delete_data {
        command.arg("/DELETEDATA");
    }
    // Runs this copy on OmniDM's folder; NSIS reads it unquoted, last on the line.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(format!("_?={}", folder.display()));
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Can't start the uninstaller: {e}"))?;

    let start_size = folder_size(folder).max(1);
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return match status.code() {
                Some(0) => Ok(()),
                code => Err(format!(
                    "The uninstaller stopped (code {}). Close OmniDM from the tray, then try again.",
                    code.map_or_else(|| "none".into(), |c| c.to_string())
                )),
            };
        }
        let phase = if folder.join("OmniDm.exe").exists() {
            "close"
        } else {
            "files"
        };
        let removed = 1.0 - folder_size(folder) as f64 / start_size as f64;
        let by_time = 0.6 * (1.0 - (-started.elapsed().as_secs_f64() / 1.5).exp());
        let fraction = removed.max(by_time).min(0.95);
        let _ = app.emit("uninstall-progress", Progress { phase, fraction });
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn folder_size(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_size(&entry.path()),
            Ok(_) => entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}
