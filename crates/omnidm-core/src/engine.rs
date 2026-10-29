// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs one download: probes the file, splits it across connections, writes every byte
//! at its offset in a pre-sized part file, and saves checkpoints so it can resume.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use omnidm_net::cookies::CookieJar;
use omnidm_net::http::{
    RangeError, RequestedRange, StatusClass, backoff_delay, check_range_response, classify_status,
    parse_content_range, parse_retry_after,
};
use omnidm_net::redirect::{
    MAX_REDIRECTS, Url, is_body_header, is_replayable_header, is_sent_cross_origin, next_hop,
    same_origin,
};
use reqwest::Response;
use reqwest::header::{self, HeaderMap};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::{AbortHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::meter::RateMeter;
use crate::ramp::{RampController, RampInput, RampPolicy};
use crate::segment::{Segment, SegmentId, SegmentMap, SplitPolicy};

const METER_WINDOW: Duration = Duration::from_secs(2);

/// The HTTP method a download is fetched with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Method {
    #[default]
    Get,
    /// A form submission that answers with the file; the body is sent on every request.
    Post,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }

    /// Parses `"GET"` or `"POST"` (any case).
    pub fn from_name(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("GET") {
            Some(Self::Get)
        } else if name.eq_ignore_ascii_case("POST") {
            Some(Self::Post)
        } else {
            None
        }
    }
}

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub method: Method,
    /// Sent with a `POST`; ignored for a `GET`.
    pub body: Vec<u8>,
    /// Extra headers sent with every request (e.g. `Referer`). Headers the engine controls,
    /// and `Cookie`, are ignored.
    pub headers: Vec<(String, String)>,
    /// The download's cookies. Each request sends the ones for its URL, and cookies the
    /// server sets are added.
    pub cookies: CookieJar,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    /// Attempts per segment before the download fails.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 10,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    pub split: SplitPolicy,
    pub ramp: RampPolicy,
    /// How often the controller checks progress, adds connections and restarts segments.
    pub tick: Duration,
    /// Save a checkpoint at least this often…
    pub checkpoint_interval: Duration,
    /// …or after this many new bytes, whichever comes first.
    pub checkpoint_bytes: u64,
    /// Bytes each connection collects before writing them to disk…
    pub write_buffer: usize,
    /// …but a connection never holds data longer than this, so slow connections still
    /// save progress (and show it) regularly.
    pub flush_interval: Duration,
    pub retry: RetryPolicy,
    /// A connection that receives nothing for this long is restarted.
    pub stall_timeout: Duration,
    /// A connection this much slower than the download's fastest (below a quarter of its
    /// speed) for this long, on a range too small to split, gets a duplicate request.
    pub hedge_after: Duration,
    /// Speed limit in bytes per second (used to stop adding connections near the limit).
    pub speed_limit: Option<u64>,
    /// Accept a `text/html` response as the file instead of treating it as an error page.
    pub allow_html: bool,
    /// When the file changed on the server, download it again from zero instead of failing.
    pub restart_if_changed: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            split: SplitPolicy::default(),
            ramp: RampPolicy::default(),
            tick: Duration::from_millis(250),
            checkpoint_interval: Duration::from_secs(2),
            checkpoint_bytes: 16 * 1024 * 1024,
            write_buffer: 1024 * 1024,
            flush_interval: Duration::from_secs(1),
            retry: RetryPolicy::default(),
            stall_timeout: Duration::from_secs(15),
            hedge_after: Duration::from_secs(8),
            speed_limit: None,
            allow_html: false,
            restart_if_changed: true,
        }
    }
}

/// What the first response revealed about the remote file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInfo {
    pub final_url: String,
    pub total: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// The server supports ranges, so the download can use several connections and resume.
    pub resumable: bool,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
    /// The method after redirects (a `303` turns a `POST` into a `GET`); every range
    /// request uses it.
    pub final_method: Method,
    /// The redirects followed to reach the file, in order.
    pub redirects: Vec<Redirect>,
}

/// One redirect: the status the server answered and where it sent the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub status: u16,
    pub to: String,
}

/// State saved earlier, used to continue a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    pub info: RemoteInfo,
    pub segments: Vec<Segment>,
}

/// Where the engine saves its progress.
pub trait Checkpointer: Send {
    /// Called once, after the first response.
    fn probed(&mut self, info: &RemoteInfo) -> Result<(), String>;
    /// Saves progress. Every byte counted in `segments` is already on disk.
    fn save(&mut self, segments: &[Segment]) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Probed(RemoteInfo),
    Progress {
        downloaded: u64,
        total: Option<u64>,
        /// Bytes per second.
        speed: u64,
        connections: u32,
    },
    /// Every segment, sent with each progress update of a segmented download.
    Segments(Vec<SegmentState>),
    /// Something worth a line in the download's log, such as a retry.
    Log(String),
}

/// A segment as the download's details show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentState {
    pub start: u64,
    /// Its last byte (inclusive), as in [`Segment`].
    pub end: u64,
    pub written: u64,
    /// A connection is working on it.
    pub active: bool,
    /// Bytes per second.
    pub speed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every byte is in the part file, which has `size` bytes.
    Completed { size: u64 },
    /// Stopped by the cancellation token; progress is saved.
    Paused,
}

