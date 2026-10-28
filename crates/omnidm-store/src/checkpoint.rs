// SPDX-License-Identifier: GPL-3.0-or-later

//! Connects the download engine to the store: saves what the engine reports, and turns a
//! stored download back into the engine's resume state.

use omnidm_core::Segment;
use omnidm_core::engine::{Checkpointer, RemoteInfo, Resume};

use crate::{DownloadId, DownloadRecord, ProbeResult, Store};

/// Saves one download's engine progress into the store.
pub struct StoreCheckpointer<'a> {
    pub store: &'a mut Store,
    pub id: DownloadId,
    /// The name the file gets when it is complete.
    pub file_name: String,
}

impl Checkpointer for StoreCheckpointer<'_> {
    fn probed(&mut self, info: &RemoteInfo) -> Result<(), String> {
        let probe = ProbeResult {
            final_url: info.final_url.clone(),
            file_name: self.file_name.clone(),
            total_size: info.total,
            etag: info.etag.clone(),
            last_modified: info.last_modified.clone(),
            resumable: info.resumable,
            final_method: info.final_method,
        };
        self.store
            .record_probe(self.id, &probe)
            .map_err(|e| e.to_string())
    }

    fn save(&mut self, segments: &[Segment]) -> Result<(), String> {
        self.store
            .save_segments(self.id, segments)
            .map_err(|e| e.to_string())
    }
}

/// The engine state to continue `record` from, if it was ever probed.
pub fn resume_from(record: &DownloadRecord, segments: Vec<Segment>) -> Option<Resume> {
    let final_url = record.final_url.clone()?;
    Some(Resume {
        info: RemoteInfo {
            final_url,
            total: record.total_size,
            etag: record.etag.clone(),
            last_modified: record.last_modified.clone(),
            resumable: record.resumable,
            content_type: None,
            content_disposition: None,
            final_method: record.final_method.unwrap_or(record.method),
        },
        segments,
    })
}
