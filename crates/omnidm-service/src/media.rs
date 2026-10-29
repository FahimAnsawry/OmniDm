// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs one video-stream download: reads its playlist, takes the chosen quality and sound,
//! downloads their pieces a few at a time into a work folder, unlocks locked ones, and
//! joins each track's pieces into one file (§12).
//!
//! The work folder sits next to the final file, as `<name>.omnidm-media`. Each track has a
//! folder in it holding `<n>.piece` files once a piece is complete, `<n>.part` while it
//! downloads, and `track-id`, a fingerprint of the track's piece list: pausing keeps the
//! pieces, and a playlist that changed since starts that track over.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use omnidm_core::engine::{EngineConfig, EngineError, Request};
use omnidm_core::fetch::fetch;
use omnidm_core::meter::RateMeter;
use omnidm_media::{
    AudioTrack, Container, Manifest, MediaError, Piece, Quality, Source, Track, Url, aes128, hls,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::ytdlp::YtDlp;

/// Appended to the file's name for its work folder.
pub const WORK_SUFFIX: &str = ".omnidm-media";

/// How often progress is reported.
const REPORT_EVERY: Duration = Duration::from_millis(500);

/// Which quality and sound to download. Kept with the download, so a resume takes the
/// same ones even if the playlist changed meanwhile (the closest, if they are gone).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    /// `None`: the best one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<QualityChoice>,
    /// `None`: the one the stream suggests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityChoice {
    pub height: Option<u32>,
    /// Bits per second.
    pub bandwidth: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioChoice {
    pub name: String,
    pub language: Option<String>,
}

/// One downloaded track, joined into one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joined {
    pub path: PathBuf,
    pub container: Container,
    /// A separate sound track (otherwise the picture, or the only track).
    pub sound: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaOutcome {
    /// The tracks, the picture (or only track) first.
    Done(Vec<Joined>),
    Paused,
}

#[derive(Debug)]
pub enum MediaJobError {
    Media(MediaError),
    Engine(EngineError),
    Io(std::io::Error),
}

impl fmt::Display for MediaJobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Media(e) => write!(f, "{e}"),
            Self::Engine(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "disk error: {e}"),
        }
    }
}

impl std::error::Error for MediaJobError {}

impl From<MediaError> for MediaJobError {
    fn from(e: MediaError) -> Self {
        Self::Media(e)
    }
}

impl From<EngineError> for MediaJobError {
    fn from(e: EngineError) -> Self {
        Self::Engine(e)
    }
}

impl From<std::io::Error> for MediaJobError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// What the job reports while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaEvent {
    /// A line for the download's log.
    Note(String),
    /// What it is downloading, to keep for a resume.
    Chose(Choice),
    Progress {
        saved: u64,
        /// An estimate until every piece is in.
        total: Option<u64>,
        speed: u64,
        /// Pieces downloading right now.
        active: u32,
    },
}

/// One video-stream download to run.
pub struct MediaJob {
    pub client: reqwest::Client,
    /// The playlist's address, with the page's headers and cookies.
    pub request: Request,
    pub work: PathBuf,
    pub choice: Choice,
    pub config: EngineConfig,
    /// Pieces downloading at once.
    pub parallel: usize,
    pub cancel: CancellationToken,
    /// For a YouTube video: what reads it, instead of a playlist.
    pub ytdlp: Option<Arc<YtDlp>>,
}

/// Reads the stream's playlist (a master playlist's qualities still point to theirs).
pub async fn read_playlist(
    client: &reqwest::Client,
    request: &Request,
    config: &EngineConfig,
    cancel: &CancellationToken,
) -> Result<Option<Manifest>, MediaJobError> {
    let Some((text, url)) = get_text(client, request, &request.url, config, cancel).await? else {
        return Ok(None);
    };
    Ok(Some(omnidm_media::read_manifest(&text, &url)?))
}