#[derive(Debug)]
pub enum EngineError {
    /// The server answered with a status that retrying won't fix.
    Status(u16),
    /// The server sent a web page instead of the file (often a login or error page).
    HtmlInsteadOfFile,
    /// The file on the server changed and the old progress can't be used.
    RemoteChanged,
    /// The connection kept failing after every retry.
    Network(String),
    /// The server started refusing the download partway (`401`, `403` or `410`), as
    /// expired links do. Progress is saved; a fresh link or fresh cookies continue it.
    LinkExpired,
    /// The drive doesn't have room for the file; nothing was written.
    DiskFull {
        needed: u64,
        available: u64,
    },
    Io(std::io::Error),
    Checkpoint(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(s) => write!(f, "server answered HTTP {s}"),
            Self::HtmlInsteadOfFile => write!(f, "server returned a web page instead of the file"),
            Self::RemoteChanged => write!(f, "the file changed on the server"),
            Self::Network(e) => write!(f, "network error: {e}"),
            Self::LinkExpired => write!(
                f,
                "the server stopped accepting the link (it may have expired);                  progress is saved, so get a fresh link or cookies and try again"
            ),
            Self::DiskFull { needed, available } => write!(
                f,
                "not enough disk space: the file needs {needed} bytes, {available} are free"
            ),
            Self::Io(e) => write!(f, "disk error: {e}"),
            Self::Checkpoint(e) => write!(f, "could not save progress: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// The first response of a fresh download, before any byte is saved. Look at `info` (for
/// example to name the file), then pass it to [`download`], which keeps reading the body.
pub struct Probe {
    pub info: RemoteInfo,
    response: Response,
}

/// Sends the first request (the whole file), retrying temporary failures, and rejects a
/// web page sent instead of the file. `None` means it was cancelled while waiting to retry.
pub async fn probe(
    client: &reqwest::Client,
    request: &Request,
    config: &EngineConfig,
    cancel: &CancellationToken,
) -> Result<Option<Probe>, EngineError> {
    let Some((response, method, redirects)) =
        first_response(client, request, config, cancel).await?
    else {
        return Ok(None);
    };
    let info = remote_info(&response, method, redirects);
    if !config.allow_html
        && info
            .content_type
            .as_deref()
            .is_some_and(|t| t.to_ascii_lowercase().starts_with("text/html"))
    {
        return Err(EngineError::HtmlInsteadOfFile);
    }
    Ok(Some(Probe { info, response }))
}

/// One download for [`download`] to run.
pub struct Download<'a> {
    pub client: &'a reqwest::Client,
    pub request: &'a Request,
    /// The `.omnidm-part` file the bytes are written to.
    pub part_path: &'a Path,
    /// Saved state to continue from; the part file must be the one it was written to.
    pub resume: Option<Resume>,
    /// A fresh download's first response, if the caller already probed.
    pub probe: Option<Probe>,
    pub config: &'a EngineConfig,
    /// Receives progress updates.
    pub events: Option<UnboundedSender<Event>>,
    /// Cancelling pauses the download after a final checkpoint.
    pub cancel: CancellationToken,
}

/// Runs `job`, saving progress through `checkpointer`.
pub async fn download(
    job: Download<'_>,
    checkpointer: &mut dyn Checkpointer,
) -> Result<Outcome, EngineError> {
    let Download {
        client,
        request,
        part_path,
        resume,
        probe,
        config,
        events,
        cancel,
    } = job;
    let mut run = Run {
        config,
        checkpointer,
        events,
        cancel,
    };

    if let Some(ctx) = resume.and_then(|r| Ctx::resumed(client, request, part_path, r, config)) {
        let done: u64 = ctx
            .shared
            .lock()
            .unwrap()
            .map
            .segments()
            .iter()
            .map(|s| s.written)
            .sum();
        run.log(format!(
            "Continuing: {} of {} already downloaded",
            bytes(done),
            bytes(ctx.total)
        ));
        match run.segmented(ctx, None).await {
            Err(EngineError::RemoteChanged) if config.restart_if_changed => {
                run.log("The file changed on the server; starting again from zero".into());
            }
            other => return other,
        }
    }
    run.fresh(client, request, part_path, probe).await
}

/// The parts of a download run that every phase needs.
struct Run<'a> {
    config: &'a EngineConfig,
    checkpointer: &'a mut dyn Checkpointer,
    events: Option<UnboundedSender<Event>>,
    cancel: CancellationToken,
}

impl Run<'_> {
    /// Starts from zero: the first request asks for the whole file and keeps streaming.
    async fn fresh(
        &mut self,
        client: &reqwest::Client,
        request: &Request,
        part_path: &Path,
        probed: Option<Probe>,
    ) -> Result<Outcome, EngineError> {
        let probed = match probed {
            Some(probed) => probed,
            None => match probe(client, request, self.config, &self.cancel).await? {
                Some(probed) => probed,
                None => return Ok(Outcome::Paused),
            },
        };
        let Probe { info, response } = probed;
        self.checkpointer
            .probed(&info)
            .map_err(EngineError::Checkpoint)?;
        self.emit(Event::Probed(info.clone()));
        for hop in &info.redirects {
            self.log(format!("Redirected ({}) to {}", hop.status, hop.to));
        }
        self.log(format!(
            "The server sent {}{}; {}",
            info.total
                .map_or_else(|| "a file of unknown size".to_string(), bytes),
            info.content_type
                .as_deref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default(),
            if info.resumable {
                "it can be split into pieces and resumed"
            } else {
                "it can't be split or resumed, so it uses one connection"
            }
        ));

        if let Some(total) = info.total {
            check_free_space(part_path, total)?;
        }
        match info.total {
            Some(total) if info.resumable => {
                let file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(part_path)?;
                make_sparse(&file);
                file.set_len(total)?;
                let map = SegmentMap::new(total);
                let first = map.segments()[0].id;
                let ctx = Ctx::new(client, request, &info, total, file, map, self.config);
                self.segmented(ctx, Some((first, response))).await
            }
            _ => {
                self.single_stream(client, request, response, part_path, info.total)
                    .await
            }
        }
    }
}

