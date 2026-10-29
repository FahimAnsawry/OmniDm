// SPDX-License-Identifier: GPL-3.0-or-later

//! The download manager behind the app: captures from the browser, downloads added by
//! the CLI, pause and resume, saved cookies, and the waiting line.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use omnidm_core::engine::{EngineConfig, RetryPolicy};
use omnidm_core::{RampMode, RampPolicy, SplitPolicy};
use omnidm_ipc::protocol::{
    AddRequest, BrowserCookie, Capture, CaptureResult, FinishStatus, Finished, Header, RejectReason,
};
use omnidm_service::manager::{CookieSource, Manager, ManagerConfig, Update};
use omnidm_store::{DownloadId, Store};
use omnidm_testserver::{FileSpec, TestServer};
use tokio::sync::broadcast;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

struct Env {
    server: TestServer,
    manager: Manager,
    data: tempfile::TempDir,
    downloads: tempfile::TempDir,
}

fn fast_engine() -> EngineConfig {
    EngineConfig {
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
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(50),
        },
        ..EngineConfig::default()
    }
}

async fn env(max_active: usize) -> Env {
    let server = TestServer::start().await.unwrap();
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: downloads.path().to_path_buf(),
        max_active,
        engine: fast_engine(),
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();
    Env {
        server,
        manager,
        data,
        downloads,
    }
}

