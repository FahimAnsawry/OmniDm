// SPDX-License-Identifier: GPL-3.0-or-later

//! OmniDM-Setup.exe: shows the setup window (app/src/setup/SetupWindow.tsx) and runs the
//! NSIS installer Tauri made for the app, silently, with the choices made in the window.
//! The installer's extra steps (browser link, shortcuts, start with Windows) are in
//! app/src-tauri/windows/hooks.nsh.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::webview::PageLoadEvent;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// The app's NSIS installer (empty in a development build: the window then pretends).
static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.exe"));

/// What the installed app takes on disk, in bytes (set by scripts/build-setup.ps1).
const INSTALLED_SIZE: &str = env!("OMNIDM_INSTALLED_SIZE");

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// The app's entry in Windows' Apps list, under HKEY_CURRENT_USER (Tauri names it after
/// the app's productName, which keeps its internal spelling).
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\OmniDm";

/// The running installer, and whether Cancel was pressed.
#[derive(Default)]
struct Install {
    child: Mutex<Option<Child>>,
    cancelled: AtomicBool,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Install::default())
        .setup(|app| {
            WebviewWindowBuilder::new(app, "setup", WebviewUrl::App("index.html".into()))
                .title("OmniDM Setup")
                .inner_size(680.0, 460.0)
                .visible(false)
                // A card on a see-through window, like the app's small windows.
                .decorations(false)
                .transparent(true)
                .background_color(Color(0, 0, 0, 0))
                .shadow(false)
                .resizable(false)
                .maximizable(false)
                .center()
                .focused(true)
                // The page's browser data goes to the temporary folder, not next to the
                // app's own.
                .data_directory(std::env::temp_dir().join("OmniDM-Setup"))
                .on_page_load(|window, payload| {
                    if payload.event() == PageLoadEvent::Finished {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = window.emit("window-reveal", ());
                    }
                })
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            setup_info,
            free_space,
            install,
            cancel_install,
            finish,
            open_page
        ])
        .run(tauri::generate_context!())
        .expect("error while running OmniDM Setup");
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupInfo {
    version: &'static str,
    size: u64,
    default_folder: String,
    installed: Option<Installed>,
}

#[derive(Serialize)]
struct Installed {
    folder: String,
    version: String,
}

#[tauri::command]
fn setup_info() -> SetupInfo {
    let default_folder = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("OmniDm");
    SetupInfo {
        version: VERSION,
        size: INSTALLED_SIZE.parse().unwrap_or(0),
        default_folder: default_folder.display().to_string(),
        installed: installed(),
    }
}

/// The OmniDM installed for this user, if its folder still has the app.
fn installed() -> Option<Installed> {
    let folder = registry::read(UNINSTALL_KEY, "InstallLocation")?;
    let folder = folder.trim_matches('"').to_string();
    if !Path::new(&folder).join("OmniDm.exe").exists() {
        return None;
    }
    let version = registry::read(UNINSTALL_KEY, "DisplayVersion").unwrap_or_default();
    Some(Installed { folder, version })
}