/// Sends the request for the whole file, retrying temporary failures.
/// `None` means the download was cancelled while waiting to retry.
async fn first_response(
    client: &reqwest::Client,
    request: &Request,
    config: &EngineConfig,
    cancel: &CancellationToken,
) -> Result<Option<(Response, Method, Vec<Redirect>)>, EngineError> {
    let everything = RequestedRange {
        start: 0,
        end: None,
    };
    let mut attempt = 0;
    loop {
        attempt += 1;
        let (error, retry_after) = match send(
            client,
            request,
            &request.url,
            request.method,
            Some(everything),
            None,
        )
        .await
        {
            Ok((response, method, redirects)) => {
                let status = response.status().as_u16();
                match classify_status(status) {
                    StatusClass::Success => return Ok(Some((response, method, redirects))),
                    StatusClass::Retry => (EngineError::Status(status), None),
                    StatusClass::SlowDown => {
                        (EngineError::Status(status), retry_after(response.headers()))
                    }
                    StatusClass::AuthRequired | StatusClass::Fatal => {
                        return Err(EngineError::Status(status));
                    }
                }
            }
            Err(SendError::TooManyRedirects) => {
                return Err(EngineError::Network(
                    SendError::TooManyRedirects.to_string(),
                ));
            }
            Err(e) => (EngineError::Network(e.to_string()), None),
        };
        if attempt >= config.retry.max_attempts {
            return Err(error);
        }
        if wait(cancel, retry_delay(config, attempt, retry_after)).await {
            return Ok(None);
        }
    }
}

/// Longest `Retry-After` the engine will wait for.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(600);

/// The server's `Retry-After` if it gave one, otherwise jittered exponential back-off.
pub(crate) fn retry_delay(
    config: &EngineConfig,
    attempt: u32,
    retry_after: Option<Duration>,
) -> Duration {
    let retry = config.retry;
    retry_after.map_or_else(
        || backoff_delay(attempt, retry.base_delay, retry.max_delay, fastrand::f64()),
        |d| d.min(MAX_RETRY_AFTER),
    )
}

