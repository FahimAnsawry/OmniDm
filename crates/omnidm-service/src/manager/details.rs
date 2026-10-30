// SPDX-License-Identifier: GPL-3.0-or-later

//! A download's details: its pieces as they are right now, its connections, what the
//! server said, and a log of what happened since OmniDM started.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use omnidm_core::engine::Event;
use omnidm_ipc::protocol::{DownloadDetails, Header, LogLine, RedirectHop, SegmentInfo};
use omnidm_store::DownloadId;

use super::Manager;

/// Log lines kept per download; older ones are dropped.
const LOG_LINES: usize = 500;

/// Headers whose values are secrets, shown as hidden.
const SECRET_HEADERS: [&str; 3] = ["authorization", "proxy-authorization", "cookie"];

/// What the running engine last said about one download, and its log.
#[derive(Default)]
pub(super) struct Live {
    segments: Vec<SegmentInfo>,
    speed: u64,
    connections: u32,
    log: VecDeque<LogLine>,
}

impl Manager {
    /// Adds a line to a download's log.
    pub(super) fn note(&self, id: i64, text: impl Into<String>) {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let mut details = self.inner.details.lock().unwrap();
        let log = &mut details.entry(id).or_default().log;
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(LogLine {
            at,
            text: text.into(),
        });
    }

    /// Keeps what the engine reports while a download runs.
    pub(super) fn engine_event(&self, id: i64, event: &Event) {
        match event {
            Event::Progress {
                speed, connections, ..
            } => {
                let mut details = self.inner.details.lock().unwrap();
                let live = details.entry(id).or_default();
                live.speed = *speed;
                live.connections = *connections;
            }
            Event::Segments(segments) => {
                let segments = segments
                    .iter()
                    .map(|s| SegmentInfo {
                        start: s.start,
                        end: s.end,
                        written: s.written,
                        active: s.active,
                        speed: s.speed,
                    })
                    .collect();
                self.inner
                    .details
                    .lock()
                    .unwrap()
                    .entry(id)
                    .or_default()
                    .segments = segments;
            }
            Event::Log(line) => self.note(id, line.clone()),
            Event::Probed(_) => {}
        }
    }

    /// Forgets a deleted download's details.
    pub(super) fn forget_details(&self, id: i64) {
        self.inner.details.lock().unwrap().remove(&id);
    }

    /// Everything about one download; `None` if there is no such download.
    pub fn details(&self, id: i64) -> Option<DownloadDetails> {
        let summary = self.summaries(Some(DownloadId(id))).into_iter().next()?;
        let (record, stored) = {
            let store = self.inner.store.lock().unwrap();
            let record = store.get_download(DownloadId(id)).ok()??;
            let stored = store.load_segments(record.id).unwrap_or_default();
            (record, stored)
        };
        let running = self.is_active(id);
        let details = self.inner.details.lock().unwrap();
        let live = details.get(&id);
        // While running, the engine's latest view; otherwise what was saved.
        let mut segments = match live {
            Some(live) if running && !live.segments.is_empty() => live.segments.clone(),
            _ => stored
                .iter()
                .map(|s| SegmentInfo {
                    start: s.start,
                    end: s.end,
                    written: s.written,
                    active: false,
                    speed: 0,
                })
                .collect(),
        };
        segments.sort_by_key(|s| s.start);
        let request_headers = record
            .headers
            .iter()
            .map(|(name, value)| Header {
                name: name.clone(),
                value: if SECRET_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                    "(hidden)".into()
                } else {
                    value.clone()
                },
            })
            .chain(record.user_agent.iter().map(|agent| Header {
                name: "User-Agent".into(),
                value: agent.clone(),
            }))
            .collect();
        Some(DownloadDetails {
            summary,
            final_url: record.final_url,
            content_type: record.content_type,
            etag: record.etag,
            last_modified: record.last_modified,
            resumable: record.resumable,
            method: record.final_method.unwrap_or(record.method).as_str().into(),
            referrer: record.referrer,
            request_headers,
            redirects: record
                .redirects
                .into_iter()
                .map(|(status, url)| RedirectHop { status, url })
                .collect(),
            segments,
            speed: live.filter(|_| running).map(|l| l.speed),
            connections: live.filter(|_| running).map_or(0, |l| l.connections),
            log: live
                .map(|l| l.log.iter().cloned().collect())
                .unwrap_or_default(),
        })
    }
}