/// Runs `job`, telling `on_event` how it goes.
pub async fn run_media(
    job: MediaJob,
    mut on_event: impl FnMut(MediaEvent),
) -> Result<MediaOutcome, MediaJobError> {
    let MediaJob {
        client,
        request,
        work,
        choice,
        config,
        parallel,
        cancel,
        ytdlp,
    } = job;

    let manifest = match &ytdlp {
        Some(ytdlp) => {
            on_event(MediaEvent::Note("Asking yt-dlp about the video".into()));
            ytdlp.read(&client, &request, &config, &cancel).await?
        }
        None => {
            on_event(MediaEvent::Note("Reading the video's playlist".into()));
            read_playlist(&client, &request, &config, &cancel).await?
        }
    };
    let Some(manifest) = manifest else {
        return Ok(MediaOutcome::Paused);
    };
    let quality = pick_quality(&manifest.qualities, choice.quality.as_ref())
        .ok_or_else(|| MediaError::Invalid("it lists no qualities".into()))?;
    let sound = pick_audio(&manifest, quality, choice.audio.as_ref());
    on_event(MediaEvent::Note(describe(quality, sound)));
    on_event(MediaEvent::Chose(Choice {
        quality: Some(QualityChoice {
            height: quality.height,
            bandwidth: quality.bandwidth,
        }),
        audio: sound.map(|a| AudioChoice {
            name: a.name.clone(),
            language: a.language.clone(),
        }),
    }));

    // The tracks to download: the picture (or the only track), then a separate sound.
    let mut tracks = vec![("main", &quality.source)];
    if let Some(sound) = sound {
        tracks.push(("sound", &sound.source));
    }
    let mut ready = Vec::new();
    for (name, source) in tracks {
        let track = match source {
            Source::Track(track) => track.clone(),
            Source::Playlist(url) => {
                let Some((text, url)) =
                    get_text(&client, &request, url.as_str(), &config, &cancel).await?
                else {
                    return Ok(MediaOutcome::Paused);
                };
                hls::read_track(text.as_bytes(), &url)?
            }
        };
        ready.push((name, track));
    }

    std::fs::create_dir_all(&work)?;
    let mut plan = Plan::default();
    let mut joined = Vec::new();
    for (name, track) in &ready {
        let file = work.join(format!("{name}.{}", track.container.extension()));
        joined.push(Joined {
            path: file.clone(),
            container: track.container,
            sound: *name == "sound",
        });
        // Joined already, by a run that stopped before it could finish up.
        if file.is_file() && !work.join(name).exists() {
            continue;
        }
        plan.add(&work.join(name), track)?;
    }
    let total_pieces = plan.todo.len() + plan.done;
    on_event(MediaEvent::Note(if plan.done == 0 {
        format!("{total_pieces} pieces to download")
    } else {
        format!("{} of {total_pieces} pieces already downloaded", plan.done)
    }));

    let keys = match get_keys(&client, &request, &plan.todo, &config, &cancel).await? {
        Some(keys) => keys,
        None => return Ok(MediaOutcome::Paused),
    };
    if !download_pieces(
        &client,
        &request,
        plan,
        keys,
        &config,
        parallel,
        &cancel,
        &mut on_event,
    )
    .await?
    {
        return Ok(MediaOutcome::Paused);
    }

    on_event(MediaEvent::Note("Joining the pieces".into()));
    for (name, track) in &ready {
        let dir = work.join(name);
        if !dir.exists() {
            continue;
        }
        let file = work.join(format!("{name}.{}", track.container.extension()));
        let count = track.pieces.len();
        let has_init = track.init.is_some();
        tokio::task::spawn_blocking(move || join_track(&dir, has_init, count, &file))
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))??;
    }
    Ok(MediaOutcome::Done(joined))
}

/// The quality chosen before, or the closest to it: the same height first, then the
/// nearest bandwidth. Without a choice, the best.
pub fn pick_quality<'a>(
    qualities: &'a [Quality],
    choice: Option<&QualityChoice>,
) -> Option<&'a Quality> {
    let Some(choice) = choice else {
        return qualities.first();
    };
    qualities.iter().min_by_key(|q| {
        (
            q.height != choice.height,
            q.bandwidth.abs_diff(choice.bandwidth),
        )
    })
}

/// The sound track chosen before (by name, then by language), else the suggested one.
pub fn pick_audio<'a>(
    manifest: &'a Manifest,
    quality: &Quality,
    choice: Option<&AudioChoice>,
) -> Option<&'a AudioTrack> {
    if let Some(choice) = choice {
        let found = manifest
            .audio_for(quality)
            .find(|a| a.name == choice.name && a.language == choice.language)
            .or_else(|| {
                manifest
                    .audio_for(quality)
                    .find(|a| choice.language.is_some() && a.language == choice.language)
            });
        if found.is_some() {
            return found;
        }
    }
    manifest.default_audio(quality)
}