/// Sleeps for `delay`; returns `true` if `cancel` fired meanwhile.
pub(crate) async fn wait(cancel: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

impl Run<'_> {
    fn log(&self, line: String) {
        self.emit(Event::Log(line));
    }

    fn emit(&self, event: Event) {
        if let Some(tx) = &self.events {
            let _ = tx.send(event);
        }
    }

    fn retry_delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        retry_delay(self.config, attempt, retry_after)
    }

    async fn wait(&self, delay: Duration) -> bool {
        wait(&self.cancel, delay).await
    }

    /// Downloads with several connections, splitting the file as connections are added.
    async fn segmented(
        &mut self,
        ctx: Arc<Ctx>,
        first: Option<(SegmentId, Response)>,
    ) -> Result<Outcome, EngineError> {
        let config = self.config;
        let mut workers: JoinSet<(SegmentId, Result<(), WorkerError>)> = JoinSet::new();
        let mut active: HashSet<SegmentId> = HashSet::new();
        // Per segment: failed attempts since it last made progress, and its `written`
        // when that count started.
        let mut failures: HashMap<SegmentId, (u32, u64)> = HashMap::new();
        let mut retry_at: HashMap<SegmentId, Instant> = HashMap::new();
        // Set once the server refuses the link: running connections finish, no new ones.
        let mut link_expired = false;
        let mut ramp = RampController::new(config.ramp);
        // Lowered when the server says it is busy (429/503): never more connections than
        // it was serving at that moment, for the rest of this run.
        let mut max_connections = config.ramp.max_connections.max(1) as usize;
        let mut last_checkpoint = Instant::now();
        let mut ticker = tokio::time::interval(config.tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Duplicate requests for crawling segments (§5.4), at most one per segment.
        let mut duplicates: JoinSet<(SegmentId, u64, Result<Vec<u8>, WorkerError>)> =
            JoinSet::new();
        let mut duplicate_handles: HashMap<SegmentId, AbortHandle> = HashMap::new();
        let mut duplicated: HashSet<SegmentId> = HashSet::new();
        let mut slow_since: HashMap<SegmentId, Instant> = HashMap::new();
        let mut peak_speed = 0u64;
        // Lets the controller cancel a worker whose range a duplicate already finished.
        let mut worker_handles: HashMap<SegmentId, AbortHandle> = HashMap::new();

        let spawn = |workers: &mut JoinSet<_>,
                     active: &mut HashSet<SegmentId>,
                     handles: &mut HashMap<SegmentId, AbortHandle>,
                     id,
                     response| {
            active.insert(id);
            let ctx = ctx.clone();
            let handle = workers.spawn(async move { (id, worker(ctx, id, response).await) });
            handles.insert(id, handle);
        };
        if let Some((id, response)) = first {
            spawn(
                &mut workers,
                &mut active,
                &mut worker_handles,
                id,
                Some(response),
            );
        }

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    workers.shutdown().await;
                    self.checkpoint(&ctx)?;
                    return Ok(Outcome::Paused);
                }
                Some(joined) = workers.join_next(), if !workers.is_empty() => {
                    let (id, result) = match joined {
                        Ok(done) => done,
                        // Cancelled because its duplicate finished the range first.
                        Err(e) if e.is_cancelled() => {
                            let id = worker_handles
                                .iter()
                                .find(|(_, h)| h.id() == e.id())
                                .map(|(id, _)| *id);
                            if let Some(id) = id {
                                worker_handles.remove(&id);
                                active.remove(&id);
                            }
                            continue;
                        }
                        Err(e) => return Err(EngineError::Network(e.to_string())),
                    };
                    worker_handles.remove(&id);
                    active.remove(&id);
                    let Err(err) = result else {
                        // The original won the race: its duplicate is no longer needed.
                        if let Some(handle) = duplicate_handles.remove(&id) {
                            handle.abort();
                        }
                        continue;
                    };
                    if err.means_link_expired() {
                        if !link_expired {
                            self.log(format!(
                                "The server refused the link ({}); it may have expired",
                                err.describe()
                            ));
                        }
                        link_expired = true;
                        continue;
                    }
                    if err.means_server_busy() {
                        let lowered = max_connections.min(active.len().max(1));
                        if lowered < max_connections {
                            self.log(format!(
                                "The server is busy ({}); using at most {lowered} connection{}",
                                err.describe(),
                                if lowered == 1 { "" } else { "s" }
                            ));
                        }
                        max_connections = lowered;
                    }
                    let written = ctx.segment(id).map_or(0, |s| s.written);
                    let attempts = err.retry_hint().map(|hint| {
                        let (count, seen) = failures.entry(id).or_insert((0, written));
                        if written > *seen {
                            *count = 0;
                            *seen = written;
                        }
                        *count += 1;
                        (*count, hint)
                    });
                    let range = ctx.segment(id).map(|s| range_text(&s)).unwrap_or_default();
                    match attempts {
                        Some((count, hint)) if count < config.retry.max_attempts => {
                            let delay = self.retry_delay(count, hint);
                            self.log(format!(
                                "A connection failed ({}); trying {range} again in {}",
                                err.describe(),
                                seconds(delay)
                            ));
                            retry_at.insert(id, Instant::now() + delay);
                        }
                        _ => {
                            self.log(format!(
                                "A connection for {range} failed ({}); giving up",
                                err.describe()
                            ));
                            workers.shutdown().await;
                            self.checkpoint(&ctx)?;
                            return Err(err.into_engine_error());
                        }
                    }
                }
                Some(joined) = duplicates.join_next(), if !duplicates.is_empty() => {
                    // Cancelled (the original finished first) or failed: nothing to do.
                    let Ok((id, start, Ok(data))) = joined else { continue };
                    duplicate_handles.remove(&id);
                    if ctx.commit_duplicate(id, start, &data).map_err(EngineError::Io)?
                        && let Some(handle) = worker_handles.get(&id)
                    {
                        handle.abort();
                    }
                }
                _ = ticker.tick() => {
                    let now = Instant::now();
                    let (complete, idle) = {
                        let shared = ctx.shared.lock().unwrap();
                        let idle: Vec<SegmentId> = shared
                            .map
                            .segments()
                            .iter()
                            .filter(|s| !s.is_complete() && !active.contains(&s.id))
                            .filter(|s| retry_at.get(&s.id).is_none_or(|at| *at <= now))
                            .map(|s| s.id)
                            .collect();
                        (shared.map.is_complete(), idle)
                    };
                    if complete && active.is_empty() {
                        self.checkpoint(&ctx)?;
                        self.progress(&ctx, 0);
                        self.segments(&ctx, &active);
                        return Ok(Outcome::Completed { size: ctx.total });
                    }
                    if link_expired && active.is_empty() {
                        self.checkpoint(&ctx)?;
                        return Err(EngineError::LinkExpired);
                    }
                    let idle = if link_expired { Vec::new() } else { idle };

                    for id in idle {
                        if active.len() >= max_connections {
                            break;
                        }
                        retry_at.remove(&id);
                        spawn(&mut workers, &mut active, &mut worker_handles, id, None);
                    }

                    // A range waiting to retry means the server is struggling: don't add
                    // connections (or split that range) until it recovers.
                    let wanted = if retry_at.is_empty() && !link_expired {
                        ramp.wanted(RampInput {
                            now,
                            active: active.len() as u32,
                            speed: ctx.speed(now),
                            speed_limit: config.speed_limit,
                        })
                    } else {
                        0
                    };
                    let wanted = (wanted as usize).min(max_connections.saturating_sub(active.len()));
                    let mut added = 0;
                    for _ in 0..wanted {
                        let Some(new_id) = ctx.split(config.split, now) else { break };
                        spawn(&mut workers, &mut active, &mut worker_handles, new_id, None);
                        added += 1;
                        if let Some(segment) = ctx.segment(new_id) {
                            self.log(format!(
                                "Opened connection {} for {}",
                                active.len(),
                                range_text(&segment)
                            ));
                        }
                    }
                    if added > 0 {
                        ramp.record_round(now);
                    }

                    // A crawling connection on a range too small to split gets a duplicate.
                    if !link_expired && retry_at.is_empty() {
                        let speeds = ctx.speeds(now);
                        peak_speed = peak_speed.max(speeds.values().copied().max().unwrap_or(0));
                        for &id in &active {
                            let speed = speeds.get(&id).copied().unwrap_or(0);
                            if duplicated.contains(&id) || speed.saturating_mul(4) >= peak_speed {
                                slow_since.remove(&id);
                                continue;
                            }
                            let since = *slow_since.entry(id).or_insert(now);
                            let Some(segment) = ctx.segment(id) else { continue };
                            if now.duration_since(since) < config.hedge_after
                                || segment.is_complete()
                                || segment.remaining() >= 2 * config.split.min_segment
                            {
                                continue;
                            }
                            duplicated.insert(id);
                            self.log(format!(
                                "{} is going slowly; asking for it on a second connection too",
                                range_text(&segment)
                            ));
                            let handle = duplicates.spawn(duplicate(
                                ctx.clone(),
                                id,
                                segment.pos(),
                                segment.end,
                            ));
                            duplicate_handles.insert(id, handle);
                        }
                    }

                    let unsaved = ctx.shared.lock().unwrap().unsaved;
                    if now.duration_since(last_checkpoint) >= config.checkpoint_interval
                        || unsaved >= config.checkpoint_bytes
                    {
                        self.checkpoint(&ctx)?;
                        last_checkpoint = now;
                    }
                    self.progress(&ctx, active.len() as u32);
                    self.segments(&ctx, &active);
                }
            }
        }
    }

    /// Downloads a file that can't be split or resumed, front to back on one connection.
    /// Such a file can't continue mid-way, so a broken transfer starts again from zero.
    async fn single_stream(
        &mut self,
        client: &reqwest::Client,
        request: &Request,
        mut response: Response,
        part_path: &Path,
        expected: Option<u64>,
    ) -> Result<Outcome, EngineError> {
        let mut attempt = 1;
        loop {
            match self.stream_once(response, part_path, expected).await {
                Err(EngineError::Network(e)) if attempt < self.config.retry.max_attempts => {
                    let delay = self.retry_delay(attempt, None);
                    self.log(format!(
                        "The connection broke ({e}); starting again from zero in {}",
                        seconds(delay)
                    ));
                    if self.wait(delay).await {
                        return Ok(Outcome::Paused);
                    }
                    attempt += 1;
                    match first_response(client, request, self.config, &self.cancel).await? {
                        Some((next, _, _)) => response = next,
                        None => return Ok(Outcome::Paused),
                    }
                }
                other => return other,
            }
        }
    }

    async fn stream_once(
        &mut self,
        mut response: Response,
        part_path: &Path,
        expected: Option<u64>,
    ) -> Result<Outcome, EngineError> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(part_path)?;
        let mut written = 0u64;
        let mut buf: Vec<u8> = Vec::with_capacity(self.config.write_buffer);
        let mut meter = RateMeter::new(METER_WINDOW);
        let mut last_progress = Instant::now();
        let mut last_flush = Instant::now();

        loop {
            let chunk = tokio::select! {
                _ = self.cancel.cancelled() => return Ok(Outcome::Paused),
                chunk = tokio::time::timeout(self.config.stall_timeout, response.chunk()) => chunk,
            };
            let chunk = chunk
                .map_err(|_| EngineError::Network("the server stopped sending data".into()))?
                .map_err(|e| EngineError::Network(e.to_string()))?;
            let Some(bytes) = chunk else { break };
            buf.extend_from_slice(&bytes);
            if buf.len() >= self.config.write_buffer
                || last_flush.elapsed() >= self.config.flush_interval
            {
                last_flush = Instant::now();
                write_all_at(&file, &buf, written)?;
                written += buf.len() as u64;
                meter.add(Instant::now(), buf.len() as u64);
                buf.clear();
            }
            if last_progress.elapsed() >= self.config.tick {
                last_progress = Instant::now();
                self.emit(Event::Progress {
                    downloaded: written,
                    total: expected,
                    speed: meter.rate(last_progress),
                    connections: 1,
                });
            }
        }
        write_all_at(&file, &buf, written)?;
        written += buf.len() as u64;
        file.sync_data()?;
        if expected.is_some_and(|total| total != written) {
            return Err(EngineError::Network(
                "the connection closed before the whole file arrived".into(),
            ));
        }
        if written > 0 {
            let whole = SegmentMap::from_parts(
                written,
                vec![Segment {
                    id: SegmentId(0),
                    start: 0,
                    end: written - 1,
                    written,
                }],
            )
            .expect("one complete segment is a valid layout");
            self.checkpointer
                .save(whole.segments())
                .map_err(EngineError::Checkpoint)?;
        }
        self.emit(Event::Progress {
            downloaded: written,
            total: Some(written),
            speed: meter.rate(Instant::now()),
            connections: 0,
        });
        Ok(Outcome::Completed { size: written })
    }

    /// Flushes the part file to disk, then saves the segments that were already written.
    fn checkpoint(&mut self, ctx: &Ctx) -> Result<(), EngineError> {
        let segments = {
            let mut shared = ctx.shared.lock().unwrap();
            shared.unsaved = 0;
            shared.map.segments().to_vec()
        };
        ctx.file.sync_data()?;
        self.checkpointer
            .save(&segments)
            .map_err(EngineError::Checkpoint)
    }

    /// Sends every segment's state, unless nobody is listening.
    fn segments(&self, ctx: &Ctx, active: &HashSet<SegmentId>) {
        if self.events.is_none() {
            return;
        }
        let speeds = ctx.speeds(Instant::now());
        let segments = ctx
            .shared
            .lock()
            .unwrap()
            .map
            .segments()
            .iter()
            .map(|s| SegmentState {
                start: s.start,
                end: s.end,
                written: s.written,
                active: active.contains(&s.id),
                speed: if active.contains(&s.id) {
                    speeds.get(&s.id).copied().unwrap_or(0)
                } else {
                    0
                },
            })
            .collect();
        self.emit(Event::Segments(segments));
    }

    fn progress(&self, ctx: &Ctx, connections: u32) {
        let now = Instant::now();
        let downloaded = {
            let shared = ctx.shared.lock().unwrap();
            shared.map.segments().iter().map(|s| s.written).sum()
        };
        self.emit(Event::Progress {
            downloaded,
            total: Some(ctx.total),
            speed: ctx.speed(now),
            connections,
        });
    }
}