impl Env {
    fn store(&self) -> Store {
        Store::open(&self.data.path().join("omnidm.db")).unwrap()
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.downloads.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn capture(&self, path: &str) -> Capture {
        Capture::new(self.server.url(path), 1)
    }

    fn add_request(&self, path: &str) -> AddRequest {
        AddRequest {
            url: self.server.url(path),
            dir: self.downloads.path().to_string_lossy().into_owned(),
            file_name: None,
            connections: None,
            headers: vec![],
            cookies: vec![],
            data: None,
            queue: None,
        }
    }
}

fn cookie(name: &str, value: &str) -> BrowserCookie {
    BrowserCookie {
        name: name.into(),
        value: value.into(),
        domain: "127.0.0.1".into(),
        host_only: true,
        path: "/".into(),
        secure: false,
        http_only: true,
        expiration_date: None,
    }
}

async fn finished(updates: &mut broadcast::Receiver<Update>, id: i64) -> Finished {
    let wait = async {
        loop {
            match updates.recv().await {
                Ok(Update::Finished(done)) if done.download_id == id => return done,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("updates ended: {e}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .expect("the download did not finish in time")
}

fn accepted(result: &CaptureResult) -> i64 {
    match result {
        CaptureResult::Accepted { download_id } => *download_id,
        other => panic!("not accepted: {other:?}"),
    }
}

// --- captures from the browser ---

#[tokio::test(flavor = "multi_thread")]
async fn a_capture_is_probed_then_downloaded_with_its_cookies_and_referrer() {
    let env = env(3).await;
    let spec = FileSpec::new(2 * MIB)
        .etag("v1")
        .require_cookie("session=abc")
        .content_disposition("attachment; filename=\"report.pdf\"");
    env.server.set_file("/get", spec.clone());
    let mut updates = env.manager.subscribe();

    let result = env
        .manager
        .capture(Capture {
            referrer: Some("https://example.com/page".into()),
            cookies: vec![cookie("session", "abc")],
            ..env.capture("/get")
        })
        .await;

    let id = accepted(&result);
    let done = finished(&mut updates, id).await;
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    let path = PathBuf::from(done.path.unwrap());
    assert_eq!(path, env.downloads.path().join("report.pdf"));
    assert!(std::fs::read(&path).unwrap() == spec.bytes(0, spec.size as usize));
    for request in env.server.requests() {
        assert_eq!(request.header("referer"), Some("https://example.com/page"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capture_the_app_cannot_fetch_goes_back_to_the_browser() {
    let env = env(3).await;

    let result = env.manager.capture(env.capture("/missing.zip")).await;

    match result {
        CaptureResult::Rejected {
            reason: RejectReason::ProbeFailed,
            detail,
        } => assert!(detail.unwrap_or_default().contains("404")),
        other => panic!("expected ProbeFailed, got {other:?}"),
    }
    assert!(env.manager.list().is_empty(), "nothing is recorded");
    assert!(env.files().is_empty(), "nothing is written");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unusable_capture_is_rejected_as_invalid() {
    let env = env(3).await;
    for capture in [
        Capture::new("file:///C:/Windows/win.ini", 1),
        Capture::new("ftp://example.com/a.zip", 1),
        Capture::new("not a url", 1),
        Capture {
            method: Some("DELETE".into()),
            ..env.capture("/a")
        },
        Capture {
            method: Some("POST".into()),
            body: Some("%%% not base64".into()),
            ..env.capture("/a")
        },
    ] {
        let result = env.manager.capture(capture.clone()).await;
        assert!(
            matches!(
                result,
                CaptureResult::Rejected {
                    reason: RejectReason::Invalid,
                    ..
                }
            ),
            "{capture:?} → {result:?}"
        );
    }
    assert!(env.server.requests().is_empty(), "nothing was fetched");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_captured_form_is_posted() {
    let env = env(3).await;
    let spec = FileSpec::new(MIB).etag("v1").post_only("id=7");
    env.server.set_file("/dl", spec.clone());
    let mut updates = env.manager.subscribe();

    let result = env
        .manager
        .capture(Capture {
            method: Some("POST".into()),
            body: Some(base64::engine::general_purpose::STANDARD.encode("id=7")),
            headers: vec![Header {
                name: "Content-Type".into(),
                value: "application/x-www-form-urlencoded".into(),
            }],
            suggested_name: Some("form-result.bin".into()),
            ..env.capture("/dl")
        })
        .await;

    let done = finished(&mut updates, accepted(&result)).await;
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    assert_eq!(env.files(), ["form-result.bin"]);
}

// --- saved cookies ---

#[tokio::test(flavor = "multi_thread")]
async fn cookies_are_kept_encrypted_while_unfinished_and_dropped_when_done() {
    let env = env(3).await;
    let spec = FileSpec::new(2 * MIB)
        .etag("v1")
        .require_cookie("session=abc");
    env.server.set_file("/f.bin", spec.clone().throttle(MIB));
    let mut updates = env.manager.subscribe();

    let id = accepted(
        &env.manager
            .capture(Capture {
                cookies: vec![cookie("session", "abc")],
                ..env.capture("/f.bin")
            })
            .await,
    );
    assert!(env.manager.pause(id));
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Paused
    );
    assert!(env.store().load_cookies(DownloadId(id)).unwrap().is_some());

    env.server.set_file("/f.bin", spec);
    env.manager.resume(id).await.unwrap();
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
    assert_eq!(env.store().load_cookies(DownloadId(id)).unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn cookies_from_a_private_window_are_never_saved() {
    let env = env(3).await;
    env.server
        .set_file("/f.bin", FileSpec::new(2 * MIB).etag("v1").throttle(MIB));
    let mut updates = env.manager.subscribe();

    let id = accepted(
        &env.manager
            .capture(Capture {
                cookies: vec![cookie("session", "abc")],
                private: true,
                ..env.capture("/f.bin")
            })
            .await,
    );
    env.manager.pause(id);
    finished(&mut updates, id).await;

    assert_eq!(env.store().load_cookies(DownloadId(id)).unwrap(), None);
}

// --- pause and resume ---

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_download_continues_where_it_stopped() {
    let env = env(3).await;
    let spec = FileSpec::new(4 * MIB).etag("v1");
    env.server
        .set_file("/f.bin", spec.clone().throttle(2 * MIB));
    let mut updates = env.manager.subscribe();
    let id = env.manager.add(env.add_request("/f.bin")).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    env.manager.pause(id);
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Paused
    );
    let before = env.server.requests().len();
    env.server.set_file("/f.bin", spec.clone());
    env.manager.resume(id).await.unwrap();

    let done = finished(&mut updates, id).await;
    assert_eq!(done.status, FinishStatus::Completed);
    assert!(std::fs::read(done.path.unwrap()).unwrap() == spec.bytes(0, spec.size as usize));
    let restarted = env.server.requests()[before..]
        .iter()
        .any(|r| r.range.as_deref() == Some("bytes=0-"));
    assert!(!restarted, "continued instead of starting over");
}

struct FreshCookies(Vec<BrowserCookie>);

impl CookieSource for FreshCookies {
    fn cookies_for(
        &self,
        _url: String,
    ) -> Pin<Box<dyn Future<Output = Option<Vec<BrowserCookie>>> + Send>> {
        let cookies = self.0.clone();
        Box::pin(async move { Some(cookies) })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resuming_asks_the_browser_for_fresh_cookies() {
    let env = env(3).await;
    let spec = FileSpec::new(2 * MIB).etag("v1");
    env.server.set_file(
        "/f.bin",
        spec.clone().throttle(MIB).require_cookie("session=old"),
    );
    let mut updates = env.manager.subscribe();
    let id = accepted(
        &env.manager
            .capture(Capture {
                cookies: vec![cookie("session", "old")],
                ..env.capture("/f.bin")
            })
            .await,
    );
    env.manager.pause(id);
    finished(&mut updates, id).await;

    // The site moved on to a new session; only the browser knows it.
    env.server
        .set_file("/f.bin", spec.clone().require_cookie("session=new"));
    env.manager
        .set_cookie_source(Some(Arc::new(FreshCookies(vec![cookie("session", "new")]))));
    env.manager.resume(id).await.unwrap();

    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
}

// --- added downloads and the waiting line ---

#[tokio::test(flavor = "multi_thread")]
async fn added_downloads_beyond_the_limit_wait_their_turn() {
    let env = env(1).await;
    env.server
        .set_file("/a.bin", FileSpec::new(MIB).etag("a").throttle(2 * MIB));
    env.server
        .set_file("/b.bin", FileSpec::new(MIB).etag("b").throttle(2 * MIB));
    let mut updates = env.manager.subscribe();

    let a = env.manager.add(env.add_request("/a.bin")).unwrap();
    let b = env.manager.add(env.add_request("/b.bin")).unwrap();

    let status = |id: i64| {
        env.manager
            .list()
            .into_iter()
            .find(|d| d.download_id == id)
            .unwrap()
            .status
    };
    assert_eq!(status(b), "queued", "only one runs at a time");
    assert_eq!(
        finished(&mut updates, a).await.status,
        FinishStatus::Completed
    );
    assert_eq!(
        finished(&mut updates, b).await.status,
        FinishStatus::Completed
    );
    assert_eq!(env.files(), ["a.bin", "b.bin"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_is_broadcast_while_downloading() {
    let env = env(3).await;
    env.server.set_file(
        "/f.bin",
        FileSpec::new(2 * MIB).etag("v1").throttle(4 * MIB),
    );
    let mut updates = env.manager.subscribe();

    let id = env.manager.add(env.add_request("/f.bin")).unwrap();

    let mut saw_progress = false;
    loop {
        match tokio::time::timeout(Duration::from_secs(20), updates.recv()).await {
            Ok(Ok(Update::Progress(p))) if p.download_id == id => saw_progress = true,
            Ok(Ok(Update::Finished(f))) if f.download_id == id => break,
            Ok(Ok(_)) => {}
            other => panic!("{other:?}"),
        }
    }
    assert!(saw_progress);
    let listed = env.manager.list();
    assert_eq!(listed[0].status, "completed");
    assert_eq!(listed[0].downloaded, 2 * MIB);
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_left_running_by_a_crash_are_paused_on_start() {
    let data = tempfile::tempdir().unwrap();
    {
        let mut store = Store::open(&data.path().join("omnidm.db")).unwrap();
        let id = store
            .insert_download(&omnidm_store::NewDownload {
                url: "https://e.com/a.zip".into(),
                save_dir: "dl".into(),
                file_name: None,
                referrer: None,
                user_agent: None,
                headers: vec![],
                method: omnidm_core::engine::Method::Get,
                body: vec![],
            })
            .unwrap();
        store
            .set_status(id, omnidm_store::Status::Running, None)
            .unwrap();
    }

    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: data.path().to_path_buf(),
        max_active: 3,
        engine: fast_engine(),
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();

    assert_eq!(manager.list()[0].status, "paused");
}

#[tokio::test(flavor = "multi_thread")]
async fn cookies_given_to_an_added_download_survive_a_pause() {
    let env = env(3).await;
    let spec = FileSpec::new(2 * MIB)
        .etag("v1")
        .require_cookie("session=abc");
    env.server.set_file("/f.bin", spec.clone().throttle(MIB));
    let mut updates = env.manager.subscribe();
    let id = env
        .manager
        .add(AddRequest {
            cookies: vec!["session=abc".into()],
            ..env.add_request("/f.bin")
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    env.manager.pause(id);
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Paused
    );

    env.server.set_file("/f.bin", spec);
    env.manager.resume(id).await.unwrap();

    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
}
