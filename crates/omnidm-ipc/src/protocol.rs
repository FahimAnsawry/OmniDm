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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DownloadList {
    pub items: Vec<DownloadSummary>,
}