/// State shared between the controller and the connections of one download.
struct Ctx {
    client: reqwest::Client,
    request: Request,
    /// URL after redirects; every segment request goes here.
    url: String,
    /// Method after redirects, used by every range request.
    method: Method,
    /// Sent as `If-Range`, so a changed file is never mixed with old bytes.
    validator: Option<String>,
    total: u64,
    file: File,
    shared: Mutex<Shared>,
    write_buffer: usize,
    flush_interval: Duration,
    stall_timeout: Duration,
}

struct Shared {
    map: SegmentMap,
    meters: HashMap<SegmentId, RateMeter>,
    /// Bytes written since the last checkpoint.
    unsaved: u64,
}

impl Ctx {
    fn new(
        client: &reqwest::Client,
        request: &Request,
        info: &RemoteInfo,
        total: u64,
        file: File,
        map: SegmentMap,
        config: &EngineConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            client: client.clone(),
            request: request.clone(),
            url: info.final_url.clone(),
            method: info.final_method,
            validator: validator(info),
            total,
            file,
            shared: Mutex::new(Shared {
                map,
                meters: HashMap::new(),
                unsaved: 0,
            }),
            write_buffer: config.write_buffer,
            flush_interval: config.flush_interval,
            stall_timeout: config.stall_timeout,
        })
    }

    /// Continues from saved state, if it is usable: the download supports ranges, the
    /// segments form a valid map, and the part file exists with the full size.
    fn resumed(
        client: &reqwest::Client,
        request: &Request,
        part_path: &Path,
        resume: Resume,
        config: &EngineConfig,
    ) -> Option<Arc<Self>> {
        let total = resume.info.total.filter(|_| resume.info.resumable)?;
        let map = SegmentMap::from_parts(total, resume.segments).ok()?;
        let file = OpenOptions::new().write(true).open(part_path).ok()?;
        if file.metadata().ok()?.len() != total {
            return None;
        }
        Some(Self::new(
            client,
            request,
            &resume.info,
            total,
            file,
            map,
            config,
        ))
    }

    fn segment(&self, id: SegmentId) -> Option<Segment> {
        self.shared.lock().unwrap().map.get(id).copied()
    }

    fn speed(&self, now: Instant) -> u64 {
        let shared = self.shared.lock().unwrap();
        shared.meters.values().map(|m| m.rate(now)).sum()
    }

    /// Each segment's current speed, in bytes per second.
    fn speeds(&self, now: Instant) -> HashMap<SegmentId, u64> {
        let shared = self.shared.lock().unwrap();
        shared
            .meters
            .iter()
            .map(|(id, m)| (*id, m.rate(now)))
            .collect()
    }

    /// Writes a duplicate's bytes (which begin at `start`) from where the segment has got
    /// to. Returns `true` once the segment is complete.
    fn commit_duplicate(&self, id: SegmentId, start: u64, data: &[u8]) -> std::io::Result<bool> {
        let mut shared = self.shared.lock().unwrap();
        let Some(segment) = shared.map.get(id).copied() else {
            return Ok(false);
        };
        let pos = segment.pos();
        let Some(rest) = pos
            .checked_sub(start)
            .and_then(|skip| data.get(skip as usize..))
        else {
            return Ok(segment.is_complete());
        };
        let take = (rest.len() as u64).min(segment.remaining()) as usize;
        if take > 0 {
            write_all_at(&self.file, &rest[..take], pos)?;
            shared
                .map
                .record_written(id, take as u64)
                .expect("take never exceeds the segment's remaining bytes");
            shared.unsaved += take as u64;
        }
        Ok(shared.map.get(id).is_some_and(Segment::is_complete))
    }

    /// Splits the largest remaining range and returns the new segment.
    fn split(&self, policy: SplitPolicy, now: Instant) -> Option<SegmentId> {
        let mut shared = self.shared.lock().unwrap();
        let shared = &mut *shared;
        let meters = &shared.meters;
        let plan = shared
            .map
            .plan_split(policy, |id| meters.get(&id).map_or(0, |m| m.rate(now)))?;
        shared.map.apply_split(plan).ok()
    }

    /// Writes buffered bytes at the segment's position, never past its (possibly shrunk)
    /// end, and only then counts them. Returns `true` once the segment is complete.
    fn flush(&self, id: SegmentId, buf: &mut Vec<u8>) -> std::io::Result<bool> {
        let now = Instant::now();
        let mut shared = self.shared.lock().unwrap();
        let segment = *shared
            .map
            .get(id)
            .expect("a worker's segment stays in the map");
        let take = (buf.len() as u64).min(segment.remaining()) as usize;
        if take > 0 {
            write_all_at(&self.file, &buf[..take], segment.pos())?;
            shared
                .map
                .record_written(id, take as u64)
                .expect("take never exceeds the segment's remaining bytes");
            shared
                .meters
                .entry(id)
                .or_insert_with(|| RateMeter::new(METER_WINDOW))
                .add(now, take as u64);
            shared.unsaved += take as u64;
        }
        buf.clear();
        Ok(shared.map.get(id).is_some_and(Segment::is_complete))
    }
}

