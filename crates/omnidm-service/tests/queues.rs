// SPDX-License-Identifier: GPL-3.0-or-later

//! Queues in the download manager: order, limits, one try per run, stop, finish.

use std::collections::HashSet;
use std::time::Duration;

use omnidm_core::engine::{EngineConfig, RetryPolicy};
use omnidm_core::{RampMode, RampPolicy, SplitPolicy};
use omnidm_ipc::protocol::{AddRequest, FinishStatus, NewQueue, QueueEdit};
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
        engine: EngineConfig {
            split: SplitPolicy {
                min_segment: 128 * KIB,
                align: 4 * KIB,
            },
            ramp: RampPolicy {
                max_connections: 2,
                mode: RampMode::Doubling,
                cooldown: Duration::from_millis(100),
            },
            tick: Duration::from_millis(20),
            checkpoint_interval: Duration::from_millis(50),
            flush_interval: Duration::from_millis(100),
            retry: RetryPolicy {
                max_attempts: 2,
                base_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(50),
            },
            ..EngineConfig::default()
        },
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
    /// A file that takes about a second to download.
    fn slow_file(&self, path: &str) {
        self.server
            .set_file(path, FileSpec::new(MIB).etag(path).throttle(512 * KIB));
    }

    /// Adds `path` to `queue`; returns its download id.
    fn add_to(&self, queue: &str, path: &str) -> i64 {
        self.manager
            .add(AddRequest {
                url: self.server.url(path),
                dir: self.downloads.path().to_string_lossy().into_owned(),
                file_name: None,
                connections: None,
                headers: vec![],
                cookies: vec![],
                data: None,
                queue: Some(queue.into()),
            })
            .unwrap()
    }

    fn status(&self, id: i64) -> String {
        self.manager
            .list()
            .into_iter()
            .find(|d| d.download_id == id)
            .unwrap()
            .status
    }

    fn make_queue(&self, name: &str, max: u32) -> i64 {
        self.manager
            .create_queue(NewQueue {
                name: name.into(),
                max_concurrent: Some(max),
                on_complete: None,
            })
            .unwrap()
    }
}

/// Waits for the next update `wanted` accepts.
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