fn describe(quality: &Quality, sound: Option<&AudioTrack>) -> String {
    let mut text = match (quality.width, quality.height) {
        (Some(w), Some(h)) => format!("Downloading {w}×{h}"),
        (None, Some(h)) => format!("Downloading {h}p"),
        _ => "Downloading the stream".into(),
    };
    if quality.bandwidth > 0 {
        text.push_str(&format!(
            " ({:.1} Mbit/s)",
            quality.bandwidth as f64 / 1_000_000.0
        ));
    }
    if let Some(sound) = sound {
        text.push_str(&format!(" with the sound track \"{}\"", sound.name));
    }
    text
}

/// A playlist as text, and where it came from after redirects.
async fn get_text(
    client: &reqwest::Client,
    request: &Request,
    url: &str,
    config: &EngineConfig,
    cancel: &CancellationToken,
) -> Result<Option<(String, Url)>, MediaJobError> {
    // Some servers label playlists as web pages; reading them tells for sure.
    let config = EngineConfig {
        allow_html: true,
        ..config.clone()
    };
    let mut bytes = Vec::new();
    let Some(fetched) = fetch(
        client,
        request,
        url,
        None,
        &config,
        cancel,
        &mut bytes,
        |_| {},
    )
    .await?
    else {
        return Ok(None);
    };
    let url = Url::parse(&fetched.final_url)
        .map_err(|_| MediaError::Invalid(format!("a bad address: {}", fetched.final_url)))?;
    Ok(Some((String::from_utf8_lossy(&bytes).into_owned(), url)))
}

/// A piece still to download, and where it goes.
struct Todo {
    dir: PathBuf,
    /// `init` or the piece's number.
    name: String,
    piece: Piece,
    /// Its track's own request headers.
    headers: Arc<Vec<(String, String)>>,
}

/// The pieces left, over every track.
#[derive(Default)]
struct Plan {
    todo: Vec<Todo>,
    /// Pieces complete before this run.
    done: usize,
    /// Bytes on disk before this run, complete pieces or not.
    saved: u64,
    /// The total size of pieces whose size is known in advance (they have a byte range).
    known_size: u64,
    /// Pieces whose size is only known once downloaded, and of those, the ones complete
    /// and their bytes, to estimate the rest.
    unknown: usize,
    unknown_done: usize,
    unknown_done_bytes: u64,
}

impl Plan {
    /// Adds the pieces of `track` that `dir` doesn't have complete yet. A folder holding
    /// another piece list (the playlist changed) is emptied first.
    fn add(&mut self, dir: &Path, track: &Track) -> std::io::Result<()> {
        let id = fingerprint(track);
        let id_file = dir.join("track-id");
        if std::fs::read_to_string(&id_file).ok().as_deref() != Some(id.as_str()) {
            if dir.exists() {
                std::fs::remove_dir_all(dir)?;
            }
            std::fs::create_dir_all(dir)?;
            std::fs::write(&id_file, &id)?;
        }
        let headers = Arc::new(track.headers.clone());
        let named = track.init.iter().map(|p| ("init".to_owned(), p)).chain(
            track
                .pieces
                .iter()
                .enumerate()
                .map(|(i, p)| (piece_name(i), p)),
        );
        for (name, piece) in named {
            let size = |file: PathBuf| std::fs::metadata(file).map(|m| m.len()).ok();
            match &piece.range {
                Some(range) => self.known_size += range.size(),
                None => self.unknown += 1,
            }
            if let Some(bytes) = size(dir.join(format!("{name}.piece"))) {
                self.done += 1;
                self.saved += bytes;
                if piece.range.is_none() {
                    self.unknown_done += 1;
                    self.unknown_done_bytes += bytes;
                }
                continue;
            }
            self.saved += size(dir.join(format!("{name}.part"))).unwrap_or(0);
            self.todo.push(Todo {
                dir: dir.to_path_buf(),
                name,
                piece: piece.clone(),
                headers: headers.clone(),
            });
        }
        Ok(())
    }
}

fn piece_name(index: usize) -> String {
    format!("{index:06}")
}