#[derive(Debug)]
enum WorkerError {
    Network(String),
    Status {
        code: u16,
        retry_after: Option<Duration>,
    },
    Range(RangeError),
    Io(std::io::Error),
    /// Fails the same way every time, such as a redirect loop.
    Fatal(String),
}

impl WorkerError {
    /// The server is overloaded or rate-limiting (`429`, `503`).
    fn means_server_busy(&self) -> bool {
        matches!(
            self,
            Self::Status {
                code: 429 | 503,
                ..
            }
        )
    }

    /// The server refuses this link now although it worked at first.
    fn means_link_expired(&self) -> bool {
        matches!(
            self,
            Self::Status {
                code: 401 | 403 | 410,
                ..
            }
        )
    }

    /// `Some(retry_after)` if trying again may help, `None` if it can't.
    fn retry_hint(&self) -> Option<Option<Duration>> {
        match self {
            Self::Network(_) => Some(None),
            Self::Status { code, retry_after } => match classify_status(*code) {
                StatusClass::Retry => Some(None),
                StatusClass::SlowDown => Some(*retry_after),
                _ => None,
            },
            // A server that answers the wrong range may do better on the next try.
            Self::Range(RangeError::Mismatch(_) | RangeError::MissingContentRange) => Some(None),
            Self::Range(_) | Self::Io(_) | Self::Fatal(_) => None,
        }
    }