async fn queue_finished(updates: &mut broadcast::Receiver<Update>, queue: i64) -> usize {
    wait_for(updates, |u| match u {
        Update::QueueFinished { queue_id, tried } if *queue_id == queue => Some(*tried),
        _ => None,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_download_waits_until_its_queue_starts() {
    let env = env().await;
    env.slow_file("/a.bin");
    let a = env.add_to("main", "/a.bin");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.status(a), "queued");
    assert!(!env.manager.is_active(a));
    let summary = env.manager.list().into_iter().find(|d| d.download_id == a);
    assert_eq!(summary.unwrap().queue.as_deref(), Some("Main"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_started_queue_runs_in_order_within_its_limit() {
    let env = env().await;
    let night = env.make_queue("Night", 2);
    for path in ["/a.bin", "/b.bin", "/c.bin"] {
        env.slow_file(path);
    }
    let a = env.add_to("Night", "/a.bin");
    let b = env.add_to("Night", "/b.bin");
    let c = env.add_to("Night", "/c.bin");
    let mut updates = env.manager.subscribe();

    env.manager.start_queue("night").unwrap();

    let mut active = HashSet::new();
    let mut started = Vec::new();
    let mut most = 0;
    let mut done = 0;
    let tried = wait_for(&mut updates, |u| {
        match u {
            Update::Progress(p) => {
                if active.insert(p.download_id) {
                    started.push(p.download_id);
                }
            }
            Update::Finished(f) => {
                assert_eq!(f.status, FinishStatus::Completed, "{f:?}");
                active.remove(&f.download_id);
                done += 1;
            }
            Update::QueueFinished { queue_id, tried } if *queue_id == night => {
                return Some(*tried);
            }
            _ => {}
        }
        most = most.max(active.len());
        None
    })
    .await;

    assert_eq!(tried, 3);
    assert_eq!(done, 3);
    assert!(most <= 2, "{most} ran at once");
    assert_eq!(started.last(), Some(&c), "the third waits for a free slot");
    assert!(started.contains(&a) && started.contains(&b));
    assert!(env.manager.queues().iter().all(|q| !q.running));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_download_is_tried_once_per_run() {
    let env = env().await;
    env.slow_file("/good.bin");
    // Nothing is served at /missing.bin.
    let bad = env.add_to("Main", "/missing.bin");
    let good = env.add_to("Main", "/good.bin");
    let mut updates = env.manager.subscribe();

    env.manager.start_queue("Main").unwrap();
    assert_eq!(queue_finished(&mut updates, 1).await, 2);
    assert_eq!(env.status(bad), "failed");
    assert_eq!(env.status(good), "completed");

    // A new run tries the failed one again, and skips the finished one.
    env.manager.start_queue("Main").unwrap();
    assert_eq!(queue_finished(&mut updates, 1).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_an_empty_queue_finishes_with_nothing_tried() {
    let env = env().await;
    let mut updates = env.manager.subscribe();
    env.manager.start_queue("Main").unwrap();
    assert_eq!(queue_finished(&mut updates, 1).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_queue_pauses_its_downloads_without_finishing_it() {
    let env = env().await;
    env.server.set_file(
        "/big.bin",
        FileSpec::new(8 * MIB).etag("big").throttle(256 * KIB),
    );
    let big = env.add_to("Main", "/big.bin");
    let mut updates = env.manager.subscribe();
    env.manager.start_queue("Main").unwrap();
    wait_for(&mut updates, |u| {
        matches!(u, Update::Progress(p) if p.download_id == big).then_some(())
    })
    .await;
    assert!(env.manager.queues()[0].running);

    env.manager.stop_queue("Main").unwrap();
    let status = wait_for(&mut updates, |u| match u {
        Update::QueueFinished { .. } => panic!("a stopped queue doesn't finish"),
        Update::Finished(f) if f.download_id == big => Some(f.status),
        _ => None,
    })
    .await;
    assert_eq!(status, FinishStatus::Paused);
    assert!(!env.manager.queues()[0].running);
    let quiet = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Ok(Update::QueueFinished { .. }) = updates.recv().await {
                return;
            }
        }
    })
    .await;
    assert!(quiet.is_err(), "no finish after a stop");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_queue_picks_up_new_downloads_and_a_higher_limit() {
    let env = env().await;
    env.make_queue("Night", 1);
    env.server.set_file(
        "/a.bin",
        FileSpec::new(4 * MIB).etag("a").throttle(256 * KIB),
    );
    env.server.set_file(
        "/b.bin",
        FileSpec::new(4 * MIB).etag("b").throttle(256 * KIB),
    );
    let a = env.add_to("Night", "/a.bin");
    let mut updates = env.manager.subscribe();
    env.manager.start_queue("Night").unwrap();
    wait_for(&mut updates, |u| {
        matches!(u, Update::Progress(p) if p.download_id == a).then_some(())
    })
    .await;

    // Added while running: it waits, because the limit is 1.
    let b = env.add_to("Night", "/b.bin");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!env.manager.is_active(b) || env.status(b) == "queued");

    env.manager
        .edit_queue(QueueEdit {
            queue: "Night".into(),
            rename: None,
            max_concurrent: Some(2),
            on_complete: None,
            schedule: None,
        })
        .unwrap();
    wait_for(&mut updates, |u| {
        matches!(u, Update::Progress(p) if p.download_id == b).then_some(())
    })
    .await;
    assert!(env.manager.is_active(a), "both run at once now");
    env.manager.stop_queue("Night").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_changes_are_checked() {
    let env = env().await;
    let new = |name: &str, max: Option<u32>, then: Option<&str>| NewQueue {
        name: name.into(),
        max_concurrent: max,
        on_complete: then.map(String::from),
    };
    let m = &env.manager;
    m.create_queue(new("Night", None, Some("sleep"))).unwrap();
    let night = m.queues().into_iter().find(|q| q.name == "Night").unwrap();
    assert_eq!(
        (night.max_concurrent, night.on_complete.as_str()),
        (3, "sleep")
    );

    assert!(
        m.create_queue(new("NIGHT", None, None))
            .unwrap_err()
            .contains("already exists")
    );
    assert!(m.create_queue(new("  ", None, None)).is_err());
    assert!(m.create_queue(new("Zero", Some(0), None)).is_err());
    assert!(m.create_queue(new("Many", Some(17), None)).is_err());
    assert!(
        m.create_queue(new("Odd", None, Some("reboot")))
            .unwrap_err()
            .contains("unknown action")
    );
    assert!(m.start_queue("Nope").unwrap_err().starts_with("no queue"));
    assert!(
        m.delete_queue("main")
            .unwrap_err()
            .contains("can't be deleted")
    );
    assert!(
        m.queue_add("Main", &[999])
            .unwrap_err()
            .starts_with("no download")
    );
    assert!(m.queue_move(999, 0).is_err());

    let rename = |from: &str, to: &str| {
        m.edit_queue(QueueEdit {
            queue: from.into(),
            rename: Some(to.into()),
            max_concurrent: None,
            on_complete: None,
            schedule: None,
        })
    };
    assert!(
        rename("Night", "main")
            .unwrap_err()
            .contains("already exists")
    );
    rename("Night", "Overnight").unwrap();
    rename("Overnight", "OVERNIGHT").unwrap();
    m.delete_queue("overnight").unwrap();
    assert_eq!(m.queues().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_move_between_and_within_queues() {
    let env = env().await;
    env.make_queue("Night", 3);
    let a = env.add_to("Main", "/a.bin");
    let b = env.add_to("Main", "/b.bin");
    let c = env.add_to("Main", "/c.bin");
    let m = &env.manager;
    let order = |name: &str| {
        m.queues()
            .into_iter()
            .find(|q| q.name == name)
            .unwrap()
            .download_ids
    };
    m.queue_move(c, 0).unwrap();
    assert_eq!(order("Main"), [c, a, b]);
    m.queue_add("Night", &[a]).unwrap();
    assert_eq!(order("Main"), [c, b]);
    assert_eq!(order("Night"), [a]);
    m.queue_remove(&[b]).unwrap();
    assert_eq!(order("Main"), [c]);
    let b_queue = m.list().into_iter().find(|d| d.download_id == b).unwrap();
    assert_eq!(b_queue.queue, None);
}
