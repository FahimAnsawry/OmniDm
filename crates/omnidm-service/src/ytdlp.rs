// SPDX-License-Identifier: GPL-3.0-or-later

//! YouTube, through yt-dlp. YouTube doesn't play videos from a standard playlist: the
//! addresses of its formats are hidden in its player and scrambled, and the scrambling
//! changes often. yt-dlp (public domain, https://github.com/yt-dlp/yt-dlp) keeps up with
//! it, so OmniDM asks yt-dlp for a video's formats and real addresses, then downloads them
//! itself, in pieces, like any other stream.
//!
//! The program is fetched from yt-dlp's latest GitHub release the first time it is needed,
//! checked against the release's SHA-256 list, and kept. When reading a video fails, it is
//! updated (`yt-dlp -U`) and the video read again: YouTube changes are its usual cause.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use omnidm_core::engine::{EngineConfig, Method, Request};
use omnidm_core::fetch::fetch;
use omnidm_media::{
    AudioTrack, ByteRange, Container, Manifest, MediaError, Piece, Quality, Source, StreamKind,
    Track, Url,
};
use omnidm_net::cookies::CookieJar;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::media::MediaJobError;

/// Where the latest yt-dlp release's files are.
pub const RELEASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";

/// The program's file, in the release and on disk.
#[cfg(windows)]
pub const PROGRAM: &str = "yt-dlp.exe";
#[cfg(not(windows))]
pub const PROGRAM: &str = "yt-dlp";

/// The release's list of SHA-256 sums.
const SUMS: &str = "SHA2-256SUMS";

/// Each format is downloaded in pieces of this size, several at once. YouTube serves big
/// single requests slowly; yt-dlp uses the same size.
pub const PIECE_SIZE: u64 = 10 * 1024 * 1024;

/// Longest a video may take to read.
const READ_TIMEOUT: Duration = Duration::from_secs(90);
/// Longest an update may take.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a video's read manifest is reused (YouTube stream links last ~6 hours).
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// Most manifests kept in memory at once.
const MAX_CACHED: usize = 32;

/// The [`AudioTrack::group`] of every separate sound track.
const AUDIO_GROUP: &str = "audio";
/// YouTube's inner player API, which answers in ~300 ms without spawning yt-dlp.
const PLAYER_API: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const VISIONOS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15";

/// yt-dlp: where it is, and where to fetch it from when it isn't.
pub struct YtDlp {
    pub path: PathBuf,
    /// The release folder to fetch it from (and update it with); `None` to never fetch
    /// or update it (a copy the user keeps).
    pub release: Option<String>,
    /// One fetch or update at a time.
    busy: tokio::sync::Mutex<()>,
    /// Recently read manifests by video ID, so the browser menu, the video window,
    /// adding the download and running it don't spawn yt-dlp four times.
    cache: Mutex<HashMap<String, (Instant, Manifest)>>,
    /// Concurrent reads of the same video share one yt-dlp run.
    inflight: Mutex<HashMap<String, Arc<Notify>>>,
}