    /// A short description for the log.
    fn describe(&self) -> String {
        match self {
            Self::Network(e) | Self::Fatal(e) => e.clone(),
            Self::Status { code, .. } | Self::Range(RangeError::Status(code)) => {
                format!("HTTP {code}")
            }
            Self::Range(RangeError::Mismatch(_) | RangeError::MissingContentRange) => {
                "the server answered a different range".into()
            }
            Self::Range(_) => "the file changed on the server".into(),
            Self::Io(e) => e.to_string(),
        }
    }

    fn into_engine_error(self) -> EngineError {
        match self {
            Self::Network(e) | Self::Fatal(e) => EngineError::Network(e),
            Self::Status { code, .. } | Self::Range(RangeError::Status(code)) => {
                EngineError::Status(code)
            }
            Self::Range(RangeError::Mismatch(_) | RangeError::MissingContentRange) => {
                EngineError::Network("the server answered a different range".into())
            }
            Self::Range(_) => EngineError::RemoteChanged,
            Self::Io(e) => EngineError::Io(e),
        }
    }
}

/// One connection: downloads its segment until it is complete.
async fn worker(
    ctx: Arc<Ctx>,
    id: SegmentId,
    response: Option<Response>,
) -> Result<(), WorkerError> {
    let mut response = match response {
        Some(response) => response,
        None => {
            let Some(segment) = ctx.segment(id).filter(|s| !s.is_complete()) else {
                return Ok(());
            };
            open_range(&ctx, segment.pos(), segment.end).await?
        }
    };

    let mut buf = Vec::with_capacity(ctx.write_buffer);
    let mut last_flush = Instant::now();
    loop {
        let chunk = tokio::time::timeout(ctx.stall_timeout, response.chunk())
            .await
            .map_err(|_| WorkerError::Network("the server stopped sending data".into()))?
            .map_err(|e| WorkerError::Network(e.to_string()))?;
        match chunk {
            Some(bytes) => {
                buf.extend_from_slice(&bytes);
                if buf.len() >= ctx.write_buffer || last_flush.elapsed() >= ctx.flush_interval {
                    last_flush = Instant::now();
                    if ctx.flush(id, &mut buf).map_err(WorkerError::Io)? {
                        return Ok(());
                    }
                }
            }
            None => {
                if ctx.flush(id, &mut buf).map_err(WorkerError::Io)? {
                    return Ok(());
                }
                return Err(WorkerError::Network(
                    "the connection closed before the segment was complete".into(),
                ));
            }
        }
    }
}

/// Requests bytes `start..=end` and checks that the server answers exactly that range.
async fn open_range(ctx: &Ctx, start: u64, end: u64) -> Result<Response, WorkerError> {
    let wanted = RequestedRange {
        start,
        end: Some(end),
    };
    let (response, _, _) = send(
        &ctx.client,
        &ctx.request,
        &ctx.url,
        ctx.method,
        Some(wanted),
        ctx.validator.as_deref(),
    )
    .await
    .map_err(|e| match e {
        SendError::Network(e) => WorkerError::Network(e.to_string()),
        SendError::TooManyRedirects => WorkerError::Fatal(e.to_string()),
    })?;
    let status = response.status().as_u16();
    if !matches!(status, 200 | 206 | 416) {
        return Err(WorkerError::Status {
            code: status,
            retry_after: retry_after(response.headers()),
        });
    }
    let content_range = header_text(response.headers(), header::CONTENT_RANGE);
    check_range_response(status, content_range.as_deref(), wanted, Some(ctx.total))
        .map_err(WorkerError::Range)?;
    Ok(response)
}

/// A second request for a crawling segment's last bytes, `start..=end`, kept in memory
/// (the range is small) for [`Ctx::commit_duplicate`].
async fn duplicate(
    ctx: Arc<Ctx>,
    id: SegmentId,
    start: u64,
    end: u64,
) -> (SegmentId, u64, Result<Vec<u8>, WorkerError>) {
    let result = async {
        let mut response = open_range(&ctx, start, end).await?;
        let len = (end - start + 1) as usize;
        let mut data = Vec::with_capacity(len);
        while data.len() < len {
            let chunk = tokio::time::timeout(ctx.stall_timeout, response.chunk())
                .await
                .map_err(|_| WorkerError::Network("the server stopped sending data".into()))?
                .map_err(|e| WorkerError::Network(e.to_string()))?;
            match chunk {
                Some(bytes) => data.extend_from_slice(&bytes),
                None => break,
            }
        }
        data.truncate(len);
        Ok(data)
    }
    .await;
    (id, start, result)
}

#[derive(Debug)]
pub(crate) enum SendError {
    Network(reqwest::Error),
    /// More than [`MAX_REDIRECTS`] hops, usually a loop; retrying won't help.
    TooManyRedirects,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(e) => write!(f, "{e}"),
            Self::TooManyRedirects => write!(f, "too many redirects (more than {MAX_REDIRECTS})"),
        }
    }
}

