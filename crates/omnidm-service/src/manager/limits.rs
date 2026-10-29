// SPDX-License-Identifier: GPL-3.0-or-later

//! Settings that apply while running: the downloads folder, how many downloads run at
//! once, and how many connections a download may use.

use std::path::PathBuf;

use omnidm_core::ramp::HARD_CAP;

use super::{Manager, ManagerConfig};

/// The most downloads outside queues that can run at once.
pub const MAX_ACTIVE_LIMIT: usize = 16;

pub(super) struct Limits {
    downloads_dir: PathBuf,
    max_active: usize,
    max_connections: u32,
}

impl From<&ManagerConfig> for Limits {
    fn from(config: &ManagerConfig) -> Self {
        Self {
            downloads_dir: config.downloads_dir.clone(),
            max_active: config.max_active.clamp(1, MAX_ACTIVE_LIMIT),
            max_connections: config.engine.ramp.max_connections.clamp(1, HARD_CAP),
        }
    }
}

impl Manager {
    /// Where downloads go when their category has no folder of its own.
    pub fn downloads_folder(&self) -> PathBuf {
        self.inner.limits.lock().unwrap().downloads_dir.clone()
    }

    /// Changes the downloads folder for downloads added from now on. It is made when a
    /// download first needs it.
    pub fn set_downloads_folder(&self, folder: PathBuf) -> Result<(), String> {
        if !folder.is_absolute() {
            return Err(format!(
                "{:?} isn't a full path, like D:\\Downloads",
                folder.display().to_string()
            ));
        }
        self.inner.limits.lock().unwrap().downloads_dir = folder;
        Ok(())
    }

    /// Downloads outside queues that run at once.
    pub fn max_active(&self) -> usize {
        self.inner.limits.lock().unwrap().max_active
    }

    /// Changes how many downloads outside queues run at once (1 to 16). Raising it starts
    /// waiting downloads at once; lowering it lets running ones finish.
    pub fn set_max_active(&self, max: usize) {
        let max = max.clamp(1, MAX_ACTIVE_LIMIT);
        self.inner.limits.lock().unwrap().max_active = max;
        loop {
            let next = {
                let mut state = self.inner.state.lock().unwrap();
                if !state.has_free_slot(max) {
                    break;
                }
                match state.waiting.pop_front() {
                    Some(next) => next,
                    None => break,
                }
            };
            self.start_now(next);
        }
    }

    /// The most connections one download uses, unless it was given its own number.
    pub fn max_connections(&self) -> u32 {
        self.inner.limits.lock().unwrap().max_connections
    }

    /// Changes the connections per download (1 to 32) for downloads that start from now on.
    pub fn set_max_connections(&self, max: u32) {
        self.inner.limits.lock().unwrap().max_connections = max.clamp(1, HARD_CAP);
    }
}
