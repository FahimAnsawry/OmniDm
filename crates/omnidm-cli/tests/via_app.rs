// SPDX-License-Identifier: GPL-3.0-or-later

//! `omnidm get` and `omnidm list` through a running app, over the protocol.

use std::time::Duration;

use omnidm_cli::{CliError, GetOptions, GetResult, get_via_app, list_via_app};
use omnidm_core::engine::EngineConfig;
use omnidm_core::{RampMode, RampPolicy, SplitPolicy};
use omnidm_service::manager::{Manager, ManagerConfig};
use omnidm_service::server::handle_connection;
use omnidm_testserver::{FileSpec, TestServer};
use tokio::io::DuplexStream;
use tokio_util::sync::CancellationToken;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

struct Env {
    server: TestServer,
    manager: Manager,
    downloads: tempfile::TempDir,
    _data: tempfile::TempDir,
}

async fn env() -> Env {
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
            tick: Duration::from_millis(20),
            checkpoint_interval: Duration::from_millis(50),
            flush_interval: Duration::from_millis(100),
            ..EngineConfig::default()
        },
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();
    Env {
        server: TestServer::start().await.unwrap(),
        manager,
        downloads,
        _data: data,
    }
}

impl Env {
    /// A fresh connection to the app, as the CLI makes.
    fn connect(&self) -> DuplexStream {
        let (ours, theirs) = tokio::io::duplex(1 << 20);
        tokio::spawn(handle_connection(self.manager.clone(), theirs));
        ours
    }

    fn options(&self, path: &str) -> GetOptions {
        GetOptions {
            url: self.server.url(path),
            output: None,
            dir: self.downloads.path().to_path_buf(),
            connections: 4,
            headers: vec![],
            cookies: vec![],
            data: None,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_app_downloads_and_the_cli_reports_progress() {
    let env = env().await;
    let spec = FileSpec::new(2 * MIB).etag("v1").throttle(4 * MIB);
    env.server.set_file("/files/report.bin", spec.clone());
    let mut updates = 0;

    let result = get_via_app(
        env.connect(),
        env.options("/files/report.bin"),
        CancellationToken::new(),
        |_| updates += 1,
    )
    .await
    .unwrap();

    let path = env.downloads.path().join("report.bin");
    assert_eq!(result, GetResult::Completed(path.clone()));
    assert!(std::fs::read(&path).unwrap() == spec.bytes(0, spec.size as usize));
    assert!(updates > 0, "progress was reported");
}

#[tokio::test(flavor = "multi_thread")]
async fn ctrl_c_pauses_the_download_in_the_app() {
    let env = env().await;
    env.server
        .set_file("/big.bin", FileSpec::new(8 * MIB).etag("v1").throttle(MIB));
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        stopper.cancel();
    });

    let result = get_via_app(env.connect(), env.options("/big.bin"), cancel, |_| {})
        .await
        .unwrap();

    assert_eq!(result, GetResult::Paused);
    assert_eq!(env.manager.list()[0].status, "paused");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_in_the_app_is_reported() {
    let env = env().await;

    let result = get_via_app(
        env.connect(),
        env.options("/missing.bin"),
        CancellationToken::new(),
        |_| {},
    )
    .await;

    match result {
        Err(CliError::App(message)) => assert!(message.contains("404"), "{message}"),
        other => panic!("expected an app error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_shows_the_apps_downloads() {
    let env = env().await;
    env.server
        .set_file("/a.bin", FileSpec::new(64 * KIB).etag("v1"));
    get_via_app(
        env.connect(),
        env.options("/a.bin"),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();

    let downloads = list_via_app(env.connect()).await.unwrap();

    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].status, "completed");
    assert_eq!(downloads[0].file_name.as_deref(), Some("a.bin"));
}