impl YtDlp {
    pub fn new(path: PathBuf, release: Option<String>) -> Self {
        Self {
            path,
            release,
            busy: tokio::sync::Mutex::new(()),
            cache: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// Reads a YouTube video: its title, length and formats. `None` means it was cancelled.
    pub async fn read(
        &self,
        client: &reqwest::Client,
        request: &Request,
        config: &EngineConfig,
        cancel: &CancellationToken,
    ) -> Result<Option<Manifest>, MediaJobError> {
        let key = video_id(&request.url).unwrap_or_else(|| request.url.clone());
        loop {
            if let Some(manifest) = self.cached(&key) {
                return Ok(Some(manifest));
            }
            let wait = {
                let mut inflight = self.inflight.lock().unwrap();
                if let Some(notify) = inflight.get(&key) {
                    Some(notify.clone())
                } else {
                    inflight.insert(key.clone(), Arc::new(Notify::new()));
                    None
                }
            };
            let Some(notify) = wait else {
                break;
            };
            tokio::select! {
                _ = cancel.cancelled() => return Ok(None),
                _ = notify.notified() => {}
            }
        }
        struct InflightGuard<'a> {
            ytdlp: &'a YtDlp,
            key: String,
        }
        impl Drop for InflightGuard<'_> {
            fn drop(&mut self) {
                if let Some(notify) = self.ytdlp.inflight.lock().unwrap().remove(&self.key) {
                    notify.notify_waiters();
                }
            }
        }
        let _guard = InflightGuard {
            ytdlp: self,
            key: key.clone(),
        };

        let result = self.read_uncached(client, request, config, cancel).await;
        if let Ok(Some(manifest)) = &result {
            let mut cache = self.cache.lock().unwrap();
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            if cache.len() >= MAX_CACHED
                && let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest);
            }
            cache.insert(key, (Instant::now(), manifest.clone()));
        }
        result
    }

    fn cached(&self, key: &str) -> Option<Manifest> {
        let mut cache = self.cache.lock().unwrap();
        let (at, manifest) = cache.get(key)?;
        if at.elapsed() < CACHE_TTL {
            Some(manifest.clone())
        } else {
            cache.remove(key);
            None
        }
    }

    async fn read_uncached(
        &self,
        client: &reqwest::Client,
        request: &Request,
        config: &EngineConfig,
        cancel: &CancellationToken,
    ) -> Result<Option<Manifest>, MediaJobError> {
        if self.release.is_some()
            && let Some(id) = video_id(&request.url)
        {
            match direct_player(client, &id, cancel).await {
                Ok(None) => return Ok(None),
                Ok(Some(manifest)) => return Ok(Some(manifest)),
                Err(_) => {}
            }
        }
        if !self.ensure(client, config, cancel).await? {
            return Ok(None);
        }
        let cookies = Url::parse(&request.url)
            .ok()
            .and_then(|url| request.cookies.header_for(&url));
        // Try the fast unauthenticated visionos client first (skips webpage & JS player
        // requests); fall back to full extraction with cookies when needed.
        if let Ok(Some(json)) = self.describe(&request.url, None, true, cancel).await
            && let Ok(manifest) = manifest_from(&json)
        {
            return Ok(Some(manifest));
        }
        let first = match self.describe(&request.url, None, false, cancel).await {
            Ok(None) => return Ok(None),
            Ok(Some(json)) => match manifest_from(&json) {
                Ok(manifest) => return Ok(Some(manifest)),
                Err(e) if cookies.is_none() => e,
                Err(_) => match self
                    .describe(&request.url, cookies.as_deref(), false, cancel)
                    .await
                {
                    Ok(None) => return Ok(None),
                    Ok(Some(json)) => return Ok(Some(manifest_from(&json)?)),
                    Err(e) => e,
                },
            },
            Err(e) if cookies.is_none() => e,
            Err(_) => match self
                .describe(&request.url, cookies.as_deref(), false, cancel)
                .await
            {
                Ok(None) => return Ok(None),
                Ok(Some(json)) => return Ok(Some(manifest_from(&json)?)),
                Err(e) => e,
            },
        };
        // A YouTube change is the usual cause; a newer yt-dlp usually copes.
        if self.release.is_none() || !self.update().await {
            return Err(first.into());
        }
        match self.describe(&request.url, None, false, cancel).await {
            Ok(None) => Ok(None),
            Ok(Some(json)) => match manifest_from(&json) {
                Ok(manifest) => Ok(Some(manifest)),
                Err(e) if cookies.is_none() => Err(e.into()),
                Err(_) => match self
                    .describe(&request.url, cookies.as_deref(), false, cancel)
                    .await
                {
                    Ok(None) => Ok(None),
                    Ok(Some(json)) => Ok(Some(manifest_from(&json)?)),
                    Err(e) => Err(e.into()),
                },
            },
            Err(e) if cookies.is_none() => Err(e.into()),
            Err(_) => match self
                .describe(&request.url, cookies.as_deref(), false, cancel)
                .await
            {
                Ok(None) => Ok(None),
                Ok(Some(json)) => Ok(Some(manifest_from(&json)?)),
                Err(e) => Err(e.into()),
            },
        }
    }

    /// Makes sure the program is there, fetching and checking it if it isn't. `false`
    /// means it was cancelled.
    pub async fn ensure(
        &self,
        client: &reqwest::Client,
        config: &EngineConfig,
        cancel: &CancellationToken,
    ) -> Result<bool, MediaJobError> {
        let _one = self.busy.lock().await;
        if self.path.is_file() {
            return Ok(true);
        }
        let Some(release) = &self.release else {
            return Err(MediaError::Unsupported(format!(
                "yt-dlp, which OmniDM needs for YouTube, isn't at {}",
                self.path.display()
            ))
            .into());
        };
        let bare = |url: String| Request {
            url,
            method: Method::Get,
            body: Vec::new(),
            headers: Vec::new(),
            cookies: CookieJar::new(),
        };
        let sums_url = format!("{release}{SUMS}");
        let mut sums = Vec::new();
        let config = EngineConfig {
            allow_html: false,
            ..config.clone()
        };
        if fetch(
            client,
            &bare(sums_url.clone()),
            &sums_url,
            None,
            &config,
            cancel,
            &mut sums,
            |_| {},
        )
        .await?
        .is_none()
        {
            return Ok(false);
        }
        let expected = sum_for(&String::from_utf8_lossy(&sums), PROGRAM).ok_or_else(|| {
            MediaError::Invalid(format!("yt-dlp's release doesn't list {PROGRAM}"))
        })?;

        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = self.path.with_extension("download");
        let program_url = format!("{release}{PROGRAM}");
        let mut file = std::fs::File::create(&partial)?;
        let fetched = fetch(
            client,
            &bare(program_url.clone()),
            &program_url,
            None,
            &config,
            cancel,
            &mut file,
            |_| {},
        )
        .await;
        drop(file);
        let fetched = match fetched {
            Ok(fetched) => fetched,
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                return Err(e.into());
            }
        };
        if fetched.is_none() {
            let _ = std::fs::remove_file(&partial);
            return Ok(false);
        }
        if sha256_of(&partial)? != expected {
            let _ = std::fs::remove_file(&partial);
            return Err(MediaError::Invalid(
                "the downloaded yt-dlp doesn't match its published checksum".into(),
            )
            .into());
        }
        std::fs::rename(&partial, &self.path)?;
        Ok(true)
    }

    /// yt-dlp's description of a video, as JSON. `Ok(None)`: cancelled.
    async fn describe(
        &self,
        url: &str,
        cookies: Option<&str>,
        fast: bool,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, MediaError> {
        let mut command = self.command();
        command.args([
            "--dump-single-json",
            "--no-playlist",
            "--no-warnings",
            "--ignore-config",
            "--js-runtimes",
            "deno",
            "--js-runtimes",
            "node",
            "--js-runtimes",
            "quickjs",
            "--js-runtimes",
            "bun",
        ]);
        if fast {
            command.args([
                "--extractor-args",
                "youtube:player_client=visionos;player_skip=webpage,configs,js",
            ]);
        }
        if let Some(cookies) = cookies {
            command.arg("--add-header").arg(format!("Cookie:{cookies}"));
        }
        command.arg("--").arg(url);
        let output = tokio::select! {
            _ = cancel.cancelled() => return Ok(None),
            output = tokio::time::timeout(READ_TIMEOUT, command.output()) => output,
        };
        let output = output
            .map_err(|_| MediaError::Unsupported("yt-dlp took too long to read the video".into()))?
            .map_err(|e| MediaError::Unsupported(format!("yt-dlp couldn't be started: {e}")))?;
        if !output.status.success() {
            return Err(MediaError::Unsupported(problem(&String::from_utf8_lossy(
                &output.stderr,
            ))));
        }
        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    }

    /// Updates the program to the latest release. `true` if it now runs.
    async fn update(&self) -> bool {
        let _one = self.busy.lock().await;
        let mut command = self.command();
        command.arg("-U");
        matches!(
            tokio::time::timeout(UPDATE_TIMEOUT, command.status()).await,
            Ok(Ok(status)) if status.success()
        )
    }

    fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.path);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // No console window flashing up.
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    }
}

