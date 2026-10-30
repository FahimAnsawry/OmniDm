// SPDX-License-Identifier: GPL-3.0-or-later

//! The video window: opens when the extension sends a video found on a page, to choose
//! its quality, name and folder.

use omnidm_ipc::protocol::MediaInfo;
use omnidm_service::manager::Manager;
use omnidm_service::media::{AudioChoice, Choice, QualityChoice};
use tauri::webview::PageLoadEvent;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager as _, WebviewUrl, WebviewWindowBuilder};

/// The window of the video with this offer number.
const LABEL_PREFIX: &str = "media-";

/// Clear: the page draws the window's background, growing in.
const BACKGROUND: Color = Color(0, 0, 0, 0);

/// Opens the window for a video the extension found.
pub fn open(app: &AppHandle, offer: i64) {
    let label = format!("{LABEL_PREFIX}{offer}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("Download video")
        // The same size as the download window.
        .inner_size(560.0, 360.0)
        // The window draws its own title bar, and is see-through so its content can
        // grow in; its shadow comes on once the content is there.
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .always_on_top(true)
        .center()
        // Hidden until the page has loaded: no white flash.
        .background_color(BACKGROUND)
        .visible(false)
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = window.show();
                let _ = window.set_focus();
            }
        })
        .build();
    if let Err(e) = built {
        eprintln!("OmniDm: can't open the video window: {e}");
    }
}

/// The video a window belongs to.
pub fn offer_of(label: &str) -> Option<i64> {
    label.strip_prefix(LABEL_PREFIX)?.parse().ok()
}

/// A video waiting in its window, and what it offers.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offered {
    url: String,
    info: MediaInfo,
    /// The quality picked on the page, if any.
    quality: Option<QualityChoice>,
}

#[tauri::command]
pub async fn media_offer(
    manager: tauri::State<'_, Manager>,
    offer: i64,
) -> Result<Offered, String> {
    let url = manager
        .offer_url(offer)
        .ok_or("this video is no longer waiting; find it again in the browser")?;
    let info = manager.look_at_offer(offer).await?;
    let quality = manager.offer_quality(offer);
    Ok(Offered { url, info, quality })
}

/// What the window chose.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    offer: i64,
    /// The file's name, without an extension.
    title: String,
    folder: String,
    quality: Option<QualityChoice>,
    audio: Option<AudioChoice>,
    queue: Option<String>,
}

#[tauri::command]
pub async fn accept_media(
    app: AppHandle,
    manager: tauri::State<'_, Manager>,
    accepted: Accepted,
) -> Result<i64, String> {
    let id = manager
        .accept_offer(
            accepted.offer,
            accepted.title,
            accepted.folder.trim().into(),
            Choice {
                quality: accepted.quality,
                audio: accepted.audio,
            },
            accepted.queue,
        )
        .await?;
    let _ = app.emit("downloads-changed", ());
    Ok(id)
}
