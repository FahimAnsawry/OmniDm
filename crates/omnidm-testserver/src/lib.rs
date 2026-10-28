// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP test server with fault injection, used by OmniDm integration tests.
//!
//! Files are generated from a seed, so a test can serve gigabytes without touching the
//! disk and still check every byte. Each file can be told to misbehave in the ways real
//! servers do: ignore ranges, change while downloading, slow down, drop the connection,
//! answer 429/503, send no length, or return an HTML page instead of the file.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use tokio::sync::oneshot;

/// Body chunk size; also the granularity of throttling and connection drops.
const CHUNK: u64 = 16 * 1024;

const HTML_PAGE: &str = "<!doctype html><html><head><title>Sign in</title></head><body><h1>Please sign in</h1></body></html>";

/// A generated file and the faults it injects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpec {
    pub size: u64,
    pub seed: u64,
    /// Honour `Range` and advertise `Accept-Ranges: bytes`.
    pub ranges: bool,
    /// Strong ETag, sent quoted.
    pub etag: Option<String>,
    pub last_modified: Option<SystemTime>,
    /// Send `Content-Length`. Without it the size is unknown and ranges are off.
    pub send_length: bool,
    /// Limit each response to this many bytes per second.
    pub throttle: Option<u64>,
    /// Abort the connection after this many body bytes, on the first `times` responses.
    pub drop_after: Option<(u64, u32)>,
    /// Stop sending (without closing) after this many body bytes, on the first `times`
    /// responses.
    pub stall_after: Option<(u64, u32)>,
    /// Answer the first `times` requests with `status` and an optional `Retry-After`.
    pub fail: Option<Failure>,
    /// Answer `200 text/html` with an error page instead of the file.
    pub html_instead: bool,
    pub content_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    pub status: u16,
    pub times: u32,
    pub retry_after_secs: Option<u64>,
}

