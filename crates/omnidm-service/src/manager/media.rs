// SPDX-License-Identifier: GPL-3.0-or-later

//! Video streams (HLS and DASH): added with the quality and sound to download, run by
//! [`crate::media`], then saved like any other download.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use omnidm_core::engine::{EngineError, Event, Method, Request};
use omnidm_ipc::protocol::{
    BrowserCookie, FinishStatus, MediaAudio, MediaFound, MediaInfo, MediaQuality, Progress,
};
use omnidm_media::{Container, Manifest, Source, StreamKind, hls};
use omnidm_net::cookies::Url;
use omnidm_net::naming::sanitize_file_name;
use omnidm_store::{DownloadId, NewDownload, Status};
use tokio_util::sync::CancellationToken;

use super::{Manager, Pending, Update, has_header, jar_from, queues};
use crate::job::unique_path;
use crate::media::{
    Choice, Joined, MediaEvent, MediaJob, MediaJobError, MediaOutcome, QualityChoice, WORK_SUFFIX,
    read_playlist, run_media,
};
use crate::ytdlp::{self, YtDlp};

/// How often a running stream's progress is saved.
const SAVE_EVERY: Duration = Duration::from_secs(2);

/// Most videos waiting in quality windows; the oldest goes when another comes.
const MAX_OFFERS: usize = 32;

/// Videos found by the extension, by offer number, until their quality window answers.
#[derive(Default)]
pub(super) struct Offers {
    last: i64,
    held: BTreeMap<i64, MediaAdd>,
}

/// Playlist names that say nothing about the video.
const GENERIC_NAMES: [&str; 8] = [
    "master",
    "index",
    "playlist",
    "manifest",
    "chunklist",
    "video",
    "stream",
    "main",
];

/// A video stream to download.
#[derive(Debug, Clone, Default)]
pub struct MediaAdd {
    /// The playlist (`.m3u8`) or manifest (`.mpd`).
    pub url: String,
    /// The page's title, for the file name.
    pub title: Option<String>,
    /// The page that plays it.
    pub referrer: Option<String>,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<BrowserCookie>,
    /// From private browsing: its cookies aren't saved.
    pub private: bool,
    pub choice: Choice,
    /// Where to save it; by default where its category says.
    pub dir: Option<PathBuf>,
    /// Put in this queue instead of starting now.
    pub queue: Option<String>,
}

impl Manager {
    /// Holds a video the extension found until the user chooses its quality, and asks for
    /// its quality window ([`Update::MediaOffer`]). Returns its offer number.
    pub fn offer_media(&self, found: MediaFound) -> Result<i64, String> {
        let add = from_found(found);
        media_request(&add)?;
        let offer = {
            let mut offers = self.inner.offers.lock().unwrap();
            offers.last += 1;
            let offer = offers.last;
            if offers.held.len() >= MAX_OFFERS {
                offers.held.pop_first();
            }
            offers.held.insert(offer, add);
            offer
        };
        self.send(Update::MediaOffer { offer });
        Ok(offer)
    }

    /// The playlist address of a video waiting in its quality window.
    pub fn offer_url(&self, offer: i64) -> Option<String> {
        let offers = self.inner.offers.lock().unwrap();
        offers.held.get(&offer).map(|add| add.url.clone())
    }

    /// The quality already picked on the page for a video waiting in its quality window.
    pub fn offer_quality(&self, offer: i64) -> Option<QualityChoice> {
        let offers = self.inner.offers.lock().unwrap();
        offers.held.get(&offer)?.choice.quality.clone()
    }

    /// What a video found on a page offers, for the menu on the video.
    pub async fn look_at_found(&self, found: MediaFound) -> Result<MediaInfo, String> {
        self.look_at_media(from_found(found)).await
    }

    /// What a video waiting in its quality window offers.
    pub async fn look_at_offer(&self, offer: i64) -> Result<MediaInfo, String> {
        let add = self.held_offer(offer)?;
        self.look_at_media(add).await
    }