/// Tells piece lists apart: the track's id, if it has one, and every piece's address
/// without its query (signed links get new tokens) and its byte range. FNV-1a, so it stays
/// the same across OmniDm versions.
fn fingerprint(track: &Track) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    if let Some(id) = &track.id {
        add(id.as_bytes());
        add(b"\n");
    }
    for piece in track.init.iter().chain(&track.pieces) {
        let mut url = piece.url.clone();
        url.set_query(None);
        add(url.as_str().as_bytes());
        if let Some(range) = piece.range {
            add(format!("{}-{}", range.start, range.end).as_bytes());
        }
        add(b"\n");
    }
    format!("{} {hash:016x}", track.pieces.len())
}

/// Fetches the AES-128 keys the remaining pieces need, each once.
async fn get_keys(
    client: &reqwest::Client,
    request: &Request,
    todo: &[Todo],
    config: &EngineConfig,
    cancel: &CancellationToken,
) -> Result<Option<HashMap<Url, Vec<u8>>>, MediaJobError> {
    let wanted: HashSet<&Url> = todo
        .iter()
        .filter_map(|t| t.piece.key.as_ref().map(|k| &k.url))
        .collect();
    let mut keys = HashMap::new();
    for url in wanted {
        let mut key = Vec::new();
        match fetch(
            client,
            request,
            url.as_str(),
            None,
            config,
            cancel,
            &mut key,
            |_| {},
        )
        .await
        {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(None),
            Err(e) => return Err(link_error(e).into()),
        }
        keys.insert(url.clone(), key);
    }
    Ok(Some(keys))
}

/// A piece's server refusing it partway through a stream means its link expired.
fn link_error(e: EngineError) -> EngineError {
    match e {
        EngineError::Status(401 | 403 | 410) => EngineError::LinkExpired,
        e => e,
    }
}

/// What the piece downloads share.
struct Shared {
    client: reqwest::Client,
    request: Request,
    config: EngineConfig,
    keys: HashMap<Url, Vec<u8>>,
    cancel: CancellationToken,
    /// Bytes on disk.
    saved: AtomicU64,
    active: AtomicU32,
}

/// Downloads every piece of `plan`, `parallel` at a time. `false` if it was paused.
#[allow(clippy::too_many_arguments)]
async fn download_pieces(
    client: &reqwest::Client,
    request: &Request,
    plan: Plan,
    keys: HashMap<Url, Vec<u8>>,
    config: &EngineConfig,
    parallel: usize,
    cancel: &CancellationToken,
    on_event: &mut impl FnMut(MediaEvent),
) -> Result<bool, MediaJobError> {
    // An error stops the other pieces without pausing the download.
    let stop = cancel.child_token();
    let shared = Arc::new(Shared {
        client: client.clone(),
        request: request.clone(),
        config: config.clone(),
        keys,
        cancel: stop.clone(),
        saved: AtomicU64::new(plan.saved),
        active: AtomicU32::new(0),
    });
    let mut unknown_done = plan.unknown_done;
    let mut unknown_done_bytes = plan.unknown_done_bytes;
    let estimate = |unknown_done: usize, unknown_done_bytes: u64| {
        if plan.unknown == 0 {
            Some(plan.known_size)
        } else if unknown_done > 0 {
            let average = unknown_done_bytes / unknown_done as u64;
            Some(plan.known_size + average * plan.unknown as u64)
        } else {
            None
        }
    };

    let mut queue: VecDeque<Todo> = plan.todo.into();
    let mut running = JoinSet::new();
    let mut meter = RateMeter::new(Duration::from_secs(3));
    let mut last_saved = shared.saved.load(Ordering::Relaxed);
    let mut failure = None;
    let mut report = tokio::time::interval(REPORT_EVERY);
    loop {
        while running.len() < parallel.max(1)
            && failure.is_none()
            && let Some(todo) = queue.pop_front()
        {
            running.spawn(get_piece(shared.clone(), todo));
        }
        if running.is_empty() {
            break;
        }
        tokio::select! {
            _ = report.tick() => {
                let saved = shared.saved.load(Ordering::Relaxed);
                let now = Instant::now();
                meter.add(now, saved.saturating_sub(last_saved));
                last_saved = saved;
                on_event(MediaEvent::Progress {
                    saved,
                    total: estimate(unknown_done, unknown_done_bytes),
                    speed: meter.rate(now),
                    active: shared.active.load(Ordering::Relaxed),
                });
            }
            Some(finished) = running.join_next() => match finished {
                Ok(Ok(Finished::Done { size, size_known })) => {
                    if !size_known {
                        unknown_done += 1;
                        unknown_done_bytes += size;
                    }
                }
                Ok(Ok(Finished::Stopped)) => {}
                Ok(Err(e)) => {
                    if failure.is_none() {
                        failure = Some(e);
                        stop.cancel();
                    }
                }
                Err(e) => {
                    if failure.is_none() {
                        failure = Some(MediaJobError::Io(std::io::Error::other(e.to_string())));
                        stop.cancel();
                    }
                }
            },
        }
    }
    let saved = shared.saved.load(Ordering::Relaxed);
    on_event(MediaEvent::Progress {
        saved,
        total: if failure.is_none() && !cancel.is_cancelled() {
            Some(saved)
        } else {
            estimate(unknown_done, unknown_done_bytes)
        },
        speed: 0,
        active: 0,
    });
    if let Some(e) = failure {
        return Err(e);
    }
    Ok(!cancel.is_cancelled())
}

