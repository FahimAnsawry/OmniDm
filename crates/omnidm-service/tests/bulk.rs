// SPDX-License-Identifier: GPL-3.0-or-later

//! Deleting downloads, and pausing or resuming them all at once.

use std::path::PathBuf;
use std::time::Duration;

use omnidm_ipc::protocol::{AddRequest, FinishStatus, Finished, Progress};
use omnidm_service::manager::{Manager, ManagerConfig, Update};
use omnidm_testserver::{FileSpec, TestServer};
use tokio::sync::broadcast;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

struct Env {
    server: TestServer,
    manager: Manager,
    _data: tempfile::TempDir,
    downloads: tempfile::TempDir,
}

async fn env() -> Env {
    let server = TestServer::start().await.unwrap();
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: downloads.path().to_path_buf(),
        max_active: 3,
        engine: Default::default(),
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();
    Env {
        server,
        manager,
        _data: data,
        downloads,
    }
}

impl Env {
    /// Starts downloading `path`; returns its download id.
    fn add(&self, path: &str) -> i64 {
        self.manager
            .add(AddRequest {
                url: self.server.url(path),
                dir: self.downloads.path().to_string_lossy().into_owned(),
                file_name: None,
                connections: None,
                headers: vec![],
                cookies: vec![],
                data: None,
                queue: None,
            })
            .unwrap()
    }

    /// A file that takes several seconds to download.
    fn slow_file(&self, path: &str) {
        self.server
            .set_file(path, FileSpec::new(4 * MIB).etag(path).throttle(256 * KIB));
    }

