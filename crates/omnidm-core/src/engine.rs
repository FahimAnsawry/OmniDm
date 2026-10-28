// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs one download: probes the file, splits it across connections, writes every byte
//! at its offset in a pre-sized part file, and saves checkpoints so it can resume.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use omnidm_net::http::{
    RangeError, RequestedRange, StatusClass, backoff_delay, check_range_response, classify_status,
    parse_content_range, parse_retry_after,
};
use reqwest::Response;
use reqwest::header::{self, HeaderMap};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::meter::RateMeter;
use crate::ramp::{RampController, RampInput, RampPolicy};
use crate::segment::{Segment, SegmentId, SegmentMap, SplitPolicy};

const METER_WINDOW: Duration = Duration::from_secs(2);

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    /// Extra headers sent with every request (e.g. `Referer`, `Cookie`).
    pub headers: Vec<(String, String)>,
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

/// One download for [`download`] to run.
pub struct Download<'a> {
    pub client: &'a reqwest::Client,
    pub request: &'a Request,
    /// The `.omnidm-part` file the bytes are written to.
    pub part_path: &'a Path,
    /// Saved state to continue from; the part file must be the one it was written to.
    pub resume: Option<Resume>,
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
        match run.segmented(ctx, None).await {
            Err(EngineError::RemoteChanged) if config.restart_if_changed => {}
            other => return other,
        }
    }
    run.fresh(client, request, part_path).await
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
    ) -> Result<Outcome, EngineError> {
        let Some(response) = self.first_response(client, request).await? else {
            return Ok(Outcome::Paused);
        };
        let info = remote_info(&response);
        if !self.config.allow_html
            && info
                .content_type
                .as_deref()
                .is_some_and(|t| t.to_ascii_lowercase().starts_with("text/html"))
        {
            return Err(EngineError::HtmlInsteadOfFile);
        }
        self.checkpointer
            .probed(&info)
            .map_err(EngineError::Checkpoint)?;
        self.emit(Event::Probed(info.clone()));

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

    /// Sends the request for the whole file, retrying temporary failures.
    /// `None` means the download was cancelled while waiting to retry.
    async fn first_response(
        &self,
        client: &reqwest::Client,
        request: &Request,
    ) -> Result<Option<Response>, EngineError> {
        let everything = RequestedRange {
            start: 0,
            end: None,
        };
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (error, retry_after) =
                match send(client, request, &request.url, Some(everything), None).await {
                    Ok(response) => {
                        let status = response.status().as_u16();
                        match classify_status(status) {
                            StatusClass::Success => return Ok(Some(response)),
                            StatusClass::Retry => (EngineError::Status(status), None),
                            StatusClass::SlowDown => {
                                (EngineError::Status(status), retry_after(response.headers()))
                            }
                            StatusClass::AuthRequired | StatusClass::Fatal => {
                                return Err(EngineError::Status(status));
                            }
                        }
                    }
                    Err(e) => (EngineError::Network(e.to_string()), None),
                };
            if attempt >= self.config.retry.max_attempts {
                return Err(error);
            }
            if self.wait(self.retry_delay(attempt, retry_after)).await {
                return Ok(None);
            }
        }
    }
}

/// Longest `Retry-After` the engine will wait for.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(600);

