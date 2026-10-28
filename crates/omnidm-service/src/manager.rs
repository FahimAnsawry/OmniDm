// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs the app's downloads: browser captures (probed before they are accepted),
//! downloads added by the CLI, pause and resume, and a waiting line beyond `max_active`.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use omnidm_core::engine::{
    EngineConfig, EngineError, Event, Method, Probe, Request, Resume, probe,
};
use omnidm_ipc::protocol::{
    AddRequest, BrowserCookie, Capture, CaptureResult, DownloadSummary, FinishStatus, Finished,
    Header, Progress, RejectReason,
};
use omnidm_net::cookies::{BrowserCookieSpec, CookieJar, Url};
use omnidm_net::naming::{choose_file_name, sanitize_file_name};
use omnidm_store::checkpoint::resume_from;
use omnidm_store::{DownloadId, NewDownload, Status, Store, StoreError};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::job::{Job, JobError, JobOutcome, run_job};

/// Name of the database file inside the data folder.
pub const DATABASE_FILE: &str = "omnidm.db";

/// How long resuming waits for the browser to send fresh cookies.
const FRESH_COOKIES_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Holds the database.
    pub data_dir: PathBuf,
    /// Captured downloads are saved here.
    pub downloads_dir: PathBuf,
    /// Downloads running at once; more wait their turn. Captures always start at once,
    /// because someone just clicked them.
    pub max_active: usize,
    pub engine: EngineConfig,
    /// A capture whose probe takes longer goes back to the browser.
    pub probe_timeout: Duration,
}

/// What the manager reports while it works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Progress(Progress),
    Finished(Finished),
}

/// Where fresh cookies come from when a download resumes (the connected browser).
pub trait CookieSource: Send + Sync {
    fn cookies_for(
        &self,
        url: String,
    ) -> Pin<Box<dyn Future<Output = Option<Vec<BrowserCookie>>> + Send>>;
}

/// The app's download manager. Clones share the same manager.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

struct Inner {
    config: ManagerConfig,
    database: PathBuf,
    client: reqwest::Client,
    store: Mutex<Store>,
    state: Mutex<State>,
    updates: broadcast::Sender<Update>,
    cookie_source: Mutex<Option<Arc<dyn CookieSource>>>,
    show: Mutex<Option<ShowHandler>>,
}

/// Opens the app's window.
pub type ShowHandler = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    running: HashMap<i64, CancellationToken>,
    waiting: VecDeque<Pending>,
}

/// A download ready to run.
struct Pending {
    id: i64,
    dir: PathBuf,
    file_name: Option<String>,
    probe: Option<Probe>,
    resume: Option<Resume>,
    request: Request,
    connections: Option<u32>,
}