    fn status(&self, id: i64) -> Option<String> {
        self.manager
            .list()
            .into_iter()
            .find(|d| d.download_id == id)
            .map(|d| d.status)
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.downloads.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

async fn wait_for<T>(
    updates: &mut broadcast::Receiver<Update>,
    mut wanted: impl FnMut(&Update) -> Option<T>,
) -> T {
    let wait = async {
        loop {
            match updates.recv().await {
                Ok(update) => {
                    if let Some(found) = wanted(&update) {
                        return found;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("updates ended: {e}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait)
        .await
        .expect("timed out waiting for an update")
}

async fn finished(updates: &mut broadcast::Receiver<Update>, id: i64) -> Finished {
    wait_for(updates, |u| match u {
        Update::Finished(done) if done.download_id == id => Some(done.clone()),
        _ => None,
    })
    .await
}

/// Waits until `id` has written something.
async fn started(updates: &mut broadcast::Receiver<Update>, id: i64) {
    wait_for(updates, |u| match u {
        Update::Progress(Progress {
            download_id,
            downloaded,
            ..
        }) if *download_id == id && *downloaded > 0 => Some(()),
        _ => None,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_completed_download_keeps_its_file_unless_asked() {
    let env = env().await;
    env.server.set_file("/keep.bin", FileSpec::new(64 * KIB));
    env.server.set_file("/drop.bin", FileSpec::new(64 * KIB));
    let mut updates = env.manager.subscribe();
    let keep = env.add("/keep.bin");
    assert_eq!(
        finished(&mut updates, keep).await.status,
        FinishStatus::Completed
    );
    let drop = env.add("/drop.bin");
    assert_eq!(
        finished(&mut updates, drop).await.status,
        FinishStatus::Completed
    );

    env.manager.delete(keep, false).await.unwrap();
    env.manager.delete(drop, true).await.unwrap();
    assert_eq!(env.status(keep), None);
    assert_eq!(env.status(drop), None);
    assert_eq!(env.files(), ["keep.bin"]);
    assert!(
        env.manager
            .delete(keep, false)
            .await
            .unwrap_err()
            .starts_with("no download")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_running_download_stops_it_and_always_removes_the_part_file() {
    let env = env().await;
    env.slow_file("/big.bin");
    let mut updates = env.manager.subscribe();
    let id = env.add("/big.bin");
    started(&mut updates, id).await;
    assert_eq!(env.files(), ["big.bin.omnidm-part"]);

    env.manager.delete(id, false).await.unwrap();
    assert!(!env.manager.is_active(id));
    assert_eq!(env.status(id), None);
    assert!(env.files().is_empty(), "{:?}", env.files());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_saved_under_another_name_is_found_again() {
    let env = env().await;
    std::fs::write(env.downloads.path().join("taken.bin"), "someone else's").unwrap();
    env.server.set_file("/taken.bin", FileSpec::new(64 * KIB));
    let mut updates = env.manager.subscribe();
    let id = env.add("/taken.bin");
    let done = finished(&mut updates, id).await;
    let saved = PathBuf::from(done.path.unwrap());
    assert_eq!(saved.file_name().unwrap(), "taken (1).bin");
    let summary = env
        .manager
        .list()
        .into_iter()
        .find(|d| d.download_id == id)
        .unwrap();
    assert_eq!(summary.file_name.as_deref(), Some("taken (1).bin"));

    // Deleting removes the right file.
    env.manager.delete(id, true).await.unwrap();
    assert_eq!(env.files(), ["taken.bin"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_all_and_resume_all() {
    let env = env().await;
    env.slow_file("/a.bin");
    env.slow_file("/b.bin");
    env.server.set_file("/done.bin", FileSpec::new(64 * KIB));
    let mut updates = env.manager.subscribe();
    let done = env.add("/done.bin");
    finished(&mut updates, done).await;
    let a = env.add("/a.bin");
    let b = env.add("/b.bin");
    started(&mut updates, a).await;

    env.manager.pause_all();
    let mut paused = std::collections::HashSet::new();
    while paused.len() < 2 {
        let id = wait_for(&mut updates, |u| match u {
            Update::Finished(done) if [a, b].contains(&done.download_id) => {
                assert_eq!(done.status, FinishStatus::Paused);
                Some(done.download_id)
            }
            _ => None,
        })
        .await;
        paused.insert(id);
    }
    assert!(!env.manager.has_running());

    assert_eq!(
        env.manager.resume_all().await,
        2,
        "the completed one stays as it is"
    );
    assert!(env.manager.is_active(a) && env.manager.is_active(b));
    assert_eq!(env.status(done).as_deref(), Some("completed"));
    env.manager.pause_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_restart_pauses_everything_and_starts_it_again_once() {
    let env = env().await;
    env.slow_file("/alone.bin");
    env.slow_file("/queued.bin");
    env.slow_file("/stopped.bin");
    env.manager
        .create_queue(omnidm_ipc::protocol::NewQueue {
            name: "Night".into(),
            max_concurrent: None,
            on_complete: None,
        })
        .unwrap();
    let mut updates = env.manager.subscribe();
    let alone = env.add("/alone.bin");
    let queued = env
        .manager
        .add(AddRequest {
            url: env.server.url("/queued.bin"),
            dir: env.downloads.path().to_string_lossy().into_owned(),
            file_name: None,
            connections: None,
            headers: vec![],
            cookies: vec![],
            data: None,
            queue: Some("Night".into()),
        })
        .unwrap();
    let stopped = env.add("/stopped.bin");
    started(&mut updates, stopped).await;
    env.manager.pause(stopped);
    env.manager.start_queue("Night").unwrap();
    started(&mut updates, alone).await;
    started(&mut updates, queued).await;

    env.manager.pause_for_restart().await.unwrap();
    assert!(!env.manager.has_running());
    let night_running = |m: &Manager| m.queues().iter().any(|q| q.name == "Night" && q.running);
    assert!(!night_running(&env.manager));

    assert_eq!(env.manager.resume_after_restart().await, 1);
    assert!(env.manager.is_active(alone) && env.manager.is_active(queued));
    assert!(night_running(&env.manager));
    assert!(
        !env.manager.is_active(stopped),
        "one the user paused stays paused"
    );
    assert_eq!(env.manager.resume_after_restart().await, 0, "only once");
    env.manager.pause_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn summaries_say_when_downloads_were_added_and_completed() {
    let env = env().await;
    env.server.set_file("/t.bin", FileSpec::new(64 * KIB));
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut updates = env.manager.subscribe();
    let id = env.add("/t.bin");
    finished(&mut updates, id).await;
    let summary = env
        .manager
        .list()
        .into_iter()
        .find(|d| d.download_id == id)
        .unwrap();
    assert!((before..before + 60).contains(&summary.added_at));
    assert!(summary.completed_at.unwrap() >= summary.added_at);
}