impl Run<'_> {
    fn emit(&self, event: Event) {
        if let Some(tx) = &self.events {
            let _ = tx.send(event);
        }
    }

    /// The server's `Retry-After` if it gave one, otherwise jittered exponential back-off.
    fn retry_delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        let retry = self.config.retry;
        retry_after.map_or_else(
            || backoff_delay(attempt, retry.base_delay, retry.max_delay, fastrand::f64()),
            |d| d.min(MAX_RETRY_AFTER),
        )
    }

    /// Sleeps for `delay`; returns `true` if the download was cancelled meanwhile.
    async fn wait(&self, delay: Duration) -> bool {
        tokio::select! {
            _ = self.cancel.cancelled() => true,
            _ = tokio::time::sleep(delay) => false,
        }
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
        let mut ramp = RampController::new(config.ramp);
        let max_connections = config.ramp.max_connections.max(1) as usize;
        let mut last_checkpoint = Instant::now();
        let mut ticker = tokio::time::interval(config.tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let spawn = |workers: &mut JoinSet<_>, active: &mut HashSet<SegmentId>, id, response| {
            active.insert(id);
            let ctx = ctx.clone();
            workers.spawn(async move { (id, worker(ctx, id, response).await) });
        };
        if let Some((id, response)) = first {
            spawn(&mut workers, &mut active, id, Some(response));
        }

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    workers.shutdown().await;
                    self.checkpoint(&ctx)?;
                    return Ok(Outcome::Paused);
                }
                Some(joined) = workers.join_next(), if !workers.is_empty() => {
                    let (id, result) = joined.map_err(|e| EngineError::Network(e.to_string()))?;
                    active.remove(&id);
                    let Err(err) = result else { continue };
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
                    match attempts {
                        Some((count, hint)) if count < config.retry.max_attempts => {
                            retry_at.insert(id, Instant::now() + self.retry_delay(count, hint));
                        }
                        _ => {
                            workers.shutdown().await;
                            self.checkpoint(&ctx)?;
                            return Err(err.into_engine_error());
                        }
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
                        return Ok(Outcome::Completed { size: ctx.total });
                    }

                    for id in idle {
                        if active.len() >= max_connections {
                            break;
                        }
                        retry_at.remove(&id);
                        spawn(&mut workers, &mut active, id, None);
                    }

                    let wanted = ramp.wanted(RampInput {
                        now,
                        active: active.len() as u32,
                        speed: ctx.speed(now),
                        speed_limit: config.speed_limit,
                    });
                    let mut added = 0;
                    for _ in 0..wanted {
                        let Some(new_id) = ctx.split(config.split, now) else { break };
                        spawn(&mut workers, &mut active, new_id, None);
                        added += 1;
                    }
                    if added > 0 {
                        ramp.record_round(now);
                    }

                    let unsaved = ctx.shared.lock().unwrap().unsaved;
                    if now.duration_since(last_checkpoint) >= config.checkpoint_interval
                        || unsaved >= config.checkpoint_bytes
                    {
                        self.checkpoint(&ctx)?;
                        last_checkpoint = now;
                    }
                    self.progress(&ctx, active.len() as u32);
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
                Err(EngineError::Network(_)) if attempt < self.config.retry.max_attempts => {
                    if self.wait(self.retry_delay(attempt, None)).await {
                        return Ok(Outcome::Paused);
                    }
                    attempt += 1;
                    match self.first_response(client, request).await? {
                        Some(next) => response = next,
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
}

impl WorkerError {
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
            Self::Range(_) | Self::Io(_) => None,
        }
    }

    fn into_engine_error(self) -> EngineError {
        match self {
            Self::Network(e) => EngineError::Network(e),
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
            let wanted = RequestedRange {
                start: segment.pos(),
                end: Some(segment.end),
            };
            let response = send(
                &ctx.client,
                &ctx.request,
                &ctx.url,
                Some(wanted),
                ctx.validator.as_deref(),
            )
            .await
            .map_err(|e| WorkerError::Network(e.to_string()))?;
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
            response
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

async fn send(
    client: &reqwest::Client,
    request: &Request,
    url: &str,
    range: Option<RequestedRange>,
    if_range: Option<&str>,
) -> reqwest::Result<Response> {
    let mut builder = client.get(url);
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
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
    builder.send().await
}

fn remote_info(response: &Response) -> RemoteInfo {
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
        content_disposition: header_text(headers, header::CONTENT_DISPOSITION),
    }
}

/// The `If-Range` value: a strong ETag if there is one, otherwise the modification date.
fn validator(info: &RemoteInfo) -> Option<String> {
    info.etag
        .clone()
        .filter(|etag| !etag.starts_with("W/"))
        .or_else(|| info.last_modified.clone())
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    header_text(headers, header::RETRY_AFTER)
        .and_then(|value| parse_retry_after(&value, SystemTime::now()))
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
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