impl Manager {
    /// Opens the database; downloads a crash left "running" become paused.
    pub fn start(config: ManagerConfig) -> Result<Self, StoreError> {
        std::fs::create_dir_all(&config.data_dir).map_err(io_error)?;
        let database = config.data_dir.join(DATABASE_FILE);
        let mut store = Store::open(&database)?;
        for record in store.list_downloads()? {
            if matches!(record.status, Status::Running | Status::Probing) {
                store.set_status(record.id, Status::Paused, None)?;
            }
        }
        let client = omnidm_net::download_client()
            .map_err(|e| StoreError::Corrupt(format!("can't create the HTTP client: {e}")))?;
        let (updates, _) = broadcast::channel(1024);
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                database,
                client,
                store: Mutex::new(store),
                state: Mutex::new(State::default()),
                updates,
                cookie_source: Mutex::new(None),
                show: Mutex::new(None),
            }),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Update> {
        self.inner.updates.subscribe()
    }

    /// Sets (or clears) where fresh cookies come from when resuming.
    pub fn set_cookie_source(&self, source: Option<Arc<dyn CookieSource>>) {
        *self.inner.cookie_source.lock().unwrap() = source;
    }

    /// Sets what opening the app's window does.
    pub fn set_show_handler(&self, handler: ShowHandler) {
        *self.inner.show.lock().unwrap() = Some(handler);
    }

    /// Opens the app's window, if there is one.
    pub fn show(&self) {
        let handler = self.inner.show.lock().unwrap().clone();
        if let Some(handler) = handler {
            handler();
        }
    }

    /// Forgets `source` if it is the current one (the browser disconnected).
    pub fn clear_cookie_source(&self, source: &Arc<dyn CookieSource>) {
        let mut current = self.inner.cookie_source.lock().unwrap();
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, source)) {
            *current = None;
        }
    }

    /// Running or waiting to run.
    pub fn is_active(&self, id: i64) -> bool {
        let state = self.inner.state.lock().unwrap();
        state.running.contains_key(&id) || state.waiting.iter().any(|p| p.id == id)
    }

    /// Takes over a download from the browser, but only if OmniDm can fetch it itself:
    /// the probe runs first, and a failure hands the download back to the browser.
    pub async fn capture(&self, capture: Capture) -> CaptureResult {
        let prepared = match prepare_capture(&capture) {
            Ok(prepared) => prepared,
            Err(detail) => return rejected(RejectReason::Invalid, detail),
        };
        let inner = &self.inner;
        let cancel = CancellationToken::new();
        let probed = tokio::time::timeout(
            inner.config.probe_timeout,
            probe(
                &inner.client,
                &prepared.request,
                &inner.config.engine,
                &cancel,
            ),
        )
        .await;
        let first = match probed {
            Ok(Ok(Some(first))) => first,
            Ok(Ok(None)) => return rejected(RejectReason::ProbeFailed, "cancelled".into()),
            Ok(Err(e)) => return rejected(RejectReason::ProbeFailed, e.to_string()),
            Err(_) => {
                return rejected(RejectReason::ProbeFailed, "the server took too long".into());
            }
        };
        let name = capture
            .suggested_name
            .as_deref()
            .map(sanitize_file_name)
            .unwrap_or_else(|| {
                choose_file_name(
                    first.info.content_disposition.as_deref(),
                    &first.info.final_url,
                    first.info.content_type.as_deref(),
                )
            });
        let dir = inner.config.downloads_dir.clone();
        let id = match self.record_capture(&capture, &prepared, &dir, Some(name.clone())) {
            Ok(id) => id,
            Err(e) => return rejected(RejectReason::NotAccepting, e.to_string()),
        };
        self.start_now(Pending {
            id,
            dir,
            file_name: Some(name),
            probe: Some(first),
            resume: None,
            request: prepared.request,
            connections: None,
        });
        CaptureResult::Accepted { download_id: id }
    }

    /// Takes several links at once; they are checked when their turn comes.
    pub fn capture_many(&self, items: Vec<Capture>) -> Vec<CaptureResult> {
        items
            .into_iter()
            .map(|capture| {
                let prepared = match prepare_capture(&capture) {
                    Ok(prepared) => prepared,
                    Err(detail) => return rejected(RejectReason::Invalid, detail),
                };
                let dir = self.inner.config.downloads_dir.clone();
                let name = capture.suggested_name.as_deref().map(sanitize_file_name);
                match self.record_capture(&capture, &prepared, &dir, name.clone()) {
                    Ok(id) => {
                        self.enqueue(Pending {
                            id,
                            dir,
                            file_name: name,
                            probe: None,
                            resume: None,
                            request: prepared.request,
                            connections: None,
                        });
                        CaptureResult::Accepted { download_id: id }
                    }
                    Err(e) => rejected(RejectReason::NotAccepting, e.to_string()),
                }
            })
            .collect()
    }

    /// Adds a download (from the CLI); it starts now or when a slot frees up.
    pub fn add(&self, add: AddRequest) -> Result<i64, String> {
        let url = Url::parse(&add.url).map_err(|e| format!("bad URL: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("only http and https links can be downloaded".into());
        }
        let dir = PathBuf::from(&add.dir);
        if !dir.is_absolute() {
            return Err("the folder must be an absolute path".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("can't create the folder: {e}"))?;
        let body = match &add.data {
            Some(data) => base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "the form data is not valid base64".to_string())?,
            None => Vec::new(),
        };
        let method = if add.data.is_some() {
            Method::Post
        } else {
            Method::Get
        };
        let (cookie_headers, mut headers): (Vec<_>, Vec<_>) = add
            .headers
            .iter()
            .map(|h| (h.name.clone(), h.value.clone()))
            .partition(|(name, _)| name.eq_ignore_ascii_case("cookie"));
        if method == Method::Post && !has_header(&headers, "content-type") {
            headers.push((
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ));
        }
        let given: Vec<BrowserCookie> = add
            .cookies
            .iter()
            .chain(cookie_headers.iter().map(|(_, v)| v))
            .flat_map(|pairs| cookies_from_pairs(&url, pairs))
            .collect();
        let file_name = add.file_name.as_deref().map(sanitize_file_name);
        let mut store = self.inner.store.lock().unwrap();
        let id = store
            .insert_download(&NewDownload {
                url: add.url.clone(),
                save_dir: dir.to_string_lossy().into_owned(),
                file_name: file_name.clone(),
                referrer: None,
                user_agent: None,
                headers: headers.clone(),
                method,
                body: body.clone(),
            })
            .map_err(|e| e.to_string())?;
        // Kept (encrypted) so a paused download can resume with them.
        if !given.is_empty() {
            let json = serde_json::to_vec(&given).map_err(|e| e.to_string())?;
            store.save_cookies(id, &json).map_err(|e| e.to_string())?;
        }
        drop(store);
        let id = id.0;
        self.enqueue(Pending {
            id,
            dir,
            file_name,
            probe: None,
            resume: None,
            request: Request {
                url: add.url,
                method,
                body,
                headers,
                cookies: jar_from(&given),
            },
            connections: add.connections,
        });
        Ok(id)
    }

    /// Pauses a running or waiting download. `false` if it is neither.
    pub fn pause(&self, id: i64) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(cancel) = state.running.get(&id) {
            cancel.cancel();
            return true;
        }
        let Some(at) = state.waiting.iter().position(|p| p.id == id) else {
            return false;
        };
        state.waiting.remove(at);
        drop(state);
        let _ = self
            .inner
            .store
            .lock()
            .unwrap()
            .set_status(DownloadId(id), Status::Paused, None);
        self.finished(id, FinishStatus::Paused, None, None);
        true
    }

    /// Continues a paused or failed download, with fresh cookies from the browser when it
    /// is connected, else the saved ones.
    pub async fn resume(&self, id: i64) -> Result<(), String> {
        {
            let state = self.inner.state.lock().unwrap();
            if state.running.contains_key(&id) || state.waiting.iter().any(|p| p.id == id) {
                return Ok(());
            }
        }
        let (record, segments, saved) = {
            let store = self.inner.store.lock().unwrap();
            let record = store
                .get_download(DownloadId(id))
                .map_err(|e| e.to_string())?
                .ok_or("no such download")?;
            let segments = store.load_segments(record.id).map_err(|e| e.to_string())?;
            let saved = store.load_cookies(record.id).map_err(|e| e.to_string())?;
            (record, segments, saved)
        };
        if record.status == Status::Completed {
            return Err("the download is already complete".into());
        }
        let source = self.inner.cookie_source.lock().unwrap().clone();
        let fresh = match source {
            Some(source) => {
                tokio::time::timeout(FRESH_COOKIES_WAIT, source.cookies_for(record.url.clone()))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        let cookies: Vec<BrowserCookie> = match (&fresh, &saved) {
            (Some(fresh), _) => fresh.clone(),
            (None, Some(saved)) => serde_json::from_slice(saved).unwrap_or_default(),
            (None, None) => Vec::new(),
        };
        // Fresh cookies replace saved ones; nothing is saved for private downloads.
        if let (Some(fresh), Some(_)) = (&fresh, &saved)
            && let Ok(json) = serde_json::to_vec(fresh)
        {
            let _ = self
                .inner
                .store
                .lock()
                .unwrap()
                .save_cookies(record.id, &json);
        }
        let resume = resume_from(&record, segments).filter(|_| record.file_name.is_some());
        self.inner
            .store
            .lock()
            .unwrap()
            .set_status(record.id, Status::Queued, None)
            .map_err(|e| e.to_string())?;
        self.enqueue(Pending {
            id,
            dir: PathBuf::from(&record.save_dir),
            file_name: record.file_name.clone(),
            probe: None,
            resume,
            request: Request {
                url: record.url.clone(),
                method: record.method,
                body: record.body.clone(),
                headers: record.headers.clone(),
                cookies: jar_from(&cookies),
            },
            connections: None,
        });
        Ok(())
    }

    /// Every download, oldest first.
    pub fn list(&self) -> Vec<DownloadSummary> {
        let store = self.inner.store.lock().unwrap();
        let Ok(records) = store.list_downloads() else {
            return Vec::new();
        };
        records
            .into_iter()
            .map(|record| {
                let downloaded = store
                    .load_segments(record.id)
                    .map(|segments| segments.iter().map(|s| s.written).sum())
                    .unwrap_or(0);
                DownloadSummary {
                    download_id: record.id.0,
                    url: record.url,
                    file_name: record.file_name,
                    save_dir: record.save_dir,
                    status: record.status.as_str().into(),
                    total: record.total_size,
                    downloaded,
                    error: record.error,
                }
            })
            .collect()
    }

    fn record_capture(
        &self,
        capture: &Capture,
        prepared: &PreparedCapture,
        dir: &Path,
        file_name: Option<String>,
    ) -> Result<i64, StoreError> {
        let mut store = self.inner.store.lock().unwrap();
        let id = store.insert_download(&NewDownload {
            url: capture.url.clone(),
            save_dir: dir.to_string_lossy().into_owned(),
            file_name,
            referrer: capture.referrer.clone(),
            user_agent: None,
            headers: prepared.request.headers.clone(),
            method: prepared.request.method,
            body: prepared.request.body.clone(),
        })?;
        if !capture.private && !capture.cookies.is_empty() {
            let json = serde_json::to_vec(&capture.cookies)
                .map_err(|e| StoreError::Corrupt(e.to_string()))?;
            store.save_cookies(id, &json)?;
        }
        Ok(id.0)
    }

    fn enqueue(&self, pending: Pending) {
        let mut state = self.inner.state.lock().unwrap();
        if state.running.len() < self.inner.config.max_active.max(1) {
            drop(state);
            self.start_now(pending);
        } else {
            state.waiting.push_back(pending);
        }
    }

    fn start_now(&self, pending: Pending) {
        let cancel = CancellationToken::new();
        self.inner
            .state
            .lock()
            .unwrap()
            .running
            .insert(pending.id, cancel.clone());
        let mut config = self.inner.config.engine.clone();
        if let Some(connections) = pending.connections {
            config.ramp.max_connections = connections.clamp(1, omnidm_core::ramp::HARD_CAP);
        }
        let id = pending.id;
        let job = Job {
            database: self.inner.database.clone(),
            id: DownloadId(id),
            dir: pending.dir,
            file_name: pending.file_name,
            probe: pending.probe,
            resume: pending.resume,
            client: self.inner.client.clone(),
            request: pending.request,
            config,
            cancel,
        };
        let manager = self.clone();
        let runtime = tokio::runtime::Handle::current();
        // The engine's future holds the job's database connection, which may not move
        // between threads, so each job runs on its own blocking-pool thread. Its
        // connections still run on the runtime's workers.
        tokio::task::spawn_blocking(move || {
            let updates = manager.inner.updates.clone();
            let result = runtime.block_on(run_job(job, |event| {
                if let Event::Progress {
                    downloaded,
                    total,
                    speed,
                    connections,
                } = *event
                {
                    let _ = updates.send(Update::Progress(Progress {
                        download_id: id,
                        downloaded,
                        total,
                        speed,
                        connections,
                    }));
                }
            }));
            match result {
                Ok(JobOutcome::Completed(path)) => manager.finished(
                    id,
                    FinishStatus::Completed,
                    Some(path.to_string_lossy().into_owned()),
                    None,
                ),
                Ok(JobOutcome::Paused) => manager.finished(id, FinishStatus::Paused, None, None),
                Err(JobError::Engine(e @ EngineError::LinkExpired)) => {
                    manager.finished(id, FinishStatus::Paused, None, Some(e.to_string()));
                }
                Err(e) => manager.finished(id, FinishStatus::Failed, None, Some(e.to_string())),
            }
            manager.next(id);
        });
    }

    /// `id` stopped: start the next waiting download, if any.
    fn next(&self, id: i64) {
        let next = {
            let mut state = self.inner.state.lock().unwrap();
            state.running.remove(&id);
            if state.running.len() < self.inner.config.max_active.max(1) {
                state.waiting.pop_front()
            } else {
                None
            }
        };
        if let Some(next) = next {
            self.start_now(next);
        }
    }

    fn finished(&self, id: i64, status: FinishStatus, path: Option<String>, error: Option<String>) {
        let _ = self.inner.updates.send(Update::Finished(Finished {
            download_id: id,
            status,
            path,
            error,
        }));
    }
}

/// A capture checked and turned into an engine request.
struct PreparedCapture {
    request: Request,
}

fn prepare_capture(capture: &Capture) -> Result<PreparedCapture, String> {
    let url = Url::parse(&capture.url).map_err(|e| format!("bad URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https links can be downloaded".into());
    }
    let method = match capture.method.as_deref() {
        None => Method::Get,
        Some(name) => Method::from_name(name).ok_or(format!("unsupported method {name}"))?,
    };
    let body = match method {
        Method::Post => capture
            .body_bytes()
            .map_err(|_| "the form data is not valid base64".to_string())?,
        Method::Get => Vec::new(),
    };
    let mut headers: Vec<(String, String)> = capture
        .headers
        .iter()
        .map(|Header { name, value }| (name.clone(), value.clone()))
        .collect();
    if let Some(referrer) = &capture.referrer
        && !has_header(&headers, "referer")
    {
        headers.push(("Referer".into(), referrer.clone()));
    }
    Ok(PreparedCapture {
        request: Request {
            url: capture.url.clone(),
            method,
            body,
            headers,
            cookies: jar_from(&capture.cookies),
        },
    })
}

/// `name=value; name2=value2` (as given with `-b`) as cookies of `url`'s host.
fn cookies_from_pairs(url: &Url, pairs: &str) -> Vec<BrowserCookie> {
    let host = url.host_str().unwrap_or_default().to_string();
    pairs
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.trim(), value.trim()))
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, value)| BrowserCookie {
            name: name.into(),
            value: value.into(),
            domain: host.clone(),
            host_only: true,
            path: "/".into(),
            secure: false,
            http_only: false,
            expiration_date: None,
        })
        .collect()
}

fn jar_from(cookies: &[BrowserCookie]) -> CookieJar {
    let jar = CookieJar::new();
    for c in cookies {
        jar.add_browser_cookie(&BrowserCookieSpec {
            name: c.name.clone(),
            value: c.value.clone(),
            domain: c.domain.clone(),
            host_only: c.host_only,
            path: c.path.clone(),
            secure: c.secure,
            http_only: c.http_only,
            expires: c.expiration_date,
        });
    }
    jar
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
}

fn rejected(reason: RejectReason, detail: String) -> CaptureResult {
    CaptureResult::Rejected {
        reason,
        detail: Some(detail),
    }
}

fn io_error(e: std::io::Error) -> StoreError {
    StoreError::Corrupt(format!("can't create the data folder: {e}"))
}