/// Free bytes on the folder's drive, or nothing if it can't be told.
#[tauri::command]
fn free_space(folder: String) -> Option<u64> {
    // The folder may not exist yet: ask about the nearest one that does.
    let mut path = PathBuf::from(folder);
    while !path.exists() {
        path = path.parent()?.to_path_buf();
    }
    disk::free(&path)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Choices {
    folder: String,
    desktop: bool,
    start_menu: bool,
    autostart: bool,
}

#[derive(Clone, Serialize)]
struct Progress {
    phase: &'static str,
    fraction: f64,
}

/// Runs the installer and reports `install-progress` until it is done.
#[tauri::command]
async fn install(app: AppHandle, choices: Choices) -> Result<(), String> {
    let fresh = installed().is_none();
    tauri::async_runtime::spawn_blocking(move || run_installer(&app, &choices, fresh))
        .await
        .map_err(|e| e.to_string())?
}

fn run_installer(app: &AppHandle, choices: &Choices, fresh: bool) -> Result<(), String> {
    let state = app.state::<Install>();
    state.cancelled.store(false, Ordering::SeqCst);
    let report = |phase, fraction: f64| {
        let _ = app.emit("install-progress", Progress { phase, fraction });
    };
    let folder = PathBuf::from(&choices.folder);
    let expected = INSTALLED_SIZE.parse::<u64>().unwrap_or(1).max(1);

    if PAYLOAD.is_empty() {
        // A development build: no installer inside, so only the progress is shown.
        for step in 0..=20 {
            if state.cancelled.load(Ordering::SeqCst) {
                return Err("Cancelled.".into());
            }
            report("copy", f64::from(step) / 20.0 * 0.95);
            std::thread::sleep(Duration::from_millis(90));
        }
        report("setup", 0.97);
        std::thread::sleep(Duration::from_millis(400));
        return Ok(());
    }

    let installer = std::env::temp_dir().join(format!(
        "OmniDM-{VERSION}-install-{}.exe",
        std::process::id()
    ));
    std::fs::write(&installer, PAYLOAD).map_err(|e| format!("Can't unpack the installer: {e}"))?;
    let result = (|| {
        let mut command = Command::new(&installer);
        command.arg("/S");
        if !choices.desktop {
            command.arg("/NODESKTOP");
        }
        if !choices.start_menu {
            command.arg("/NOSTARTMENU");
        }
        command.arg(if choices.autostart {
            "/AUTOSTART"
        } else {
            "/NOAUTOSTART"
        });
        // NSIS reads the folder from the rest of the line, unquoted, even with spaces.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.raw_arg(format!("/D={}", choices.folder));
        }
        let child = command
            .spawn()
            .map_err(|e| format!("Can't start the installer: {e}"))?;
        *state.child.lock().unwrap() = Some(child);

        let started = Instant::now();
        loop {
            let status = match state.child.lock().unwrap().as_mut() {
                // Cancel took it away and stopped it.
                None => return Err("Cancelled.".into()),
                Some(child) => child.try_wait().ok().flatten(),
            };
            if let Some(status) = status {
                *state.child.lock().unwrap() = None;
                if state.cancelled.load(Ordering::SeqCst) {
                    return Err("Cancelled.".into());
                }
                return match status.code() {
                    Some(0) => Ok(()),
                    code => Err(format!(
                        "The installer stopped (code {}). If OmniDM is open, close it from the tray, then try again.",
                        code.map_or_else(|| "none".into(), |c| c.to_string())
                    )),
                };
            }
            // A new install shows the files arriving; an update rewrites files that are
            // already there, so its bar follows the time instead.
            let by_time = 0.9 * (1.0 - (-started.elapsed().as_secs_f64() / 1.5).exp());
            let (phase, fraction) = if fresh {
                let copied = (folder_size(&folder) as f64 / expected as f64).min(0.95);
                let phase = if folder.join("uninstall.exe").exists() {
                    "setup"
                } else {
                    "copy"
                };
                (phase, copied.max(by_time.min(0.3)))
            } else {
                ("copy", by_time)
            };
            report(phase, fraction);
            std::thread::sleep(Duration::from_millis(150));
        }
    })();
    let _ = std::fs::remove_file(&installer);
    result
}

/// Stops the installer. A new install that was stopped halfway is removed again.
#[tauri::command]
fn cancel_install(app: AppHandle, folder: Option<String>) {
    let state = app.state::<Install>();
    state.cancelled.store(true, Ordering::SeqCst);
    let fresh = installed().is_none();
    if let Some(mut child) = state.child.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let (true, Some(folder)) = (fresh, folder) {
        let uninstaller = Path::new(&folder).join("uninstall.exe");
        if uninstaller.exists() {
            let _ = Command::new(uninstaller).arg("/S").spawn();
        }
    }
}

/// The Finish button: starts the app and opens the release notes if asked.
#[tauri::command]
fn finish(folder: String, launch: bool, whats_new: bool) -> Result<(), String> {
    if whats_new {
        open_page("whats-new".into())?;
    }
    if launch {
        let app = Path::new(&folder).join("OmniDm.exe");
        Command::new(&app)
            .spawn()
            .map_err(|e| format!("can't start {}: {e}", app.display()))?;
    }
    Ok(())
}

/// Opens one of OmniDM's pages in the browser.
#[tauri::command]
fn open_page(page: String) -> Result<(), String> {
    let url = match page.as_str() {
        "licence" => format!("{REPOSITORY}/blob/main/LICENSE"),
        // How to add the extension: it isn't in the browsers' stores.
        "extension" => format!("{REPOSITORY}#readme"),
        "whats-new" => format!("{REPOSITORY}/releases/tag/v{VERSION}"),
        other => return Err(format!("no page called {other}")),
    };
    tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| format!("can't open {url}: {e}"))
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

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

    /// A text value under `HKEY_CURRENT_USER\<key>`.
    pub(super) fn read(key: &str, name: &str) -> Option<String> {
        let key = super::wide(key);
        let name = super::wide(name);
        let mut buffer = vec![0u16; 1024];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: the strings are NUL-terminated; `bytes` is the buffer's size in bytes,
        // and the call writes at most that many.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = (bytes as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buffer[..len]))
    }
}

#[cfg(not(windows))]
mod registry {
    pub(super) fn read(_key: &str, _name: &str) -> Option<String> {
        None
    }
}

#[cfg(windows)]
mod disk {
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    pub(super) fn free(path: &Path) -> Option<u64> {
        let path = super::wide(&path.display().to_string());
        let mut free = 0u64;
        // SAFETY: `path` is NUL-terminated; the other outputs may be null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }
}

#[cfg(not(windows))]
mod disk {
    pub(super) fn free(_path: &std::path::Path) -> Option<u64> {
        None
    }
}
