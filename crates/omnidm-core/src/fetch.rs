// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetching a small file, or part of one, over a single connection: a video's playlist,
//! its keys and its pieces. Requests follow the engine's rules (redirects by hand, cookies,
//! captured headers) and temporary failures are retried the same way.

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

use omnidm_net::http::{RequestedRange, StatusClass, check_range_response, classify_status};
use reqwest::header;
use tokio_util::sync::CancellationToken;

use crate::engine::{
    EngineConfig, EngineError, Method, Request, SendError, header_text, retry_after, retry_delay,
    send, wait,
};

/// Most bytes fetched into memory: a playlist or key bigger than this is not one.
pub const MEMORY_LIMIT: u64 = 64 * 1024 * 1024;

/// What was fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The address after redirects; relative addresses in a playlist are relative to it.
    pub final_url: String,
    pub content_type: Option<String>,
    /// Bytes now held: the whole range, when one was asked for.
    pub size: u64,
}

/// Where fetched bytes go.
pub trait Sink {
    /// Bytes held so far.
    fn held(&mut self) -> io::Result<u64>;
    /// Drops everything held.
    fn clear(&mut self) -> io::Result<()>;
    fn append(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Most bytes it takes.
    fn limit(&self) -> Option<u64> {
        None
    }
}

impl Sink for Vec<u8> {
    fn held(&mut self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }

    fn clear(&mut self) -> io::Result<()> {
        Vec::clear(self);
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }

    fn limit(&self) -> Option<u64> {
        Some(MEMORY_LIMIT)
    }
}

impl Sink for File {
    fn held(&mut self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.set_len(0)?;
        self.seek(SeekFrom::Start(0)).map(|_| ())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.seek(SeekFrom::End(0))?;
        self.write_all(bytes)
    }
}

/// Fetches `url` with a `GET`, or only bytes `range` of it (both ends included), into
/// `sink`. Bytes `sink` already holds count as fetched, and only the rest is asked for
/// when the server allows ranges; otherwise it starts over. `on_bytes` hears about every
/// new byte. `None` means it was cancelled; what arrived stays in `sink`.
#[allow(clippy::too_many_arguments)]
pub async fn fetch(
    client: &reqwest::Client,
    request: &Request,
    url: &str,
    range: Option<(u64, u64)>,
    config: &EngineConfig,
    cancel: &CancellationToken,
    sink: &mut impl Sink,
    mut on_bytes: impl FnMut(u64),
) -> Result<Option<Fetched>, EngineError> {
    let wanted_size = range.map(|(start, end)| end - start + 1);
    let mut attempt = 0;
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        attempt += 1;
        let mut held = sink.held()?;
        if wanted_size.is_some_and(|size| held > size) {
            sink.clear()?;
            held = 0;
        }
        if wanted_size == Some(held) {
            return Ok(Some(Fetched {
                final_url: url.to_owned(),
                content_type: None,
                size: held,
            }));
        }
        let first = range.map_or(0, |(start, _)| start);
        let asked = (range.is_some() || held > 0).then(|| RequestedRange {
            start: first + held,
            end: range.map(|(_, end)| end),
        });

        let (error, wait_for) = match send(client, request, url, Method::Get, asked, None).await {
            Err(SendError::TooManyRedirects) => {
                return Err(EngineError::Network(
                    SendError::TooManyRedirects.to_string(),
                ));
            }
            Err(e) => (EngineError::Network(e.to_string()), None),
            Ok((mut response, _, _)) => {
                let status = response.status().as_u16();
                match classify_status(status) {
                    StatusClass::Success => {
                        let content_type = header_text(response.headers(), header::CONTENT_TYPE);
                        if !config.allow_html
                            && content_type
                                .as_deref()
                                .is_some_and(|t| t.to_ascii_lowercase().starts_with("text/html"))
                        {
                            return Err(EngineError::HtmlInsteadOfFile);
                        }
                        // Bytes of the body to skip, and how many to keep at most.
                        let (mut skip, total) = if status == 206 {
                            let content_range =
                                header_text(response.headers(), header::CONTENT_RANGE);
                            let requested = asked.unwrap_or(RequestedRange {
                                start: 0,
                                end: None,
                            });
                            match check_range_response(
                                206,
                                content_range.as_deref(),
                                requested,
                                None,
                            ) {
                                Ok(got) => (0, wanted_size.or(got.total.map(|t| t - first))),
                                Err(_) => {
                                    // Not the bytes asked for: what is held can't be
                                    // trusted to continue from.
                                    sink.clear()?;
                                    if attempt >= config.retry.max_attempts.max(1) {
                                        return Err(EngineError::Network(
                                            "the server sent other bytes than asked for".into(),
                                        ));
                                    }
                                    continue;
                                }
                            }
                        } else {
                            // The whole file: what was held is redone from its start.
                            sink.clear()?;
                            held = 0;
                            let length = response.content_length();
                            (first, wanted_size.or(length))
                        };
                        if let (Some(total), Some(limit)) = (total, sink.limit())
                            && total > limit
                        {
                            return Err(EngineError::Network(format!(
                                "the file is too big ({total} bytes) to be a playlist or key"
                            )));
                        }

                        let mut got = 0u64;
                        let failed = loop {
                            if total.is_some_and(|t| held + got >= t) {
                                break None;
                            }
                            let chunk = tokio::select! {
                                _ = cancel.cancelled() => return Ok(None),
                                chunk = tokio::time::timeout(config.stall_timeout, response.chunk()) => chunk,
                            };
                            let chunk = match chunk {
                                Err(_) => {
                                    break Some(EngineError::Network(
                                        "the server stopped sending data".into(),
                                    ));
                                }
                                Ok(Err(e)) => break Some(EngineError::Network(e.to_string())),
                                Ok(Ok(None)) => break None,
                                Ok(Ok(Some(chunk))) => chunk,
                            };
                            let mut bytes = &chunk[..];
                            if skip > 0 {
                                let cut = (skip as usize).min(bytes.len());
                                bytes = &bytes[cut..];
                                skip -= cut as u64;
                            }
                            if let Some(total) = total {
                                let room = (total - held - got).min(bytes.len() as u64);
                                bytes = &bytes[..room as usize];
                            }
                            if sink
                                .limit()
                                .is_some_and(|limit| held + got + bytes.len() as u64 > limit)
                            {
                                return Err(EngineError::Network(
                                    "the file is too big to be a playlist or key".into(),
                                ));
                            }
                            sink.append(bytes)?;
                            got += bytes.len() as u64;
                            on_bytes(bytes.len() as u64);
                        };
                        if got > 0 {
                            attempt = 0;
                        }
                        match failed {
                            Some(error) => (error, None),
                            None if total.is_some_and(|t| held + got < t) => (
                                EngineError::Network("the server sent less than it said".into()),
                                None,
                            ),
                            None => {
                                return Ok(Some(Fetched {
                                    final_url: response.url().to_string(),
                                    content_type,
                                    size: held + got,
                                }));
                            }
                        }
                    }
                    StatusClass::Retry => (EngineError::Status(status), None),
                    StatusClass::SlowDown => {
                        (EngineError::Status(status), retry_after(response.headers()))
                    }
                    // What was held is no longer wanted by the server: start over.
                    _ if status == 416 && held > 0 => {
                        sink.clear()?;
                        (EngineError::Status(status), None)
                    }
                    StatusClass::AuthRequired | StatusClass::Fatal => {
                        return Err(EngineError::Status(status));
                    }
                }
            }
        };
        if attempt >= config.retry.max_attempts.max(1) {
            return Err(error);
        }
        if wait(cancel, retry_delay(config, attempt, wait_for)).await {
            return Ok(None);
        }
    }
}