    /// Downloads a video waiting in its quality window, as chosen there. On an error it
    /// stays, so the window can try again.
    pub async fn accept_offer(
        &self,
        offer: i64,
        title: String,
        dir: PathBuf,
        choice: Choice,
        queue: Option<String>,
    ) -> Result<i64, String> {
        let add = MediaAdd {
            title: Some(title),
            dir: Some(dir),
            choice,
            queue,
            ..self.held_offer(offer)?
        };
        let id = self.add_media(add).await?;
        self.forget_offer(offer);
        Ok(id)
    }

    /// Drops a video whose quality window closed.
    pub fn forget_offer(&self, offer: i64) {
        self.inner.offers.lock().unwrap().held.remove(&offer);
    }

    fn held_offer(&self, offer: i64) -> Result<MediaAdd, String> {
        let offers = self.inner.offers.lock().unwrap();
        offers
            .held
            .get(&offer)
            .cloned()
            .ok_or_else(|| "this video is no longer waiting; find it again in the browser".into())
    }

    /// What a video stream offers, for choosing its quality: its playlist is read, and for
    /// an HLS master playlist the first quality's too, to learn the video's length.
    pub async fn look_at_media(&self, look: MediaAdd) -> Result<MediaInfo, String> {
        let request = media_request(&look)?;
        let manifest = self.read_media(&request).await?;
        let mut duration = manifest.duration;
        if duration.is_none()
            && let Some(Source::Playlist(url)) = manifest.qualities.first().map(|q| &q.source)
        {
            let first = Request {
                url: url.to_string(),
                ..request.clone()
            };
            if let Some(text) = self.read_text(&first).await
                && let Ok(track) = hls::read_track(text.as_bytes(), url)
            {
                duration = track.duration;
            }
        }
        let name = media_file_name(look.title.as_deref(), &look.url);
        Ok(media_info(
            &manifest,
            duration,
            name.trim_end_matches(".mp4"),
        ))
    }

