// SPDX-License-Identifier: GPL-3.0-or-later

//! The capture prompt: a captured download is held, its first response kept, until the
//! user says where to save it and whether to start now or later in a queue. Also the
//! app's saved settings.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use omnidm_ipc::protocol::{AskAbout, CaptureAnswer};
use omnidm_net::naming::sanitize_file_name;
use omnidm_store::{CategoryId, DownloadId, Status};

use super::{Manager, Pending, Update, categories, queues};

/// How long the server's first response is kept for a held capture. After that it is
/// dropped (a server may give up on a connection nobody reads), and the download asks the
/// server again when it starts.
const PROBE_KEPT: Duration = Duration::from_secs(60);

/// A capture waiting for the user's answer.
pub(super) struct Held {
    pub(super) pending: Pending,
    pub(super) about: AskAbout,
}

impl Manager {
    /// Whether captures wait for the user's answer (off until the app turns it on).
    pub fn set_ask_before_download(&self, ask: bool) {
        self.inner.ask_first.store(ask, Ordering::Relaxed);
    }

    pub fn asks_before_download(&self) -> bool {
        self.inner.ask_first.load(Ordering::Relaxed)
    }

    /// A saved setting; `None` if it was never set.
    pub fn setting(&self, key: &str) -> Option<String> {
        self.inner.store.lock().unwrap().setting(key).ok().flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.inner
            .store
            .lock()
            .unwrap()
            .set_setting(key, value)
            .map_err(|e| e.to_string())
    }

    /// Where downloads go when their category has no folder of its own.
    pub fn downloads_folder(&self) -> &Path {
        &self.inner.config.downloads_dir
    }

    /// The category a link belongs to by its name (`file_name`, else the end of the URL),
    /// and the folder it would be saved in. Nothing is created.
    pub fn suggest_place(&self, url: &str, file_name: Option<&str>) -> (Option<String>, PathBuf) {
        let name = file_name
            .map(String::from)
            .unwrap_or_else(|| categories::name_from_url(url));
        match self.category_for(&name, None, url) {
            Some(category) => {
                let folder = category
                    .settings
                    .save_dir
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.downloads_folder().to_path_buf());
                (Some(category.settings.name), folder)
            }
            None => (None, self.downloads_folder().to_path_buf()),
        }
    }

    /// Holds a probed capture and asks the user about it.
    pub(super) fn ask(
        &self,
        pending: Pending,
        url: &str,
        size: Option<u64>,
        resumable: bool,
        category: Option<CategoryId>,
    ) {
        let id = pending.id;
        let category = category.and_then(|c| {
            let store = self.inner.store.lock().unwrap();
            store
                .get_category(c)
                .ok()
                .flatten()
                .map(|c| c.settings.name)
        });
        // If OmniDm closes before the answer, the download is left paused, not lost.
        let _ = self
            .inner
            .store
            .lock()
            .unwrap()
            .set_status(DownloadId(id), Status::Paused, None);
        let about = AskAbout {
            download_id: id,
            url: url.to_string(),
            file_name: pending.file_name.clone().unwrap_or_default(),
            size,
            folder: pending.dir.to_string_lossy().into_owned(),
            category,
            resumable,
            downloads_folder: self.downloads_folder().to_string_lossy().into_owned(),
        };
        self.inner.state.lock().unwrap().asking.insert(
            id,
            Held {
                pending,
                about: about.clone(),
            },
        );
        let manager = self.clone();
        let wait = tokio::time::sleep(PROBE_KEPT);
        tokio::spawn(async move {
            wait.await;
            if let Some(held) = manager.inner.state.lock().unwrap().asking.get_mut(&id) {
                held.pending.probe = None;
            }
        });
        self.send(Update::Ask(about));
    }

    pub(super) fn is_asking(&self, id: i64) -> bool {
        self.inner.state.lock().unwrap().asking.contains_key(&id)
    }

    /// The captures waiting for an answer, oldest first.
    pub fn asking(&self) -> Vec<AskAbout> {
        let mut waiting: Vec<AskAbout> = self
            .inner
            .state
            .lock()
            .unwrap()
            .asking
            .values()
            .map(|held| held.about.clone())
            .collect();
        waiting.sort_by_key(|about| about.download_id);
        waiting
    }

    /// Starts a held capture under the chosen name and folder, or puts it in a queue. On an
    /// error it stays held, so the user can correct the answer.
    pub fn answer_capture(&self, answer: CaptureAnswer) -> Result<(), String> {
        let id = answer.download_id;
        let mut held = self
            .inner
            .state
            .lock()
            .unwrap()
            .asking
            .remove(&id)
            .ok_or_else(|| format!("no download #{id} is waiting for an answer"))?;
        let queue = match self.apply_answer(&mut held.pending, &answer) {
            Ok(queue) => queue,
            Err(e) => {
                self.inner.state.lock().unwrap().asking.insert(id, held);
                return Err(e);
            }
        };
        match queue {
            Some(queue) => {
                // The kept response is dropped; the queue starts it afresh.
                drop(held);
                self.fill_queue(queue);
            }
            None => self.start_now(held.pending),
        }
        Ok(())
    }

    /// Saves the answer's name, folder, category and queue. Returns the queue, if any.
    fn apply_answer(
        &self,
        pending: &mut Pending,
        answer: &CaptureAnswer,
    ) -> Result<Option<i64>, String> {
        let given = answer.file_name.trim();
        if given.is_empty() {
            return Err("the file needs a name".into());
        }
        let name = sanitize_file_name(given);
        let folder = PathBuf::from(answer.folder.trim());
        if !folder.is_absolute() {
            return Err(format!(
                "{:?} isn't a full path, like D:\\Downloads",
                answer.folder
            ));
        }
        std::fs::create_dir_all(&folder).map_err(|e| format!("can't create the folder: {e}"))?;
        let id = DownloadId(pending.id);
        let mut store = self.inner.store.lock().unwrap();
        let category = match answer.category.as_deref().map(str::trim) {
            Some(wanted) if !wanted.is_empty() => {
                let all = store.list_categories().map_err(|e| e.to_string())?;
                Some(categories::find(&all, wanted)?.id)
            }
            _ => None,
        };
        let queue = match &answer.queue {
            Some(name) => Some(queues::find(&store, name)?.id),
            None => None,
        };
        let saved = (|| {
            store.set_save_dir(id, &folder.to_string_lossy())?;
            store.set_file_name(id, &name)?;
            store.set_download_category(id, category)?;
            if let Some(queue) = queue {
                store.add_to_queue(queue, id)?;
            }
            store.set_status(id, Status::Queued, None)
        })();
        saved.map_err(|e| e.to_string())?;
        pending.dir = folder;
        pending.file_name = Some(name);
        Ok(queue.map(|q| q.0))
    }

    /// Drops a held capture: it leaves the list, and nothing was saved to disk yet.
    pub fn cancel_capture(&self, id: i64) -> Result<(), String> {
        self.inner
            .state
            .lock()
            .unwrap()
            .asking
            .remove(&id)
            .ok_or_else(|| format!("no download #{id} is waiting for an answer"))?;
        self.inner
            .store
            .lock()
            .unwrap()
            .delete_download(DownloadId(id))
            .map_err(|e| e.to_string())
    }
}