enum Finished {
    Done { size: u64, size_known: bool },
    Stopped,
}

/// Downloads one piece into `<name>.part`, unlocks it if it is locked, and renames it
/// `<name>.piece`.
async fn get_piece(shared: Arc<Shared>, todo: Todo) -> Result<Finished, MediaJobError> {
    shared.active.fetch_add(1, Ordering::Relaxed);
    let result = async {
        let part = todo.dir.join(format!("{}.part", todo.name));
        let done = todo.dir.join(format!("{}.piece", todo.name));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part)?;
        // The track's own headers replace the page's of the same name.
        let own;
        let request = if todo.headers.is_empty() {
            &shared.request
        } else {
            let mut headers: Vec<(String, String)> = shared
                .request
                .headers
                .iter()
                .filter(|(name, _)| {
                    !todo
                        .headers
                        .iter()
                        .any(|(n, _)| n.eq_ignore_ascii_case(name))
                })
                .cloned()
                .collect();
            headers.extend(todo.headers.iter().cloned());
            own = Request {
                headers,
                ..shared.request.clone()
            };
            &own
        };
        let fetched = fetch(
            &shared.client,
            request,
            todo.piece.url.as_str(),
            todo.piece.range.map(|r| (r.start, r.end)),
            &shared.config,
            &shared.cancel,
            &mut file,
            |n| {
                shared.saved.fetch_add(n, Ordering::Relaxed);
            },
        )
        .await
        .map_err(link_error)?;
        drop(file);
        let Some(fetched) = fetched else {
            return Ok(Finished::Stopped);
        };
        let size = match &todo.piece.key {
            Some(key) => {
                let secret = shared
                    .keys
                    .get(&key.url)
                    .ok_or_else(|| MediaError::Invalid("a piece's key wasn't fetched".into()))?;
                let mut data = std::fs::read(&part)?;
                let locked = data.len() as u64;
                aes128::decrypt(secret, &key.iv, &mut data)?;
                // Written beside it first, so a piece is never half unlocked.
                let plain = todo.dir.join(format!("{}.plain", todo.name));
                std::fs::write(&plain, &data)?;
                std::fs::rename(&plain, &done)?;
                std::fs::remove_file(&part)?;
                shared
                    .saved
                    .fetch_sub(locked - data.len() as u64, Ordering::Relaxed);
                data.len() as u64
            }
            None => {
                std::fs::rename(&part, &done)?;
                fetched.size
            }
        };
        Ok(Finished::Done {
            size,
            size_known: todo.piece.range.is_some(),
        })
    }
    .await;
    shared.active.fetch_sub(1, Ordering::Relaxed);
    result
}

/// Joins a track's pieces, in order, into `file`, then removes its folder. Written under
/// another name first, so `file` only exists once complete.
fn join_track(dir: &Path, has_init: bool, count: usize, file: &Path) -> std::io::Result<()> {
    let partial = file.with_extension("joining");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&partial)?);
    let names = has_init
        .then(|| "init".to_owned())
        .into_iter()
        .chain((0..count).map(piece_name));
    for name in names {
        let mut piece = std::fs::File::open(dir.join(format!("{name}.piece")))?;
        std::io::copy(&mut piece, &mut out)?;
    }
    std::io::Write::flush(&mut out)?;
    drop(out);
    std::fs::rename(&partial, file)?;
    std::fs::remove_dir_all(dir)
}
