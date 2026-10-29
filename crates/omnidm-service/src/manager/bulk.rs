// SPDX-License-Identifier: GPL-3.0-or-later

//! Actions on many downloads at once (the app's toolbar and tray), and deleting.

use std::path::PathBuf;
use std::time::Duration;

use omnidm_store::{DownloadId, Status};
use tokio::sync::broadcast::error::RecvError;

use super::{Manager, Update};
use crate::job::PART_SUFFIX;
use crate::media::WORK_SUFFIX;

/// How long deleting waits for a running download to stop.
const STOP_WAIT: Duration = Duration::from_secs(10);

impl Manager {
    /// Stops every started queue and pauses every running or waiting download.
    pub fn pause_all(&self) {
        let (queues, active): (Vec<i64>, Vec<i64>) = {
            let state = self.inner.state.lock().unwrap();
            let active = state
                .running
                .keys()
                .copied()
                .chain(state.waiting.iter().map(|p| p.id))
                .collect();
            (state.queues.keys().copied().collect(), active)
        };
        for queue in queues {
            self.stop_queue_id(queue);
        }
        for id in active {
            self.pause(id);
        }
    }

    /// Continues every paused or failed download, all at once (each may wait for the
    /// browser's cookies). Returns how many it continued.
    pub async fn resume_all(&self) -> usize {
        let stopped: Vec<i64> = {
            let store = self.inner.store.lock().unwrap();
            store
                .list_downloads()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| matches!(r.status, Status::Paused | Status::Failed))
                .map(|r| r.id.0)
                .collect()
        };
        let mut tasks = tokio::task::JoinSet::new();
        for id in stopped {
            let manager = self.clone();
            tasks.spawn(async move { manager.resume(id).await.is_ok() });
        }
        let mut resumed = 0;
        while let Some(done) = tasks.join_next().await {
            resumed += usize::from(done.unwrap_or(false));
        }
        resumed
    }

    /// Where a download is on disk: the saved file once it completed, else the unfinished
    /// part file (a video stream's work folder), or just the folder before the file has a
    /// name.
    pub fn file_of(&self, id: i64) -> Result<PathBuf, String> {
        let record = self
            .inner
            .store
            .lock()
            .unwrap()
            .get_download(DownloadId(id))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no download #{id}"))?;
        let dir = PathBuf::from(&record.save_dir);
        Ok(match &record.file_name {
            Some(name) if record.status == Status::Completed => dir.join(name),
            Some(name) if record.media.is_some() => dir.join(format!("{name}{WORK_SUFFIX}")),
            Some(name) => dir.join(format!("{name}{PART_SUFFIX}")),
            None => dir,
        })
    }

    /// Removes a download from the list, stopping it first. An unfinished download's part
    /// file is deleted, since nothing can continue it any more; a completed download's
    /// file only with `delete_file`.
    pub async fn delete(&self, id: i64, delete_file: bool) -> Result<(), String> {
        // A capture waiting for an answer has nothing running yet.
        self.inner.state.lock().unwrap().asking.remove(&id);
        let mut updates = self.subscribe();
        if self.pause(id) {
            // Its last checkpoint must be written before the record goes.
            let stopped = tokio::time::timeout(STOP_WAIT, async {
                while self.is_active(id) {
                    match updates.recv().await {
                        Ok(Update::Finished(done)) if done.download_id == id => {}
                        Ok(_) | Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            })
            .await;
            if stopped.is_err() {
                return Err("the download didn't stop in time; try again".into());
            }
        }
        let file = self.file_of(id)?;
        self.inner
            .store
            .lock()
            .unwrap()
            .delete_download(DownloadId(id))
            .map_err(|e| e.to_string())?;
        self.forget_details(id);
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let removed = if name.ends_with(WORK_SUFFIX) && file.is_dir() {
            Some(std::fs::remove_dir_all(&file))
        } else if (delete_file || name.ends_with(PART_SUFFIX)) && file.is_file() {
            Some(std::fs::remove_file(&file))
        } else {
            None
        };
        if let Some(removed) = removed {
            match removed {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!(
                        "removed from the list, but {} couldn't be deleted: {e}",
                        file.display()
                    ));
                }
            }
        }
        Ok(())
    }
}
