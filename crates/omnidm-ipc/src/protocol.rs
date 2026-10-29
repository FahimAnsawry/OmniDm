// SPDX-License-Identifier: GPL-3.0-or-later

//! The messages exchanged by the extension, the CLI and the app (plan §8.2).
//!
//! These Rust types are the single source of truth: `cargo test` exports matching
//! TypeScript types to `protocol/ts/`, used by the extension and the app's UI (see
//! `.cargo/config.toml`).

use base64::Engine;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The protocol version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// Every message: `{"v": 1, "id": "…", "re"?: "…", "type": "…", "payload"?: {…}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Envelope {
    pub v: u32,
    /// Unique per message; replies name it in `re`.
    pub id: String,
    /// The `id` of the message this one answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub re: Option<String>,
    #[serde(flatten)]
    #[ts(flatten)]
    pub message: Message,
}

impl Envelope {
    pub fn new(id: impl Into<String>, message: Message) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            re: None,
            message,
        }
    }

    /// Marks this message as the answer to the message with id `re`.
    pub fn reply_to(mut self, re: impl Into<String>) -> Self {
        self.re = Some(re.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
#[ts(export)]
pub enum Message {
    // --- extension ↔ app ---
    /// First message from the extension.
    Hello(Hello),
    HelloAck(HelloAck),
    /// A browser download the extension has paused, offered to the app.
    Capture(Capture),
    CaptureResult(CaptureResult),
    /// Several links at once ("Download all links").
    CaptureMany(CaptureMany),
    CaptureManyResult(CaptureManyResult),
    /// The app asks for fresh cookies, to resume a download after a restart.
    CookiesRequest(CookiesRequest),
    CookiesResponse(CookiesResponse),
    Ping,
    Pong,
    Error(ErrorInfo),

    // --- CLI ↔ app ---
    Add(AddRequest),
    Added(DownloadRef),
    /// Asks for `progress` messages about a download until it finishes.
    Watch(DownloadRef),
    Progress(Progress),
    Finished(Finished),
    Pause(DownloadRef),
    Resume(DownloadRef),
    Ack,
    List,
    Downloads(DownloadList),
    /// Asks the app to open its window (sent when OmniDm is started a second time).
    Show,

    // --- queues (CLI ↔ app); each is answered with `ack`, `queues` or `error` ---
    ListQueues,
    Queues(QueueList),
    CreateQueue(NewQueue),
    EditQueue(QueueEdit),
    DeleteQueue(QueueRef),
    StartQueue(QueueRef),
    /// Pauses the queue's running downloads; its after-queue action doesn't run.
    StopQueue(QueueRef),
    /// Adds downloads to the end of a queue, taking them out of any other.
    QueueAdd(QueueItems),
    /// Takes downloads out of whatever queue they are in.
    QueueRemove(DownloadIds),
    /// Moves a download within its queue.
    QueueMove(QueueMove),
    /// Cancels the after-queue action that is counting down.
    CancelAfterQueue,

    // --- categories (CLI ↔ app) ---
    ListCategories,
    Categories(CategoryList),
    /// Sets where a category's browser downloads are saved; without a folder, they go to
    /// the downloads folder.
    SetCategoryFolder(CategoryFolder),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Hello {
    pub extension_version: String,
    /// `chrome`, `edge`, `firefox`…
    pub browser: String,
    pub browser_version: String,
    /// Protocol versions the extension speaks.
    pub protocols: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HelloAck {
    pub app_version: String,
    /// The version both sides use from now on.
    pub protocol: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// A cookie as the browser's `cookies` API reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    /// The cookie's domain; a leading dot means "and subdomains" (`host_only` false).
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Expiry in Unix seconds; absent for a session cookie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expiration_date: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct Capture {
    pub url: String,
    /// The URL after the browser followed redirects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    /// `GET` (when absent) or `POST`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// A `POST` body, base64-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    #[serde(default)]
    #[ts(optional = false)]
    pub headers: Vec<Header>,
    #[serde(default)]
    #[ts(optional = false)]
    pub cookies: Vec<BrowserCookie>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number")]
    pub size: Option<u64>,
    /// Started in a private window.
    #[serde(default)]
    #[ts(optional = false)]
    pub private: bool,
    /// The browser's own id for the paused download.
    #[ts(type = "number")]
    pub browser_download_id: u64,
}

impl Capture {
    pub fn new(url: impl Into<String>, browser_download_id: u64) -> Self {
        Self {
            url: url.into(),
            final_url: None,
            method: None,
            body: None,
            referrer: None,
            page_url: None,
            headers: Vec::new(),
            cookies: Vec::new(),
            suggested_name: None,
            mime: None,
            size: None,
            private: false,
            browser_download_id,
        }
    }

    /// The decoded `POST` body (empty without one).
    pub fn body_bytes(&self) -> Result<Vec<u8>, base64::DecodeError> {
        match &self.body {
            Some(body) => base64::engine::general_purpose::STANDARD.decode(body),
            None => Ok(Vec::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "outcome",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum CaptureResult {
    /// OmniDm took the download: the browser should cancel its copy.
    Accepted {
        #[ts(type = "number")]
        download_id: i64,
    },
    /// The browser should resume its own download.
    Rejected {
        reason: RejectReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RejectReason {
    /// The capture itself was unusable (bad URL, bad body…).
    Invalid,
    /// OmniDm couldn't fetch the file itself, so only the browser can.
    ProbeFailed,
    /// OmniDm isn't taking downloads right now.
    NotAccepting,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CaptureMany {
    pub items: Vec<Capture>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CaptureManyResult {
    /// One result per item, in order.
    pub results: Vec<CaptureResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CookiesRequest {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CookiesResponse {
    pub cookies: Vec<BrowserCookie>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

/// A download added by the CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct AddRequest {
    pub url: String,
    /// The folder to save into (absolute).
    pub dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connections: Option<u32>,
    #[serde(default)]
    #[ts(optional = false)]
    pub headers: Vec<Header>,
    /// `name=value; name2=value2` strings, as given with `-b`.
    #[serde(default)]
    #[ts(optional = false)]
    pub cookies: Vec<String>,
    /// Form data to `POST`, base64-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Put it in this queue (by name) instead of starting it now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DownloadRef {
    #[ts(type = "number")]
    pub download_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct Progress {
    #[ts(type = "number")]
    pub download_id: i64,
    #[ts(type = "number")]
    pub downloaded: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number")]
    pub total: Option<u64>,
    /// Bytes per second.
    #[ts(type = "number")]
    pub speed: u64,
    pub connections: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FinishStatus {
    Completed,
    Paused,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct Finished {
    #[ts(type = "number")]
    pub download_id: i64,
    pub status: FinishStatus,
    /// The saved file, when completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct DownloadSummary {
    #[ts(type = "number")]
    pub download_id: i64,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    pub save_dir: String,
    /// `queued`, `running`, `paused`, `completed`, `failed`…
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number")]
    pub total: Option<u64>,
    #[ts(type = "number")]
    pub downloaded: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The queue it is in, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
    /// Its category, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// When it was added, in Unix seconds.
    #[serde(default)]
    #[ts(type = "number")]
    pub added_at: i64,
    /// When it completed, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number")]
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct CategorySummary {
    #[ts(type = "number")]
    pub category_id: i64,
    pub name: String,
    /// File extensions without the dot.
    #[ts(optional = false)]
    pub extensions: Vec<String>,
    /// MIME types; `video/*` matches every video type.
    #[ts(optional = false)]
    pub mime_types: Vec<String>,
    /// Sites, each with its subdomains.
    #[ts(optional = false)]
    pub hosts: Vec<String>,
    /// Where its browser downloads are saved; absent means the downloads folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CategoryList {
    pub items: Vec<CategorySummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, optional_fields)]
pub struct CategoryFolder {
    /// The category's name, ignoring case.
    pub category: String,
    /// An absolute folder; absent to use the downloads folder again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DownloadList {
    pub items: Vec<DownloadSummary>,
}

/// Names a queue. Names are unique, ignoring case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueueRef {
    pub queue: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct NewQueue {
    pub name: String,
    /// Downloads running at once (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    /// `none` (default), `sleep`, `hibernate`, `shutdown` or `quit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_complete: Option<String>,
}

/// Changes a queue; absent fields stay as they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct QueueEdit {
    pub queue: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_complete: Option<String>,
    /// Replaces the whole schedule; an empty one means "only by hand".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<QueueSchedule>,
}

/// When a queue starts and stops by itself, in the PC's local time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct QueueSchedule {
    /// Start on each of `days` at this time, `HH:MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<String>,
    /// With `start_at`: `daily` (the default), `weekdays`, `weekend` or `mon,wed,fri`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<String>,
    /// Start once at this time, `YYYY-MM-DD HH:MM`; not together with `start_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<String>,
    /// Stop (pausing its downloads) at this time of day, `HH:MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_at: Option<String>,
    /// Wake the PC from sleep for the start.
    #[serde(default)]
    #[ts(optional = false)]
    pub wake: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QueueItems {
    pub queue: String,
    #[ts(type = "number[]")]
    pub download_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DownloadIds {
    #[ts(type = "number[]")]
    pub download_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QueueMove {
    #[ts(type = "number")]
    pub download_id: i64,
    /// The new place, 0 = first; past the end means last.
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QueueSummary {
    #[ts(type = "number")]
    pub queue_id: i64,
    pub name: String,
    pub max_concurrent: u32,
    /// `none`, `sleep`, `hibernate`, `shutdown` or `quit`.
    pub on_complete: String,
    /// Started and not yet finished or stopped.
    pub running: bool,
    /// Its downloads, in order.
    #[ts(type = "number[]")]
    pub download_ids: Vec<i64>,
    /// Absent when it only starts by hand and never stops by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub schedule: Option<QueueSchedule>,
    /// Its next scheduled start, `YYYY-MM-DD HH:MM` local time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_start: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueueList {
    pub items: Vec<QueueSummary>,
}

/// A captured download waiting for the user to say what to do with it (the capture
/// prompt).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct AskAbout {
    #[ts(type = "number")]
    pub download_id: i64,
    pub url: String,
    pub file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number")]
    pub size: Option<u64>,
    /// Where it would be saved: its category's folder, or the downloads folder.
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The server can continue it after a pause.
    pub resumable: bool,
    /// The downloads folder, for a category without its own.
    pub downloads_folder: String,
}

/// The answer to a capture prompt: download it (maybe renamed or elsewhere), now or later
/// in a queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct CaptureAnswer {
    #[ts(type = "number")]
    pub download_id: i64,
    pub file_name: String,
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Put in this queue instead of starting now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
}

/// A queue finished and its after-queue action is coming: sent every second while it
/// counts down, then once more with the outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct AfterQueue {
    #[ts(type = "number")]
    pub queue_id: i64,
    pub queue: String,
    /// `sleep`, `hibernate`, `shutdown` or `quit`.
    pub action: String,
    /// Seconds until it happens.
    pub seconds_left: u32,
    /// Other downloads are still running; the countdown starts when they end.
    pub waiting_for_downloads: bool,
    /// Absent while counting down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<AfterQueueOutcome>,
    /// Why it was cancelled or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AfterQueueOutcome {
    Done,
    Cancelled,
    Failed,
}