/// Queries YouTube's VISIONOS player API directly (~300 ms) and translates its response
/// into the JSON shape [`manifest_from`] expects. Falls back to yt-dlp when the video needs
/// signature deciphering, cookies or age verification.
async fn direct_player(
    client: &reqwest::Client,
    id: &str,
    cancel: &CancellationToken,
) -> Result<Option<Manifest>, MediaError> {
    let body = serde_json::json!({
        "videoId": id,
        "context": {
            "client": {
                "clientName": "VISIONOS",
                "clientVersion": "0.1",
                "hl": "en",
                "gl": "US"
            }
        }
    });
    let send = client
        .post(PLAYER_API)
        .header("User-Agent", VISIONOS_UA)
        .header("Content-Type", "application/json")
        .header("Origin", "https://www.youtube.com")
        .body(body.to_string())
        .send();
    let response = tokio::select! {
        _ = cancel.cancelled() => return Ok(None),
        res = tokio::time::timeout(Duration::from_secs(6), send) => res
            .map_err(|_| MediaError::Unsupported("YouTube player API timed out".into()))?
            .map_err(|e| MediaError::Unsupported(e.to_string()))?,
    };
    if !response.status().is_success() {
        return Err(MediaError::Unsupported(format!(
            "HTTP {}",
            response.status()
        )));
    }
    let bytes = tokio::select! {
        _ = cancel.cancelled() => return Ok(None),
        val = response.bytes() => val.map_err(|e| MediaError::Invalid(e.to_string()))?,
    };
    let player: Value =
        serde_json::from_slice(&bytes).map_err(|e| MediaError::Invalid(e.to_string()))?;
    if player["playabilityStatus"]["status"].as_str() != Some("OK") {
        return Err(MediaError::Unsupported("not directly playable".into()));
    }
    let details = &player["videoDetails"];
    if details["videoId"].as_str() != Some(id) {
        return Err(MediaError::Invalid("wrong videoId".into()));
    }
    let is_live = details["isLive"].as_bool() == Some(true)
        || details["isLiveContent"].as_bool() == Some(true)
            && details["isUpcoming"].as_bool() == Some(true);
    let duration = details["lengthSeconds"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| *d > 0.0);

    let mut formats = Vec::new();
    for list in [
        &player["streamingData"]["formats"],
        &player["streamingData"]["adaptiveFormats"],
    ] {
        let Some(items) = list.as_array() else {
            continue;
        };
        for item in items {
            let Some(url) = item["url"].as_str() else {
                continue;
            };
            let mime = item["mimeType"].as_str().unwrap_or_default();
            let (kind_ext, codecs) = mime.split_once(';').unwrap_or((mime, ""));
            let (media_type, sub) = kind_ext.trim().split_once('/').unwrap_or(("", "mp4"));
            let is_audio = media_type == "audio";
            let is_video = media_type == "video";
            if !is_audio && !is_video {
                continue;
            }
            let ext = match (is_audio, sub.trim()) {
                (true, "mp4") => "m4a",
                (_, "webm") => "webm",
                _ => "mp4",
            };
            let codec_str = codecs
                .split_once("codecs=\"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(c, _)| c.trim())
                .unwrap_or_default();
            let (vcodec, acodec) = if is_video && codec_str.contains(',') {
                let (v, a) = codec_str.split_once(',').unwrap();
                (v.trim(), a.trim())
            } else if is_video {
                (
                    if codec_str.is_empty() { "avc1" } else { codec_str },
                    "none",
                )
            } else {
                (
                    "none",
                    if codec_str.is_empty() { "mp4a" } else { codec_str },
                )
            };
            let bitrate = item["averageBitrate"]
                .as_f64()
                .or_else(|| item["bitrate"].as_f64())
                .unwrap_or(0.0);
            let filesize = item["contentLength"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok());
            let language = item["audioTrack"]["id"]
                .as_str()
                .map(|id| id.split('.').next().unwrap_or(id));
            let is_default_audio = item["audioTrack"]["audioIsDefault"].as_bool() == Some(true);
            let itag = item["itag"]
                .as_i64()
                .map(|n| n.to_string())
                .unwrap_or_default();
            formats.push(serde_json::json!({
                "format_id": itag,
                "url": url,
                "protocol": "https",
                "ext": ext,
                "vcodec": vcodec,
                "acodec": acodec,
                "width": item["width"],
                "height": item["height"],
                "fps": item["fps"],
                "tbr": bitrate / 1000.0,
                "filesize": filesize,
                "language": language,
                "language_preference": if is_default_audio { 10 } else { -1 },
                "http_headers": {
                    "User-Agent": VISIONOS_UA
                }
            }));
        }
    }
    let synth = serde_json::json!({
        "id": id,
        "title": details["title"],
        "duration": duration,
        "is_live": is_live,
        "live_status": if is_live { "is_live" } else { "not_live" },
        "formats": formats
    });
    manifest_from(&synth.to_string()).map(Some)
}

/// Whether `url` is a YouTube video's page (a watch page, a short, an embed or a `youtu.be`
/// link).
pub fn is_youtube(url: &str) -> bool {
    video_id(url).is_some()
}

/// The video id in a YouTube link.
pub fn video_id(url: &str) -> Option<String> {
    let url = Url::parse(url.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .or_else(|| host.strip_prefix("music."))
        .unwrap_or(&host);
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    let id = match host {
        "youtu.be" => segments.next()?.to_owned(),
        "youtube.com" | "youtube-nocookie.com" => match segments.next()? {
            "watch" => url
                .query_pairs()
                .find(|(name, _)| name == "v")
                .map(|(_, v)| v.into_owned())?,
            "shorts" | "embed" | "live" | "v" => segments.next()?.to_owned(),
            _ => return None,
        },
        _ => return None,
    };
    let valid = id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then_some(id)
}

/// The sum listed for `file` in a `SHA2-256SUMS` file (`<hex>  <name>` lines).
fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, name) = line.trim().split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file).then(|| sum.to_ascii_lowercase())
    })
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// yt-dlp's complaint, made readable: its last `ERROR:` line without the prefixes.
fn problem(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp couldn't read the video");
    let text = line.trim_start_matches("ERROR:").trim();
    // "[youtube] dQw4w9WgXcQ: Video unavailable" → "Video unavailable".
    let text = match text
        .strip_prefix('[')
        .and_then(|t| t.split_once("]: ").or(t.split_once("] ")))
    {
        Some((_, rest)) => rest.split_once(": ").map_or(rest, |(_, why)| why),
        None => text,
    };
    format!("YouTube: {text}")
}

