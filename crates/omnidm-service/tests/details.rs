// SPDX-License-Identifier: GPL-3.0-or-later

//! A download's details: pieces, connections, server answers, redirects and log.

use std::time::Duration;

use omnidm_core::engine::EngineConfig;
use omnidm_core::{RampMode, RampPolicy, SplitPolicy};
use omnidm_ipc::protocol::{AddRequest, FinishStatus, Finished, Header};
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
                max_connections: 4,
                mode: RampMode::Doubling,
                cooldown: Duration::from_millis(100),
            },
            tick: Duration::from_millis(50),
            flush_interval: Duration::from_millis(100),
            checkpoint_interval: Duration::from_millis(200),
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
    fn add(&self, path: &str, headers: Vec<Header>) -> i64 {
        self.manager
            .add(AddRequest {
                url: self.server.url(path),
                dir: self.downloads.path().to_string_lossy().into_owned(),
                file_name: None,
                connections: None,
                headers,
                cookies: vec![],
                data: None,
                queue: None,
            })
            .unwrap()
    }
}

async fn finished(updates: &mut broadcast::Receiver<Update>, id: i64) -> Finished {
    let wait = async {
        loop {
            match updates.recv().await {
                Ok(Update::Finished(done)) if done.download_id == id => return done,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("updates ended: {e}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait)
        .await
        .expect("the download did not finish in time")
}

#[tokio::test(flavor = "multi_thread")]
async fn details_follow_a_download_from_start_to_end() {
    let env = env().await;
    env.server.set_file(
        "/big.bin",
        FileSpec::new(4 * MIB)
            .etag("v1")
            .content_type("application/octet-stream")
            .throttle(512 * KIB),
    );
    let mut updates = env.manager.subscribe();
    let id = env.add("/big.bin", vec![]);

    // While it runs: several pieces being downloaded at once, each with its speed.
    let mut seen_running = false;
    let mut seen = Vec::new();
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let Some(d) = env.manager.details(id) else {
            continue;
        };
        let active: Vec<_> = d.segments.iter().filter(|s| s.active).collect();
        if active.len() > 1
            && active.iter().filter(|s| s.speed > 0).count() > 1
            && d.speed.is_some_and(|s| s > 0)
            && d.connections > 1
        {
            seen_running = true;
            break;
        }
        seen.push((
            active.len(),
            active.iter().map(|s| s.speed).collect::<Vec<_>>(),
            d.speed,
            d.connections,
        ));
    }
    assert!(
        seen_running,
        "several working connections were seen: {seen:?}"
    );

    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
    let d = env.manager.details(id).unwrap();
    assert_eq!(d.summary.status, "completed");
    assert_eq!(d.speed, None);
    assert_eq!(d.connections, 0);
    assert!(
        d.segments
            .iter()
            .all(|s| !s.active && s.written == s.end - s.start + 1)
    );
    assert_eq!(d.segments.iter().map(|s| s.written).sum::<u64>(), 4 * MIB);
    assert!(
        d.segments.windows(2).all(|w| w[0].end + 1 == w[1].start),
        "in file order"
    );
    assert_eq!(d.content_type.as_deref(), Some("application/octet-stream"));
    assert_eq!(d.etag.as_deref(), Some("\"v1\""));
    assert!(d.resumable);
    assert_eq!(d.method, "GET");

    let log: Vec<&str> = d.log.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(log[0], "Started", "{log:?}");
    assert!(
        log[1].starts_with("The server sent 4.0 MB (application/octet-stream)"),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l.starts_with("Opened connection 2")),
        "{log:?}"
    );
    assert!(
        log.last().unwrap().starts_with("Completed: saved as "),
        "{log:?}"
    );
    assert!(d.log.windows(2).all(|w| w[0].at <= w[1].at));
}

#[tokio::test(flavor = "multi_thread")]
async fn redirects_and_headers_are_shown_with_secrets_hidden() {
    let env = env().await;
    env.server.set_file("/file.bin", FileSpec::new(64 * KIB));
    env.server.redirect("/go", "/file.bin", 302);
    let mut updates = env.manager.subscribe();
    let id = env.add(
        "/go",
        vec![
            Header {
                name: "Authorization".into(),
                value: "Bearer secret".into(),
            },
            Header {
                name: "X-Team".into(),
                value: "blue".into(),
            },
        ],
    );
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
    let d = env.manager.details(id).unwrap();
    assert_eq!(d.redirects.len(), 1);
    assert_eq!(d.redirects[0].status, 302);
    assert_eq!(d.redirects[0].url, env.server.url("/file.bin"));
    assert_eq!(
        d.final_url.as_deref(),
        Some(env.server.url("/file.bin").as_str())
    );
    let header = |name: &str| {
        d.request_headers
            .iter()
            .find(|h| h.name == name)
            .map(|h| h.value.clone())
    };
    assert_eq!(header("Authorization").as_deref(), Some("(hidden)"));
    assert_eq!(header("X-Team").as_deref(), Some("blue"));
    assert!(
        d.log
            .iter()
            .any(|l| l.text == format!("Redirected (302) to {}", env.server.url("/file.bin")))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_is_logged_and_deleting_forgets_the_details() {
    let env = env().await;
    env.server
        .set_file("/gone.bin", FileSpec::new(64 * KIB).fail(404, 10, None));
    let mut updates = env.manager.subscribe();
    let id = env.add("/gone.bin", vec![]);
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Failed
    );
    let d = env.manager.details(id).unwrap();
    assert!(
        d.log.last().unwrap().text.starts_with("Failed: "),
        "{:?}",
        d.log
    );
    assert!(d.segments.is_empty());

    env.manager.delete(id, true).await.unwrap();
    assert!(env.manager.details(id).is_none());
    assert!(env.manager.details(9999).is_none());
}