    /// Adds a video stream. Its playlist is read first, so DRM, a live stream or a broken
    /// link is refused before anything is added.
    pub async fn add_media(&self, add: MediaAdd) -> Result<i64, String> {
        let request = media_request(&add)?;
        let manifest = self.read_media(&request).await?;
        let queue = match &add.queue {
            Some(name) => Some(queues::find(&self.inner.store.lock().unwrap(), name)?.id),
            None => None,
        };

        let title = add.title.as_deref().or(manifest.title.as_deref());
        let name = media_file_name(title, &add.url);
        let (category, dir) = match &add.dir {
            Some(dir) => {
                if !dir.is_absolute() {
                    return Err("the folder must be an absolute path".into());
                }
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("can't create the folder: {e}"))?;
                (self.place(&name, None, &add.url, dir).0, dir.clone())
            }
            None => self.place(&name, Some("video/mp4"), &add.url, &self.downloads_folder()),
        };
        let choice = serde_json::to_string(&add.choice).map_err(|e| e.to_string())?;
        let id = {
            let mut store = self.inner.store.lock().unwrap();
            let id = store
                .insert_download(&NewDownload {
                    url: add.url.clone(),
                    save_dir: dir.to_string_lossy().into_owned(),
                    file_name: Some(name.clone()),
                    referrer: add.referrer.clone(),
                    user_agent: None,
                    headers: request.headers.clone(),
                    method: Method::Get,
                    body: Vec::new(),
                })
                .map_err(|e| e.to_string())?;
            store.set_media(id, &choice).map_err(|e| e.to_string())?;
            if category.is_some() {
                store
                    .set_download_category(id, category)
                    .map_err(|e| e.to_string())?;
            }
            if !add.private && !add.cookies.is_empty() {
                let json = serde_json::to_vec(&add.cookies).map_err(|e| e.to_string())?;
                store.save_cookies(id, &json).map_err(|e| e.to_string())?;
            }
            if let Some(queue) = queue {
                store.add_to_queue(queue, id).map_err(|e| e.to_string())?;
            }
            id.0
        };
        self.note(id, "Added a video stream");
        if let Some(queue) = queue {
            self.fill_queue(queue.0);
            return Ok(id);
        }
        self.enqueue(Pending {
            id,
            dir,
            file_name: Some(name),
            probe: None,
            resume: None,
            request,
            connections: None,
            media: Some(add.choice),
        });
        Ok(id)
    }

    /// Runs a video stream (already counted as running) until it ends.
    pub(super) fn run_media(&self, pending: Pending, cancel: CancellationToken) {
        let id = pending.id;
        let url = pending.request.url.clone();
        let dir = pending.dir;
        let name = pending.file_name.unwrap_or_else(|| "video.mp4".into());
        let work = dir.join(format!("{name}{WORK_SUFFIX}"));
        let job = MediaJob {
            client: self.inner.client.clone(),
            request: pending.request,
            work: work.clone(),
            choice: pending.media.unwrap_or_default(),
            config: self.inner.config.engine.clone(),
            parallel: self.max_connections() as usize,
            cancel,
            ytdlp: ytdlp::is_youtube(&url).then(|| self.ytdlp()),
        };
        self.set_status(id, Status::Running, None);
        self.note(id, "Started");
        let manager = self.clone();
        tokio::spawn(async move {
            let mut last_save: Option<Instant> = None;
            let result = run_media(job, |event| match event {
                MediaEvent::Note(text) => manager.note(id, text),
                MediaEvent::Chose(choice) => {
                    if let Ok(json) = serde_json::to_string(&choice) {
                        let _ = manager
                            .inner
                            .store
                            .lock()
                            .unwrap()
                            .set_media(DownloadId(id), &json);
                    }
                }
                MediaEvent::Progress {
                    saved,
                    total,
                    speed,
                    active,
                } => {
                    manager.send(Update::Progress(Progress {
                        download_id: id,
                        downloaded: saved,
                        total,
                        speed,
                        connections: active,
                    }));
                    manager.engine_event(
                        id,
                        &Event::Progress {
                            downloaded: saved,
                            total,
                            speed,
                            connections: active,
                        },
                    );
                    if active == 0 || last_save.is_none_or(|at| at.elapsed() >= SAVE_EVERY) {
                        last_save = Some(Instant::now());
                        let _ = manager.inner.store.lock().unwrap().set_media_progress(
                            DownloadId(id),
                            saved,
                            total,
                        );
                    }
                }
            })
            .await;

            let (status, path, error) = match result {
                Ok(MediaOutcome::Done(tracks)) => match save_tracks(&dir, &name, &work, tracks) {
                    Ok(path) => {
                        let saved_as = path.file_name().and_then(|n| n.to_str());
                        if let Some(saved_as) = saved_as.filter(|n| *n != name) {
                            let _ = manager
                                .inner
                                .store
                                .lock()
                                .unwrap()
                                .set_file_name(DownloadId(id), saved_as);
                        }
                        (
                            FinishStatus::Completed,
                            Some(path.to_string_lossy().into_owned()),
                            None,
                        )
                    }
                    Err(e) => (
                        FinishStatus::Failed,
                        None,
                        Some(format!("the video couldn't be saved: {e}")),
                    ),
                },
                Ok(MediaOutcome::Paused) => (FinishStatus::Paused, None, None),
                // Saved pieces are kept; a fresh playlist link continues them.
                Err(e @ MediaJobError::Engine(EngineError::LinkExpired)) => {
                    (FinishStatus::Paused, None, Some(e.to_string()))
                }
                Err(e) => (FinishStatus::Failed, None, Some(e.to_string())),
            };
            let stored = match status {
                FinishStatus::Completed => Status::Completed,
                FinishStatus::Paused => Status::Paused,
                FinishStatus::Failed => Status::Failed,
            };
            let stored_error = error.as_deref().filter(|_| stored == Status::Failed);
            manager.set_status(id, stored, stored_error);
            manager.ended(id, status, path, error);
        });
    }

    /// The stream's playlist (or, for YouTube, yt-dlp's description of the video), read;
    /// its problems (DRM, live, a dead link) as the error.
    async fn read_media(&self, request: &Request) -> Result<Manifest, String> {
        if ytdlp::is_youtube(&request.url) {
            // yt-dlp keeps its own time limit; the first time, it is downloaded too.
            let ytdlp = self.ytdlp();
            let cancel = CancellationToken::new();
            let read = ytdlp.read(
                &self.inner.client,
                request,
                &self.inner.config.engine,
                &cancel,
            );
            return match read.await {
                Ok(Some(manifest)) => Ok(manifest),
                Ok(None) => Err("cancelled".into()),
                Err(e) => Err(e.to_string()),
            };
        }
        let read = tokio::time::timeout(
            self.inner.config.probe_timeout,
            read_playlist(
                &self.inner.client,
                request,
                &self.inner.config.engine,
                &CancellationToken::new(),
            ),
        )
        .await;
        match read {
            Ok(Ok(Some(manifest))) => Ok(manifest),
            Ok(Ok(None)) => Err("cancelled".into()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("the server took too long".into()),
        }
    }

    /// A playlist's text, or `None` if it can't be had quickly.
    async fn read_text(&self, request: &Request) -> Option<String> {
        let mut bytes = Vec::new();
        let fetched = tokio::time::timeout(
            self.inner.config.probe_timeout,
            omnidm_core::fetch::fetch(
                &self.inner.client,
                request,
                &request.url,
                None,
                &self.inner.config.engine,
                &CancellationToken::new(),
                &mut bytes,
                |_| {},
            ),
        )
        .await;
        match fetched {
            Ok(Ok(Some(_))) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            _ => None,
        }
    }

    /// What reads YouTube videos.
    pub(crate) fn ytdlp(&self) -> Arc<YtDlp> {
        self.inner.ytdlp.lock().unwrap().clone()
    }

    /// Uses yt-dlp at `path`, fetching it from `release` when it is missing (`None`: never).
    pub fn set_ytdlp(&self, path: PathBuf, release: Option<String>) {
        *self.inner.ytdlp.lock().unwrap() = Arc::new(YtDlp::new(path, release));
    }

    fn set_status(&self, id: i64, status: Status, error: Option<&str>) {
        let _ = self
            .inner
            .store
            .lock()
            .unwrap()
            .set_status(DownloadId(id), status, error);
    }
}

