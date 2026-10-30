// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP test server with fault injection, used by OmniDM integration tests.
//!
//! Files are generated from a seed, so a test can serve gigabytes without touching the
//! disk and still check every byte. Each file can be told to misbehave in the ways real
//! servers do: ignore ranges, change while downloading, slow down, drop the connection,
//! answer 429/503, send no length, or return an HTML page instead of the file.
//! [`TestProxy`] adds an HTTP proxy.

mod proxy;
pub use proxy::{ProxyConnection, TestProxy};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::StreamExt;
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
    /// Sent as `Content-Disposition`, byte for byte (non-ASCII included).
    pub content_disposition: Option<String>,
    /// Answer `403` unless the request's `Cookie` header has this `name=value` pair.
    pub required_cookie: Option<String>,
    /// Only answer a `POST` with exactly this body: `405` for other methods, `400` for
    /// another body.
    pub post_body: Option<Vec<u8>>,
    /// Serve this many requests, then answer `403` to every later one, as an expired
    /// signed link does.
    pub expire_after: Option<u32>,
    /// Send at most this many responses at once; answer `503` to the rest, as busy
    /// mirrors do.
    pub max_concurrent: Option<u32>,
    /// Throttle the first `n` responses to this many bytes per second, whatever
    /// `throttle` says: one crawling connection among fast ones.
    pub slow_first: Option<(u32, u64)>,
    /// Answer `403` unless the request has this header (name, exact value).
    pub required_header: Option<(String, String)>,
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
            content_disposition: None,
            required_cookie: None,
            post_body: None,
            expire_after: None,
            max_concurrent: None,
            slow_first: None,
            required_header: None,
        }
    }

    pub fn require_header(mut self, name: &str, value: &str) -> Self {
        self.required_header = Some((name.into(), value.into()));
        self
    }

    pub fn slow_first(mut self, responses: u32, bytes_per_sec: u64) -> Self {
        self.slow_first = Some((responses, bytes_per_sec));
        self
    }

    pub fn max_concurrent(mut self, responses: u32) -> Self {
        self.max_concurrent = Some(responses);
        self
    }

    pub fn expire_after(mut self, requests: u32) -> Self {
        self.expire_after = Some(requests);
        self
    }

    pub fn post_only(mut self, body: &str) -> Self {
        self.post_body = Some(body.as_bytes().to_vec());
        self
    }

    pub fn require_cookie(mut self, pair: &str) -> Self {
        self.required_cookie = Some(pair.into());
        self
    }

    pub fn content_type(mut self, content_type: &str) -> Self {
        self.content_type = content_type.into();
        self
    }

    pub fn content_disposition(mut self, value: &str) -> Self {
        self.content_disposition = Some(value.into());
        self
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
    /// Every header, names in lower case, in the order received.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RequestRecord {
    /// The first value of header `name` (any case).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
struct Shared {
    files: HashMap<String, FileState>,
    /// Path → (status, target, `Set-Cookie` value).
    redirects: HashMap<String, (u16, String, Option<String>)>,
    /// Path → (content type, body), served as is.
    texts: HashMap<String, (String, Vec<u8>)>,
    log: Vec<RequestRecord>,
}

struct FileState {
    spec: FileSpec,
    failures_sent: u32,
    /// Requests answered so far (for `expire_after`).
    served: u32,
    /// Responses still sending their body (for `max_concurrent`).
    in_flight: Arc<AtomicU32>,
    /// Requests refused with `503` for being over `max_concurrent`.
    refused: u32,
    /// Responses sent slowly so far (for `slow_first`).
    slow_sent: u32,
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
                served: 0,
                in_flight: Arc::new(AtomicU32::new(0)),
                refused: 0,
                slow_sent: 0,
                drops_sent: 0,
                stalls_sent: 0,
            },
        );
    }

    /// Serves `body` at `path` with `content_type` (for small files such as PAC scripts).
    pub fn set_text(&self, path: &str, content_type: &str, body: &str) {
        self.set_bytes(path, content_type, body.as_bytes().to_vec());
    }

    /// Serves `body` at `path` with `content_type`, as is: no ranges or faults.
    pub fn set_bytes(&self, path: &str, content_type: &str, body: Vec<u8>) {
        self.shared
            .lock()
            .unwrap()
            .texts
            .insert(path.into(), (content_type.into(), body));
    }

    /// Redirects `from` to `to` (a path or absolute URL) with a 3xx `status`.
    pub fn redirect(&self, from: &str, to: &str, status: u16) {
        self.shared
            .lock()
            .unwrap()
            .redirects
            .insert(from.into(), (status, to.into(), None));
    }

    /// Like [`redirect`](Self::redirect), also sending `Set-Cookie: set_cookie`.
    pub fn redirect_setting_cookie(&self, from: &str, to: &str, status: u16, set_cookie: &str) {
        self.shared
            .lock()
            .unwrap()
            .redirects
            .insert(from.into(), (status, to.into(), Some(set_cookie.into())));
    }

    /// How many requests for `path` were refused for being over `max_concurrent`.
    pub fn refused(&self, path: &str) -> u32 {
        self.shared
            .lock()
            .unwrap()
            .files
            .get(path)
            .map_or(0, |f| f.refused)
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
    /// Counts this response as in flight until its body is dropped.
    slot: Option<InFlight>,
}

