// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs one download that is already in the store, from its probe (or saved progress)
//! to the renamed, finished file, keeping its status up to date.

use std::fmt;
use std::path::{Path, PathBuf};

use omnidm_core::engine::{
    Download, EngineConfig, EngineError, Event, Outcome, Probe, Request, Resume, download, probe,
};
use omnidm_net::naming::choose_file_name;
use omnidm_store::checkpoint::StoreCheckpointer;
use omnidm_store::{DownloadId, Status, Store, StoreError};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Appended to a file's name while it is being downloaded.
pub const PART_SUFFIX: &str = ".omnidm-part";

/// One download to run.
pub struct Job {
    /// The database file; the job opens its own connection.
    pub database: PathBuf,
    pub id: DownloadId,
    /// The folder the file is saved in.
    pub dir: PathBuf,
    /// The name chosen by the user or by an earlier probe; otherwise picked after the probe.
    pub file_name: Option<String>,
    /// The first response, if the caller probed already.
    pub probe: Option<Probe>,
    /// Saved progress to continue from (used only together with a known `file_name`).
    pub resume: Option<Resume>,
    pub client: reqwest::Client,
    pub request: Request,
    pub config: EngineConfig,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// The finished file.
    Completed(PathBuf),
    /// Stopped on request; progress is saved.
    Paused,
}

#[derive(Debug)]
pub enum JobError {
    Store(StoreError),
    /// The download failed, or (for [`EngineError::LinkExpired`]) paused with progress saved.
    Engine(EngineError),
    Io(std::io::Error),
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Engine(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for JobError {}

impl From<StoreError> for JobError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<std::io::Error> for JobError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Runs `job`, calling `progress` with every engine event.
pub async fn run_job(job: Job, mut progress: impl FnMut(&Event)) -> Result<JobOutcome, JobError> {
    let Job {
        database,
        id,
        dir,
        file_name,
        probe: first,
        resume,
        client,
        request,
        config,
        cancel,
    } = job;
    let mut store = Store::open(&database)?;
    store.set_status(id, Status::Running, None)?;

    // A fresh download asks the server first, so the part file gets the real name.
    let (file_name, first) = match (file_name, first, &resume) {
        (Some(name), first, Some(_)) => (name, first),
        (known, Some(first), None) => (known.unwrap_or_else(|| name_from(&first)), Some(first)),
        (known, _, _) => match probe(&client, &request, &config, &cancel).await {
            Ok(Some(first)) => (known.unwrap_or_else(|| name_from(&first)), Some(first)),
            Ok(None) => {
                store.set_status(id, Status::Paused, None)?;
                return Ok(JobOutcome::Paused);
            }
            Err(e) => {
                store.set_status(id, Status::Failed, Some(&e.to_string()))?;
                return Err(JobError::Engine(e));
            }
        },
    };
    // Saved progress belongs to the old part file; a fresh probe starts over.
    let resume = resume.filter(|_| first.is_none());

    let part = dir.join(format!("{file_name}{PART_SUFFIX}"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let result = {
        let mut checkpointer = StoreCheckpointer {
            store: &mut store,
            id,
            file_name: file_name.clone(),
        };
        let run = download(
            Download {
                client: &client,
                request: &request,
                part_path: &part,
                resume,
                probe: first,
                config: &config,
                events: Some(tx),
                cancel,
            },
            &mut checkpointer,
        );
        tokio::pin!(run);
        loop {
            tokio::select! {
                result = &mut run => break result,
                Some(event) = rx.recv() => progress(&event),
            }
        }
    };
    while let Ok(event) = rx.try_recv() {
        progress(&event);
    }

    match result {
        Ok(Outcome::Completed { .. }) => {
            let target = unique_path(&dir, &file_name, Path::exists);
            std::fs::rename(&part, &target)?;
            store.set_status(id, Status::Completed, None)?;
            Ok(JobOutcome::Completed(target))
        }
        Ok(Outcome::Paused) => {
            store.set_status(id, Status::Paused, None)?;
            Ok(JobOutcome::Paused)
        }
        // Progress is saved; the download continues once the link works again.
        Err(e @ EngineError::LinkExpired) => {
            store.set_status(id, Status::Paused, None)?;
            Err(JobError::Engine(e))
        }
        Err(e) => {
            store.set_status(id, Status::Failed, Some(&e.to_string()))?;
            Err(JobError::Engine(e))
        }
    }
}

fn name_from(first: &Probe) -> String {
    choose_file_name(
        first.info.content_disposition.as_deref(),
        &first.info.final_url,
        first.info.content_type.as_deref(),
    )
}

/// `dir/name`, or `dir/name (1).ext`, `dir/name (2).ext`… if that is taken.
pub fn unique_path(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    (1u32..)
        .map(|n| match extension {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|candidate| !exists(candidate))
        .expect("some number is free")
}