impl FileSpec {
    pub fn new(size: u64) -> Self {
        Self {
            size,
            seed: 1,
            ranges: true,
            etag: None,
            last_modified: None,
            send_length: true,
            throttle: None,
            drop_after: None,
            stall_after: None,
            fail: None,
            html_instead: false,
            content_type: "application/octet-stream".into(),
        }
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn no_ranges(mut self) -> Self {
        self.ranges = false;
        self
    }

    pub fn etag(mut self, etag: &str) -> Self {
        self.etag = Some(etag.into());
        self
    }

    pub fn last_modified(mut self, time: SystemTime) -> Self {
        self.last_modified = Some(time);
        self
    }

    pub fn unknown_length(mut self) -> Self {
        self.send_length = false;
        self.ranges = false;
        self
    }

    pub fn throttle(mut self, bytes_per_sec: u64) -> Self {
        self.throttle = Some(bytes_per_sec);
        self
    }

    pub fn drop_after(mut self, bytes: u64, times: u32) -> Self {
        self.drop_after = Some((bytes, times));
        self
    }

    pub fn stall_after(mut self, bytes: u64, times: u32) -> Self {
        self.stall_after = Some((bytes, times));
        self
    }

    pub fn fail(mut self, status: u16, times: u32, retry_after_secs: Option<u64>) -> Self {
        self.fail = Some(Failure {
            status,
            times,
            retry_after_secs,
        });
        self
    }

    pub fn html_instead(mut self) -> Self {
        self.html_instead = true;
        self
    }

    /// The file's bytes in `[start, start + len)`.
    pub fn bytes(&self, start: u64, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        fill(self.seed, start, &mut out);
        out
    }
}

/// Fills `buf` with the generated content of a file with `seed`, starting at `offset`.
pub fn fill(seed: u64, offset: u64, buf: &mut [u8]) {
    // Whole 8-byte words at a time: fast enough to serve gigabytes in tests.
    let mut at = offset;
    let mut filled = 0;
    while filled < buf.len() {
        let bytes = word(seed, at / 8).to_le_bytes();
        let skip = (at % 8) as usize;
        let n = (8 - skip).min(buf.len() - filled);
        buf[filled..filled + n].copy_from_slice(&bytes[skip..skip + n]);
        filled += n;
        at += n as u64;
    }
}

/// SplitMix64 of the word index, keyed by the seed.
fn word(seed: u64, index: u64) -> u64 {
    let mut z = index
        .wrapping_add(seed.rotate_left(32))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// One request the server received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    pub method: String,
    pub path: String,
    pub range: Option<String>,
    pub if_range: Option<String>,
}

#[derive(Default)]
struct Shared {
    files: HashMap<String, FileState>,
    redirects: HashMap<String, (u16, String)>,
    log: Vec<RequestRecord>,
}

struct FileState {
    spec: FileSpec,
    failures_sent: u32,
    drops_sent: u32,
    stalls_sent: u32,
}

/// A running test server on `127.0.0.1` with a random port. Stops when dropped.
pub struct TestServer {
    addr: SocketAddr,
    shared: Arc<Mutex<Shared>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    pub async fn start() -> std::io::Result<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new().fallback(handle).with_state(shared.clone());
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Ok(Self {
            addr,
            shared,
            shutdown: Some(tx),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Absolute URL of `path` (which starts with `/`).
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    /// Serves `spec` at `path`, replacing any previous file and resetting its fault counters.
    pub fn set_file(&self, path: &str, spec: FileSpec) {
        self.shared.lock().unwrap().files.insert(
            path.into(),
            FileState {
                spec,
                failures_sent: 0,
                drops_sent: 0,
                stalls_sent: 0,
            },
        );
    }

    /// Redirects `from` to `to` (a path or absolute URL) with a 3xx `status`.
    pub fn redirect(&self, from: &str, to: &str, status: u16) {
        self.shared
            .lock()
            .unwrap()
            .redirects
            .insert(from.into(), (status, to.into()));
    }

    /// Every request received so far, oldest first.
    pub fn requests(&self) -> Vec<RequestRecord> {
        self.shared.lock().unwrap().log.clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// What to send for one file request, decided while holding the lock.
struct Plan {
    spec: FileSpec,
    range: Option<(u64, u64)>,
    cut: Option<Cut>,
}

/// How a response is cut short after a number of body bytes.
#[derive(Debug, Clone, Copy)]
enum Cut {
    /// Abort the connection.
    Drop(u64),
    /// Stop sending but keep the connection open.
    Stall(u64),
}

impl Cut {
    fn at(self) -> u64 {
        match self {
            Self::Drop(at) | Self::Stall(at) => at,
        }
    }
}

async fn handle(
    State(shared): State<Arc<Mutex<Shared>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let text = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let range_header = text(header::RANGE);
    let if_range = text(header::IF_RANGE);
    let path = uri.path().to_string();

    let plan = {
        let mut shared = shared.lock().unwrap();
        shared.log.push(RequestRecord {
            method: method.to_string(),
            path: path.clone(),
            range: range_header.clone(),
            if_range: if_range.clone(),
        });
        if let Some((status, to)) = shared.redirects.get(&path) {
            let status = StatusCode::from_u16(*status).unwrap_or(StatusCode::FOUND);
            return (status, [(header::LOCATION, to.clone())]).into_response();
        }
        let Some(file) = shared.files.get_mut(&path) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if let Some(fail) = file.spec.fail
            && file.failures_sent < fail.times
        {
            file.failures_sent += 1;
            let status =
                StatusCode::from_u16(fail.status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
            let mut resp = status.into_response();
            if let Some(secs) = fail.retry_after_secs {
                resp.headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from(secs));
            }
            return resp;
        }
        if file.spec.html_instead {
            return (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                HTML_PAGE,
            )
                .into_response();
        }
        let spec = file.spec.clone();
        let range = match range_header.as_deref() {
            Some(r) if spec.ranges && if_range_matches(&spec, if_range.as_deref()) => {
                match parse_range(r, spec.size) {
                    RangeRequest::Range(start, end) => Some((start, end)),
                    RangeRequest::Unsatisfiable => {
                        return (
                            StatusCode::RANGE_NOT_SATISFIABLE,
                            [(header::CONTENT_RANGE, format!("bytes */{}", spec.size))],
                        )
                            .into_response();
                    }
                    RangeRequest::Ignore => None,
                }
            }
            _ => None,
        };
        let mut cut = None;
        if method != Method::HEAD {
            if let Some((bytes, times)) = file.spec.drop_after
                && file.drops_sent < times
            {
                file.drops_sent += 1;
                cut = Some(Cut::Drop(bytes));
            } else if let Some((bytes, times)) = file.spec.stall_after
                && file.stalls_sent < times
            {
                file.stalls_sent += 1;
                cut = Some(Cut::Stall(bytes));
            }
        }
        Plan { spec, range, cut }
    };

    file_response(plan, method == Method::HEAD)
}

fn file_response(plan: Plan, head: bool) -> Response {
    let spec = &plan.spec;
    let (status, start, end) = match plan.range {
        Some((start, end)) => (StatusCode::PARTIAL_CONTENT, start, end),
        None => (StatusCode::OK, 0, spec.size.saturating_sub(1)),
    };
    let len = if spec.size == 0 { 0 } else { end - start + 1 };

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, &spec.content_type);
    if spec.ranges {
        builder = builder.header(header::ACCEPT_RANGES, "bytes");
    }
    if let Some(etag) = &spec.etag {
        builder = builder.header(header::ETAG, format!("\"{etag}\""));
    }
    if let Some(modified) = spec.last_modified {
        builder = builder.header(header::LAST_MODIFIED, httpdate::fmt_http_date(modified));
    }
    if spec.send_length {
        builder = builder.header(header::CONTENT_LENGTH, len);
    }
    if plan.range.is_some() {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", spec.size),
        );
    }

    let body = if head || len == 0 {
        Body::empty()
    } else {
        body_stream(spec.seed, start, len, spec.throttle, plan.cut)
    };
    builder.body(body).unwrap()
}

/// Streams `len` generated bytes from `start`, optionally throttled or cut short.
fn body_stream(seed: u64, start: u64, len: u64, throttle: Option<u64>, cut: Option<Cut>) -> Body {
    let began = Instant::now();
    let stream = futures_util::stream::unfold(0u64, move |sent| async move {
        if sent >= len {
            return None;
        }
        match cut {
            Some(Cut::Drop(at)) if sent >= at => {
                // Let the bytes already sent reach the client before the connection breaks,
                // as they would on a real network failure.
                tokio::time::sleep(Duration::from_millis(50)).await;
                let err =
                    std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "injected drop");
                return Some((Err(err), len));
            }
            // Never resumes; the task ends when the client gives up on the connection.
            Some(Cut::Stall(at)) if sent >= at => std::future::pending::<()>().await,
            _ => {}
        }
        let mut n = CHUNK.min(len - sent);
        if let Some(cut) = cut {
            n = n.min(cut.at() - sent);
        }
        if let Some(rate) = throttle.filter(|&r| r > 0) {
            let due = Duration::from_secs_f64((sent + n) as f64 / rate as f64);
            tokio::time::sleep(due.saturating_sub(began.elapsed())).await;
        }
        let mut buf = vec![0; n as usize];
        fill(seed, start + sent, &mut buf);
        Some((Ok::<_, std::io::Error>(Bytes::from(buf)), sent + n))
    });
    Body::from_stream(stream)
}

fn if_range_matches(spec: &FileSpec, if_range: Option<&str>) -> bool {
    let Some(value) = if_range else {
        return true;
    };
    let etag_matches = spec
        .etag
        .as_ref()
        .is_some_and(|etag| value == format!("\"{etag}\""));
    let date_matches = spec
        .last_modified
        .is_some_and(|modified| value == httpdate::fmt_http_date(modified));
    etag_matches || date_matches
}

#[derive(Debug, PartialEq, Eq)]
enum RangeRequest {
    Range(u64, u64),
    Unsatisfiable,
    /// Malformed or multi-range: serve the whole file.
    Ignore,
}

fn parse_range(value: &str, size: u64) -> RangeRequest {
    let Some(spec) = value.strip_prefix("bytes=") else {
        return RangeRequest::Ignore;
    };
    if spec.contains(',') {
        return RangeRequest::Ignore;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return RangeRequest::Ignore;
    };
    let parse = |s: &str| s.trim().parse::<u64>().ok();
    match (first.trim().is_empty(), last.trim().is_empty()) {
        // "-n": the last n bytes
        (true, false) => match parse(last) {
            Some(0) => RangeRequest::Unsatisfiable,
            Some(_) if size == 0 => RangeRequest::Unsatisfiable,
            Some(n) => RangeRequest::Range(size.saturating_sub(n), size - 1),
            None => RangeRequest::Ignore,
        },
        // "a-" or "a-b"
        (false, open) => {
            let Some(start) = parse(first) else {
                return RangeRequest::Ignore;
            };
            let end = if open { Some(u64::MAX) } else { parse(last) };
            match end {
                Some(end) if end < start => RangeRequest::Ignore,
                Some(_) if start >= size => RangeRequest::Unsatisfiable,
                Some(end) => RangeRequest::Range(start, end.min(size - 1)),
                None => RangeRequest::Ignore,
            }
        }
        (true, true) => RangeRequest::Ignore,
    }
}