/// One usable format of a video.
struct Format {
    id: String,
    url: Url,
    ext: String,
    video: bool,
    audio: bool,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f64>,
    /// Bits per second.
    bandwidth: u64,
    /// The exact size, when YouTube tells it.
    size: Option<u64>,
    codecs: Option<String>,
    language: Option<String>,
    language_preference: i64,
    headers: Vec<(String, String)>,
}

/// A video as yt-dlp describes it (`--dump-single-json`), as a manifest: its picture
/// qualities best first (one per height and frame rate, preferring a single file with sound,
/// then MP4, then the higher bitrate) and its sound tracks (the best per language,
/// preferring M4A).
pub fn manifest_from(json: &str) -> Result<Manifest, MediaError> {
    let info: Value = serde_json::from_str(json)
        .map_err(|e| MediaError::Invalid(format!("yt-dlp's answer couldn't be read: {e}")))?;
    match info["live_status"].as_str() {
        Some("is_live") | Some("post_live") => return Err(MediaError::Live),
        Some("is_upcoming") => {
            return Err(MediaError::Unsupported(
                "This YouTube video hasn't started yet".into(),
            ));
        }
        _ if info["is_live"].as_bool() == Some(true) => return Err(MediaError::Live),
        _ => {}
    }
    let duration = info["duration"].as_f64();
    let title = info["title"].as_str().map(str::to_owned);

    let all = info["formats"].as_array().cloned().unwrap_or_default();
    let formats: Vec<Format> = all.iter().filter_map(format_of).collect();
    if formats.is_empty() {
        let locked = all.iter().any(|f| f["has_drm"].as_bool() == Some(true));
        return Err(if locked {
            MediaError::Protected("YouTube".into())
        } else {
            MediaError::Unsupported("YouTube offered no format OmniDM can download".into())
        });
    }

    // Sound: the best per language.
    let mut sounds: Vec<&Format> = Vec::new();
    for format in formats.iter().filter(|f| f.audio && !f.video) {
        let better =
            |a: &Format, b: &Format| (a.ext == "m4a", a.bandwidth) > (b.ext == "m4a", b.bandwidth);
        match sounds.iter_mut().find(|s| s.language == format.language) {
            Some(kept) if better(format, kept) => *kept = format,
            Some(_) => {}
            None => sounds.push(format),
        }
    }
    sounds.sort_by_key(|s| {
        (
            std::cmp::Reverse(s.language_preference),
            std::cmp::Reverse(s.bandwidth),
        )
    });
    let audio: Vec<AudioTrack> = sounds
        .iter()
        .enumerate()
        .map(|(index, sound)| AudioTrack {
            name: sound_name(sound, sounds.len()),
            language: sound.language.clone(),
            group: Some(AUDIO_GROUP.into()),
            bandwidth: Some(sound.bandwidth),
            codecs: sound.codecs.clone(),
            default: index == 0,
            source: Source::Track(track_of(sound, duration)),
        })
        .collect();

    // Pictures: the best per height and frame rate.
    let mut pictures: HashMap<(Option<u32>, bool), &Format> = HashMap::new();
    for format in formats.iter().filter(|f| f.video) {
        let key = (format.height, format.fps.is_some_and(|fps| fps > 30.5));
        let rank = |f: &Format| (f.audio, f.ext == "mp4", f.bandwidth);
        match pictures.get(&key) {
            Some(kept) if rank(kept) >= rank(format) => {}
            _ => {
                pictures.insert(key, format);
            }
        }
    }
    let mut qualities: Vec<Quality> = pictures
        .values()
        .map(|picture| Quality {
            bandwidth: picture.bandwidth,
            width: picture.width,
            height: picture.height,
            frame_rate: picture.fps,
            codecs: picture.codecs.clone(),
            // A format with sound needs no separate track.
            audio_group: (!picture.audio && !audio.is_empty()).then(|| AUDIO_GROUP.into()),
            source: Source::Track(track_of(picture, duration)),
        })
        .collect();
    qualities.sort_by_key(|q| {
        std::cmp::Reverse((
            q.height.unwrap_or(0),
            q.frame_rate.map(|f| f as u64),
            q.bandwidth,
        ))
    });

    let only_sound = qualities.is_empty();
    // Offer sound-only qualities too (one per bitrate/container on the default language),
    // so the "Sound only" tab and quality picker work on any YouTube video.
    let default_lang = sounds.first().and_then(|s| s.language.as_deref());
    let mut sound_only: Vec<&Format> = formats
        .iter()
        .filter(|f| f.audio && !f.video && f.language.as_deref() == default_lang)
        .collect();
    sound_only.sort_by_key(|s| std::cmp::Reverse((s.ext == "m4a", s.bandwidth)));
    sound_only.dedup_by(|a, b| a.ext == b.ext && a.bandwidth.abs_diff(b.bandwidth) < 8_000);
    for sound in sound_only {
        qualities.push(Quality {
            bandwidth: sound.bandwidth,
            width: None,
            height: None,
            frame_rate: None,
            codecs: sound.codecs.clone(),
            audio_group: None,
            source: Source::Track(track_of(sound, duration)),
        });
    }
    let audio = if only_sound { Vec::new() } else { audio };

    Ok(Manifest {
        kind: StreamKind::YouTube,
        title,
        duration,
        qualities,
        audio,
    })
}