/// Sends a request to `url`, following redirects by hand. Captured headers the engine
/// controls are left out, and credentials are only sent to the request's own origin.
/// Returns the response, the method its request was sent with, and the redirects followed.
pub(crate) async fn send(
    client: &reqwest::Client,
    request: &Request,
    url: &str,
    mut method: Method,
    range: Option<RequestedRange>,
    if_range: Option<&str>,
) -> Result<(Response, Method, Vec<Redirect>), SendError> {
    let origin = Url::parse(&request.url).ok();
    let mut url = url.to_string();
    let mut redirects = Vec::new();
    for _ in 0..=MAX_REDIRECTS {
        let target = Url::parse(&url).ok();
        let same_site = match (&origin, &target) {
            (Some(origin), Some(target)) => same_origin(origin, target),
            _ => false,
        };
        let mut builder = match method {
            Method::Get => client.get(&url),
            Method::Post => client.post(&url).body(request.body.clone()),
        };
        for (name, value) in &request.headers {
            let allowed = is_replayable_header(name)
                && (same_site || is_sent_cross_origin(name))
                && (method == Method::Post || !is_body_header(name));
            if allowed {
                builder = builder.header(name.as_str(), value.as_str());
            }
        }
        if let Some(range) = range {
            let value = match range.end {
                Some(end) => format!("bytes={}-{end}", range.start),
                None => format!("bytes={}-", range.start),
            };
            builder = builder.header(header::RANGE, value);
        }
        if let Some(validator) = if_range {
            builder = builder.header(header::IF_RANGE, validator);
        }
        if let Some(cookies) = target.as_ref().and_then(|t| request.cookies.header_for(t)) {
            builder = builder.header(header::COOKIE, cookies);
        }
        let response = builder.send().await.map_err(SendError::Network)?;
        if let Some(target) = &target {
            let set_cookies = response.headers().get_all(header::SET_COOKIE);
            request
                .cookies
                .store_response(target, set_cookies.iter().filter_map(|v| v.to_str().ok()));
        }
        let location = header_text(response.headers(), header::LOCATION);
        let hop = target
            .as_ref()
            .and_then(|t| next_hop(t, response.status().as_u16(), location.as_deref()));
        match hop {
            Some(hop) => {
                url = hop.url.to_string();
                redirects.push(Redirect {
                    status: response.status().as_u16(),
                    to: url.clone(),
                });
                if hop.switch_to_get {
                    method = Method::Get;
                }
            }
            None => return Ok((response, method, redirects)),
        }
    }
    Err(SendError::TooManyRedirects)
}

fn remote_info(response: &Response, final_method: Method, redirects: Vec<Redirect>) -> RemoteInfo {
    let headers = response.headers();
    let (total, resumable) = if response.status().as_u16() == 206 {
        let range = header_text(headers, header::CONTENT_RANGE)
            .as_deref()
            .and_then(parse_content_range);
        let total = range.and_then(|r| r.total);
        let starts_at_zero = range.is_some_and(|r| r.start == 0);
        (total, starts_at_zero && total.is_some_and(|t| t > 0))
    } else {
        let total = header_text(headers, header::CONTENT_LENGTH).and_then(|v| v.parse().ok());
        (total, false)
    };
    RemoteInfo {
        final_url: response.url().to_string(),
        total,
        etag: header_text(headers, header::ETAG),
        last_modified: header_text(headers, header::LAST_MODIFIED),
        resumable,
        content_type: header_text(headers, header::CONTENT_TYPE),
        content_disposition: headers
            .get(header::CONTENT_DISPOSITION)
            .map(|v| decode_header_bytes(v.as_bytes())),
        final_method,
        redirects,
    }
}

/// The `If-Range` value: a strong ETag if there is one, otherwise the modification date.
fn validator(info: &RemoteInfo) -> Option<String> {
    info.etag
        .clone()
        .filter(|etag| !etag.starts_with("W/"))
        .or_else(|| info.last_modified.clone())
}

pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    header_text(headers, header::RETRY_AFTER)
        .and_then(|value| parse_retry_after(&value, SystemTime::now()))
}

/// Header bytes as text: UTF-8 if valid (many servers send raw UTF-8 file names),
/// otherwise ISO-8859-1, the old HTTP default.
fn decode_header_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

pub(crate) fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Fails with [`EngineError::DiskFull`] if the drive holding `part_path` has less than
/// `needed` bytes free. Part files are sparse, so without this check a full drive would
/// only show up as a write error partway through.
fn check_free_space(part_path: &Path, needed: u64) -> Result<(), EngineError> {
    let dir = match part_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    match free_space(dir) {
        Some(available) if available < needed => Err(EngineError::DiskFull { needed, available }),
        _ => Ok(()),
    }
}

/// Bytes the current user may still write on the drive holding `dir`.
#[cfg(windows)]
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the two unused outputs
    // may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

/// Other systems skip the check for now: a full drive shows up as a write error.
#[cfg(not(windows))]
fn free_space(_dir: &Path) -> Option<u64> {
    None
}

/// Marks `file` sparse, so writing far into it doesn't make Windows zero-fill everything
/// before the write first (seconds of stalls and double the disk writes on big files).
/// Best effort: file systems without sparse files, such as FAT32, work without it.
#[cfg(windows)]
fn make_sparse(file: &File) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;

    let mut returned = 0u32;
    // SAFETY: the handle stays valid while `file` is borrowed. FSCTL_SET_SPARSE uses no
    // input or output buffer, and without an OVERLAPPED the call completes synchronously.
    unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
    }
}

/// Unix file systems already leave unwritten areas as holes.
#[cfg(not(windows))]
fn make_sparse(_file: &File) {}

/// Writes all of `buf` at `offset`, independent of the file cursor.
fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_write(file, buf, offset)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::write_at(file, buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

/// "bytes 1,048,576–2,097,151" for the log.
fn range_text(segment: &Segment) -> String {
    format!("bytes {}–{}", segment.pos(), segment.end)
}

/// "1.5 MB" for the log.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// "3 s" or "0.5 s" for the log.
fn seconds(delay: Duration) -> String {
    let secs = delay.as_secs_f64();
    if secs < 10.0 && secs.fract() != 0.0 {
        format!("{secs:.1} s")
    } else {
        format!("{} s", secs.round())
    }
}