/// One response counted against `max_concurrent`; released when dropped.
struct InFlight(Arc<AtomicU32>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
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
    body: Bytes,
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
            headers: headers
                .iter()
                .map(|(n, v)| {
                    (
                        n.as_str().to_string(),
                        String::from_utf8_lossy(v.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            body: body.to_vec(),
        });
        if let Some((status, to, set_cookie)) = shared.redirects.get(&path) {
            let status = StatusCode::from_u16(*status).unwrap_or(StatusCode::FOUND);
            let mut resp = (status, [(header::LOCATION, to.clone())]).into_response();
            if let Some(value) = set_cookie
                .as_deref()
                .and_then(|v| HeaderValue::from_str(v).ok())
            {
                resp.headers_mut().insert(header::SET_COOKIE, value);
            }
            return resp;
        }
        if let Some((content_type, text)) = shared.texts.get(&path) {
            return ([(header::CONTENT_TYPE, content_type.clone())], text.clone()).into_response();
        }
        let Some(file) = shared.files.get_mut(&path) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if let Some(expected) = &file.spec.post_body {
            if method != Method::POST {
                return StatusCode::METHOD_NOT_ALLOWED.into_response();
            }
            if body.as_ref() != expected.as_slice() {
                return StatusCode::BAD_REQUEST.into_response();
            }
        }
        if let Some((name, value)) = &file.spec.required_header {
            let sent = headers.get(name.as_str()).and_then(|v| v.to_str().ok());
            if sent != Some(value.as_str()) {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        if let Some(pair) = &file.spec.required_cookie {
            let sent = text(header::COOKIE).unwrap_or_default();
            if !sent.split(';').any(|p| p.trim() == pair) {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        if let Some(limit) = file.spec.expire_after {
            if file.served >= limit {
                return StatusCode::FORBIDDEN.into_response();
            }
            file.served += 1;
        }
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
        let mut slot = None;
        if let Some(max) = file.spec.max_concurrent
            && method != Method::HEAD
        {
            if file.in_flight.load(Ordering::SeqCst) >= max {
                file.refused += 1;
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            file.in_flight.fetch_add(1, Ordering::SeqCst);
            slot = Some(InFlight(file.in_flight.clone()));
        }
        let mut throttle = file.spec.throttle;
        if let Some((count, rate)) = file.spec.slow_first
            && file.slow_sent < count
            && method != Method::HEAD
        {
            file.slow_sent += 1;
            throttle = Some(rate);
        }
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
        Plan {
            spec: FileSpec { throttle, ..spec },
            range,
            cut,
            slot,
        }
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
    if let Some(disposition) = &spec.content_disposition {
        let value = HeaderValue::from_bytes(disposition.as_bytes())
            .expect("Content-Disposition has no control characters");
        builder = builder.header(header::CONTENT_DISPOSITION, value);
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
        body_stream(spec.seed, start, len, spec.throttle, plan.cut, plan.slot)
    };
    builder.body(body).unwrap()
}

/// Streams `len` generated bytes from `start`, optionally throttled or cut short.
fn body_stream(
    seed: u64,
    start: u64,
    len: u64,
    throttle: Option<u64>,
    cut: Option<Cut>,
    slot: Option<InFlight>,
) -> Body {
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
    // The slot lives as long as the stream: until the body is sent or the client leaves.
    Body::from_stream(stream.map(move |chunk| {
        let _ = &slot;
        chunk
    }))
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