/// A format yt-dlp lists, if OmniDM can download it: a plain web address (not a playlist
/// of its own, nor a storyboard of thumbnails), without DRM.
fn format_of(format: &Value) -> Option<Format> {
    let protocol = format["protocol"].as_str().unwrap_or("https");
    if !matches!(protocol, "https" | "http") || format["has_drm"].as_bool() == Some(true) {
        return None;
    }
    let url = Url::parse(format["url"].as_str()?).ok()?;
    let codec = |name: &str| {
        format[name]
            .as_str()
            .filter(|c| *c != "none")
            .map(str::to_owned)
    };
    let (vcodec, acodec) = (codec("vcodec"), codec("acodec"));
    let ext = format["ext"].as_str().unwrap_or("mp4").to_owned();
    if (vcodec.is_none() && acodec.is_none()) || ext == "mhtml" {
        return None;
    }
    let number = |name: &str| format[name].as_f64().filter(|n| *n >= 0.0);
    let headers = format["http_headers"]
        .as_object()
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    Some(Format {
        id: format["format_id"].as_str().unwrap_or_default().to_owned(),
        url,
        ext,
        video: vcodec.is_some(),
        audio: acodec.is_some(),
        width: number("width").map(|n| n as u32),
        height: number("height").map(|n| n as u32),
        fps: number("fps"),
        bandwidth: number("tbr").map_or(0, |kbit| (kbit * 1000.0) as u64),
        size: number("filesize").map(|n| n as u64).filter(|n| *n > 0),
        codecs: match (vcodec, acodec) {
            (Some(v), Some(a)) => Some(format!("{v}, {a}")),
            (v, a) => v.or(a),
        },
        language: format["language"].as_str().map(str::to_owned),
        language_preference: format["language_preference"].as_i64().unwrap_or(0),
        headers,
    })
}