/// A video the extension found, to add (or look at).
fn from_found(found: MediaFound) -> MediaAdd {
    MediaAdd {
        url: found.url.trim().to_owned(),
        title: found.title.filter(|t| !t.trim().is_empty()),
        referrer: found.page_url,
        headers: found
            .headers
            .into_iter()
            .map(|h| (h.name, h.value))
            .collect(),
        cookies: found.cookies,
        private: found.private,
        choice: Choice {
            quality: found.quality.map(|q| QualityChoice {
                height: q.height,
                bandwidth: q.bandwidth,
            }),
            audio: None,
        },
        ..MediaAdd::default()
    }
}

/// The request for a stream's playlist, with the page's address and cookies.
fn media_request(add: &MediaAdd) -> Result<Request, String> {
    let url = Url::parse(add.url.trim()).map_err(|e| format!("bad URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https links can be downloaded".into());
    }
    let mut headers = add.headers.clone();
    if let Some(referrer) = &add.referrer
        && !has_header(&headers, "referer")
    {
        headers.push(("Referer".into(), referrer.clone()));
    }
    Ok(Request {
        url: add.url.trim().to_owned(),
        method: Method::Get,
        body: Vec::new(),
        headers,
        cookies: jar_from(&add.cookies),
    })
}

/// `manifest` for the quality window, with sizes estimated from `duration`.
fn media_info(manifest: &Manifest, duration: Option<f64>, title: &str) -> MediaInfo {
    let place = |track: &omnidm_media::AudioTrack| {
        manifest
            .audio
            .iter()
            .position(|a| std::ptr::eq(a, track))
            .map(|i| i as u32)
    };
    let qualities = manifest
        .qualities
        .iter()
        .map(|quality| {
            let default = manifest.default_audio(quality);
            // HLS counts the sound in a quality's bandwidth; DASH lists it apart.
            let sound = default.and_then(|a| a.bandwidth).unwrap_or(0);
            MediaQuality {
                width: quality.width,
                height: quality.height,
                bandwidth: quality.bandwidth,
                frame_rate: quality.frame_rate,
                codecs: quality.codecs.clone(),
                audio: manifest.audio_for(quality).filter_map(place).collect(),
                default_audio: default.and_then(place),
                size: duration
                    .filter(|_| quality.bandwidth > 0)
                    .map(|seconds| ((quality.bandwidth + sound) as f64 * seconds / 8.0) as u64),
            }
        })
        .collect();
    MediaInfo {
        kind: match manifest.kind {
            StreamKind::Hls => "hls",
            StreamKind::Dash => "dash",
            StreamKind::YouTube => "youtube",
        }
        .into(),
        title: manifest
            .title
            .as_deref()
            .map(sanitize_file_name)
            .unwrap_or_else(|| title.to_owned()),
        duration,
        qualities,
        audio: manifest
            .audio
            .iter()
            .map(|a| MediaAudio {
                name: a.name.clone(),
                language: a.language.clone(),
                bandwidth: a.bandwidth,
            })
            .collect(),
    }
}

