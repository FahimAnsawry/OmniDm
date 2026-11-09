// SPDX-License-Identifier: GPL-3.0-or-later

//! Updates: OmniDM looks for a newer version on GitHub (the release's `latest.json`) a
//! little after it starts, then twice a day, and offers it in a small window. Installing
//! pauses the downloads, runs the signed installer quietly and restarts OmniDM, which
//! then continues them. The first start of a new version, by update or by its setup, shows
//! what changed in the same window.
//!
//! The browser extension doesn't update itself. When a release changes it, its notes end
//! with an [`EXTENSION_HEADING`] section (plain steps, which older versions show as text),
//! and the window shows how to update it, with a button to the release page.

use std::sync::Mutex;
use std::time::Duration;

use omnidm_service::manager::Manager;
use serde::Serialize;
use tauri::webview::PageLoadEvent;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager as _, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const LABEL: &str = "update";

/// Clear: the page draws the window's background, growing in.
const BACKGROUND: Color = Color(0, 0, 0, 0);

/// The first look waits until OmniDM has settled (and the network is up after a wake).
const FIRST_CHECK: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(12 * 60 * 60);

/// The version that ran last, to notice the first start of a new one.
const VERSION_SETTING: &str = "last_version";

/// The line that starts the "update the browser extension" section of a release's notes.
const EXTENSION_HEADING: &str = "Update the browser extension too";

/// The window's size: taller when it also shows the extension steps.
const SIZE: (f64, f64) = (600.0, 400.0);
const SIZE_WITH_EXTENSION: (f64, f64) = (600.0, 652.0);

// docs/releases/<version>.md, built in by build.rs.
include!(concat!(env!("OUT_DIR"), "/release_notes.rs"));

/// The update the window offers, or the one just installed.
#[derive(Default)]
pub struct Found {
    offer: Mutex<Option<Update>>,
    installed: Mutex<Option<UpdateInfo>>,
}

/// What the window shows of it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    current: String,
    notes: Option<String>,
    /// When it was published, in seconds since 1970.
    date: Option<i64>,
    /// Already installed: `version` is running now, and `previous` ran before it.
    installed: bool,
    previous: Option<String>,
    /// This version changed the browser extension, which the user has to update by hand.
    extension: bool,
}

/// A release's notes without the extension section, and whether it had one.
fn split_notes(text: Option<&str>) -> (Option<String>, bool) {
    let Some(text) = text else {
        return (None, false);
    };
    let mut kept = Vec::new();
    let mut extension = false;
    for line in text.lines() {
        if line.trim_start().starts_with(EXTENSION_HEADING) {
            extension = true;
            break;
        }
        kept.push(line);
    }
    let notes = kept.join("\n").trim().to_string();
    ((!notes.is_empty()).then_some(notes), extension)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Downloaded {
    done: u64,
    total: Option<u64>,
}

/// Continues what an update's restart paused, then looks for updates now and then. A
/// development build doesn't look (installing would replace the installed copy) unless
/// `OMNIDM_UPDATE_CHECK` is set.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let manager = app.state::<Manager>();
        // Set until resume_after_restart reads it: OmniDM restarted into an update.
        let restarted = manager
            .setting("resume_after_restart")
            .is_some_and(|s| !s.is_empty());
        manager.resume_after_restart().await;
        announce_new_version(&app, restarted);
        if cfg!(debug_assertions) && std::env::var_os("OMNIDM_UPDATE_CHECK").is_none() {
            return;
        }
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if let Err(e) = check(&app, false).await {
                eprintln!("OmniDM: can't look for updates: {e}");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Opens the window saying what changed, at the first start of a new version: one that
/// differs from the last version that ran, or (before OmniDM noted versions) one an update
/// restarted into. Not on a first install.
fn announce_new_version(app: &AppHandle, restarted: bool) {
    let manager = app.state::<Manager>();
    let current = app.package_info().version.to_string();
    let previous = manager.setting(VERSION_SETTING);
    if previous.as_deref() != Some(current.as_str())
        && let Err(e) = manager.set_setting(VERSION_SETTING, &current)
    {
        eprintln!("OmniDM: can't note its version: {e}");
    }
    let new = match &previous {
        Some(previous) => *previous != current,
        None => restarted,
    };
    if !new {
        return;
    }
    let (notes, extension) = split_notes(
        RELEASE_NOTES
            .iter()
            .find(|(version, _)| *version == current)
            .map(|(_, notes)| *notes),
    );
    *app.state::<Found>().installed.lock().unwrap() = Some(UpdateInfo {
        version: current.clone(),
        current,
        notes,
        date: None,
        installed: true,
        previous,
        extension,
    });
    open(app, extension);
}

/// Looks for a newer version and opens the window if there is one. `asked`: from the tray's
/// "Check for updates", so "none" and errors are shown too.
pub async fn check(app: &AppHandle, asked: bool) -> Result<(), String> {
    let found = async {
        app.updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    match found {
        Ok(Some(update)) => {
            let (_, extension) = split_notes(update.body.as_deref());
            *app.state::<Found>().offer.lock().unwrap() = Some(update);
            open(app, extension);
        }
        Ok(None) if asked => {
            let version = app.package_info().version.to_string();
            tell(
                app,
                "OmniDM is up to date",
                &format!("You have the latest version, {version}."),
            );
        }
        Ok(None) => {}
        Err(e) if asked => tell(app, "Can't look for updates", &e),
        Err(e) => return Err(e),
    }
    Ok(())
}

fn tell(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("OmniDM: can't show a notification: {e}");
    }
}

fn open(app: &AppHandle, extension: bool) {
    let (width, height) = if extension { SIZE_WITH_EXTENSION } else { SIZE };
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
        let _ = window.emit_to(LABEL, "update-found", ());
        crate::reveal(&window);
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Update OmniDM")
        // As wide as the download window, drawn the same way: see media.rs.
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .center()
        .background_color(BACKGROUND)
        .visible(false)
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                crate::reveal(&window);
            }
        })
        .build();
    if let Err(e) = built {
        eprintln!("OmniDM: can't open the update window: {e}");
    }
}