fn sound_name(sound: &Format, count: usize) -> String {
    let base = sound.language.clone().unwrap_or_else(|| "Sound".into());
    if count > 1 && sound.language_preference >= 10 {
        format!("{base} (original)")
    } else {
        base
    }
}

/// A format as a track: pieces of [`PIECE_SIZE`] when its size is known, else one piece.
fn track_of(format: &Format, duration: Option<f64>) -> Track {
    let pieces = match format.size {
        Some(size) => (0..size.div_ceil(PIECE_SIZE))
            .map(|n| Piece {
                url: format.url.clone(),
                range: Some(ByteRange {
                    start: n * PIECE_SIZE,
                    end: ((n + 1) * PIECE_SIZE).min(size) - 1,
                }),
                key: None,
            })
            .collect(),
        None => vec![Piece {
            url: format.url.clone(),
            range: None,
            key: None,
        }],
    };
    Track {
        id: Some(format.id.clone()),
        headers: format.headers.clone(),
        init: None,
        pieces,
        container: if format.ext == "webm" {
            Container::WebM
        } else {
            Container::Mp4
        },
        duration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_links() {
        let id = |url| video_id(url);
        assert_eq!(
            id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(
            id("https://m.youtube.com/watch?feature=x&v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(
            id("https://youtu.be/dQw4w9WgXcQ?si=abc"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(
            id("https://www.youtube.com/shorts/aBcDeFgHiJk"),
            Some("aBcDeFgHiJk".into())
        );
        assert_eq!(
            id("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
        assert_eq!(
            id("https://music.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".into())
        );
        for not in [
            "https://www.youtube.com/",
            "https://www.youtube.com/@channel",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "file:///watch?v=dQw4w9WgXcQ",
        ] {
            assert_eq!(id(not), None, "{not}");
        }
    }

    #[test]
    fn checksums() {
        let sums = "abc123  yt-dlp\nDEF456  yt-dlp.exe\n789  yt-dlp_macos\n";
        assert_eq!(sum_for(sums, "yt-dlp.exe"), Some("def456".into()));
        assert_eq!(sum_for(sums, "yt-dlp"), Some("abc123".into()));
        assert_eq!(sum_for(sums, "yt-dlp_x86.exe"), None);
    }

    #[test]
    fn problems_read_plainly() {
        let stderr = "WARNING: something\nERROR: [youtube] dQw4w9WgXcQ: Video unavailable. This video is private\n";
        assert_eq!(
            problem(stderr),
            "YouTube: Video unavailable. This video is private"
        );
        assert_eq!(
            problem("ERROR: Unable to download webpage"),
            "YouTube: Unable to download webpage"
        );
        assert_eq!(problem(""), "YouTube: yt-dlp couldn't read the video");
    }

    fn format(id: &str, extra: Value) -> Value {
        let mut format = serde_json::json!({
            "format_id": id,
            "url": format!("https://rr1.googlevideo.com/videoplayback?itag={id}&sig=x"),
            "protocol": "https",
            "http_headers": {"User-Agent": "Mozilla/5.0 test", "Accept": "*/*"}
        });
        format
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        format
    }

    fn video(formats: Vec<Value>) -> String {
        serde_json::json!({
            "id": "dQw4w9WgXcQ",
            "title": "A song",
            "duration": 212.0,
            "live_status": "not_live",
            "formats": formats
        })
        .to_string()
    }

    #[test]
    fn formats_become_qualities_and_sound() {
        let json = video(vec![
            format(
                "sb0",
                serde_json::json!({"ext": "mhtml", "vcodec": "none", "acodec": "none", "protocol": "mhtml"}),
            ),
            format(
                "18",
                serde_json::json!({"ext": "mp4", "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "width": 640, "height": 360, "fps": 25, "tbr": 500.0, "filesize": 13_000_000}),
            ),
            format(
                "134",
                serde_json::json!({"ext": "mp4", "vcodec": "avc1.4d401e", "acodec": "none", "width": 640, "height": 360, "fps": 25, "tbr": 300.0, "filesize": 8_000_000}),
            ),
            format(
                "137",
                serde_json::json!({"ext": "mp4", "vcodec": "avc1.640028", "acodec": "none", "width": 1920, "height": 1080, "fps": 25, "tbr": 4000.0, "filesize": 25_000_000}),
            ),
            format(
                "248",
                serde_json::json!({"ext": "webm", "vcodec": "vp9", "acodec": "none", "width": 1920, "height": 1080, "fps": 25, "tbr": 2600.0, "filesize": 20_000_000}),
            ),
            format(
                "299",
                serde_json::json!({"ext": "mp4", "vcodec": "avc1.64002a", "acodec": "none", "width": 1920, "height": 1080, "fps": 50, "tbr": 6000.0, "filesize_approx": 60_000_000}),
            ),
            format(
                "140",
                serde_json::json!({"ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "tbr": 129.0, "filesize": 3_400_000, "language": "en", "language_preference": 10}),
            ),
            format(
                "251",
                serde_json::json!({"ext": "webm", "vcodec": "none", "acodec": "opus", "tbr": 140.0, "filesize": 3_500_000, "language": "en", "language_preference": 10}),
            ),
            format(
                "140-1",
                serde_json::json!({"ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "tbr": 129.0, "filesize": 3_400_000, "language": "de", "language_preference": -1}),
            ),
            format(
                "hls",
                serde_json::json!({"ext": "mp4", "vcodec": "avc1", "acodec": "mp4a", "height": 720, "protocol": "m3u8_native"}),
            ),
        ]);
        let manifest = manifest_from(&json).unwrap();
        assert_eq!(manifest.kind, StreamKind::YouTube);
        assert_eq!(manifest.title.as_deref(), Some("A song"));
        assert_eq!(manifest.duration, Some(212.0));

        let picked: Vec<_> = manifest
            .qualities
            .iter()
            .map(|q| match &q.source {
                Source::Track(t) => (q.height, t.id.clone().unwrap()),
                Source::Playlist(_) => unreachable!(),
            })
            .collect();
        // 1080p60, then 1080p (MP4 over WebM), then 360p (with its sound, one file), then sound-only.
        assert_eq!(
            picked,
            [
                (Some(1080), "299".to_owned()),
                (Some(1080), "137".to_owned()),
                (Some(360), "18".to_owned()),
                (None, "140".to_owned()),
                (None, "251".to_owned()),
            ]
        );
        assert_eq!(manifest.qualities[1].audio_group.as_deref(), Some("audio"));
        assert_eq!(
            manifest.qualities[2].audio_group, None,
            "360p has its sound"
        );
        assert_eq!(manifest.qualities[1].bandwidth, 4_000_000);

        // Sound: M4A per language, the original first.
        let sounds: Vec<_> = manifest
            .audio
            .iter()
            .map(|a| (a.name.as_str(), a.default))
            .collect();
        assert_eq!(sounds, [("en (original)", true), ("de", false)]);

        // Known sizes are cut into pieces; the track keeps its format and headers.
        let Source::Track(full_hd) = &manifest.qualities[1].source else {
            unreachable!()
        };
        assert_eq!(full_hd.pieces.len(), 3);
        assert_eq!(
            full_hd.pieces[2].range,
            Some(ByteRange {
                start: 2 * PIECE_SIZE,
                end: 25_000_000 - 1
            })
        );
        assert_eq!(full_hd.container, Container::Mp4);
        assert!(
            full_hd
                .headers
                .contains(&("User-Agent".into(), "Mozilla/5.0 test".into()))
        );
        // An approximate size can't be cut: one piece.
        let Source::Track(sixty) = &manifest.qualities[0].source else {
            unreachable!()
        };
        assert_eq!((sixty.pieces.len(), sixty.pieces[0].range), (1, None));
    }

    #[test]
    fn live_drm_and_music() {
        let live =
            serde_json::json!({"title": "x", "live_status": "is_live", "formats": []}).to_string();
        assert_eq!(manifest_from(&live), Err(MediaError::Live));
        let drm = video(vec![format(
            "1",
            serde_json::json!({"vcodec": "avc1", "acodec": "none", "has_drm": true}),
        )]);
        assert!(matches!(manifest_from(&drm), Err(MediaError::Protected(_))));
        let music = video(vec![format(
            "140",
            serde_json::json!({"ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "tbr": 129.0, "filesize": 100}),
        )]);
        let manifest = manifest_from(&music).unwrap();
        assert_eq!(manifest.qualities.len(), 1);
        assert!(manifest.audio.is_empty());
        assert!(manifest_from("not json").is_err());
    }
}