/// The file name for a stream: the page's title, else the playlist's name (or its folder's,
/// when the playlist is just `master.m3u8` or the like). The extension may change once the
/// format is known.
pub(super) fn media_file_name(title: Option<&str>, url: &str) -> String {
    let from_title = title.map(str::trim).filter(|t| !t.is_empty());
    let stem = match from_title {
        Some(title) => title.to_owned(),
        None => {
            let segments: Vec<String> = Url::parse(url)
                .ok()
                .and_then(|u| {
                    u.path_segments()
                        .map(|s| s.filter(|p| !p.is_empty()).map(String::from).collect())
                })
                .unwrap_or_default();
            let stem_of = |segment: &str| {
                segment
                    .rsplit_once('.')
                    .map_or(segment, |(stem, _)| stem)
                    .to_owned()
            };
            let mut stems = segments.iter().rev().map(|s| stem_of(s));
            let last = stems.next().unwrap_or_default();
            if GENERIC_NAMES.contains(&last.to_ascii_lowercase().as_str()) {
                stems.next().unwrap_or(last)
            } else {
                last
            }
        }
    };
    let stem = if stem.trim().is_empty() {
        "video".into()
    } else {
        sanitize_file_name(&stem)
    };
    format!("{stem}.mp4")
}

/// Moves the joined tracks to their final names next to the work folder, which is then
/// removed: the picture as `<name>.<ext>`, a separate sound track as `<name> (sound).<ext>`.
/// Returns the picture's file.
fn save_tracks(
    dir: &Path,
    name: &str,
    work: &Path,
    tracks: Vec<Joined>,
) -> std::io::Result<PathBuf> {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let mut main = None;
    for track in tracks {
        let extension = match (track.sound, track.container) {
            (true, Container::Mp4) => "m4a",
            (_, container) => container.extension(),
        };
        let file = if track.sound {
            format!("{stem} (sound).{extension}")
        } else {
            format!("{stem}.{extension}")
        };
        let target = unique_path(dir, &file, Path::exists);
        std::fs::rename(&track.path, &target)?;
        if !track.sound {
            main = Some(target);
        }
    }
    match std::fs::remove_dir_all(work) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    main.ok_or_else(|| std::io::Error::other("no picture was downloaded"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        let name = |title, url| media_file_name(title, url);
        assert_eq!(
            name(Some(" A: talk? "), "https://x/master.m3u8"),
            "A_ talk_.mp4"
        );
        assert_eq!(name(None, "https://x/shows/cats.m3u8?t=1"), "cats.mp4");
        assert_eq!(name(None, "https://x/films/one/manifest.mpd"), "one.mp4");
        assert_eq!(name(None, "https://x/master.m3u8"), "master.mp4");
        assert_eq!(name(Some(""), "https://x/"), "video.mp4");
    }
}