#[tauri::command]
pub fn update_info(app: AppHandle, found: tauri::State<'_, Found>) -> Option<UpdateInfo> {
    let Some(update) = found.offer.lock().unwrap().clone() else {
        return found.installed.lock().unwrap().clone();
    };
    let (notes, extension) = split_notes(update.body.as_deref());
    Some(UpdateInfo {
        version: update.version.clone(),
        current: app.package_info().version.to_string(),
        notes,
        date: update.date.map(|d| d.unix_timestamp()),
        installed: false,
        previous: None,
        extension,
    })
}

/// Opens a version's GitHub release page, where its extension zips are.
#[tauri::command]
pub fn open_release_page(version: String) -> Result<(), String> {
    let page = release_page(&version).ok_or("not a version number")?;
    tauri_plugin_opener::open_url(&page, None::<&str>)
        .map_err(|e| format!("can't open {page}: {e}"))
}

/// `https://github.com/…/releases/tag/v<version>`, for a version made only of the
/// characters a version number has.
fn release_page(version: &str) -> Option<String> {
    let valid = !version.is_empty()
        && version.len() <= 32
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    valid.then(|| format!("{}/releases/tag/v{version}", env!("CARGO_PKG_HOMEPAGE")))
}

/// Downloads the update (its progress goes to the window as `update-downloaded`), checks
/// its signature, pauses the downloads and installs it. On Windows OmniDM closes here and
/// the installer starts it again.
#[tauri::command]
pub async fn install_update(app: AppHandle, found: tauri::State<'_, Found>) -> Result<(), String> {
    let update = found
        .offer
        .lock()
        .unwrap()
        .clone()
        .ok_or("there is no update to install")?;
    let mut done = 0;
    let bytes = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                let _ = app.emit_to(LABEL, "update-downloaded", Downloaded { done, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("can't download the update: {e}"))?;
    let manager = app.state::<Manager>().inner().clone();
    manager.pause_for_restart().await?;
    if let Err(e) = update.install(bytes) {
        // Still running: carry on as before.
        manager.resume_after_restart().await;
        return Err(format!("can't install the update: {e}"));
    }
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_lose_the_extension_section_and_flag_it() {
        let text = "- New: TikTok videos.\n- Fixed: a crash.\n\nUpdate the browser extension too:\n1. Download it.\n";
        assert_eq!(
            split_notes(Some(text)),
            (
                Some("- New: TikTok videos.\n- Fixed: a crash.".into()),
                true
            )
        );
    }

    #[test]
    fn notes_without_the_section_stay_whole() {
        assert_eq!(
            split_notes(Some("- Fixed: a crash.\n")),
            (Some("- Fixed: a crash.".into()), false)
        );
        assert_eq!(split_notes(None), (None, false));
        assert_eq!(
            split_notes(Some("Update the browser extension too:\n1. Download it.")),
            (None, true)
        );
    }

    #[test]
    fn release_pages_only_take_version_numbers() {
        assert_eq!(
            release_page("0.2.0").as_deref(),
            Some("https://github.com/FahimAnsawry/OmniDm/releases/tag/v0.2.0")
        );
        assert_eq!(release_page("../../evil"), None);
        assert_eq!(release_page("0.2.0?x=1"), None);
        assert_eq!(release_page(""), None);
    }
}
