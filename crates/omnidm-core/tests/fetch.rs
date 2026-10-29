// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetching small files and parts of files over one connection.

use std::time::Duration;

use omnidm_core::engine::{EngineConfig, EngineError, Method, Request, RetryPolicy};
use omnidm_core::fetch::{Sink, fetch};
use omnidm_net::cookies::CookieJar;
use omnidm_testserver::{FileSpec, TestServer};
use tokio_util::sync::CancellationToken;

fn config() -> EngineConfig {
    EngineConfig {
        retry: RetryPolicy {
            max_attempts: 4,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(50),
        },
        stall_timeout: Duration::from_millis(500),
        ..EngineConfig::default()
    }
}

fn request(server: &TestServer) -> Request {
    Request {
        url: server.url("/page"),
        method: Method::Get,
        body: Vec::new(),
        headers: vec![("Referer".into(), server.url("/page"))],
        cookies: CookieJar::new(),
    }
}

async fn get(
    server: &TestServer,
    path: &str,
    range: Option<(u64, u64)>,
    sink: &mut impl Sink,
) -> Result<Option<omnidm_core::fetch::Fetched>, EngineError> {
    let client = omnidm_net::download_client().unwrap();
    fetch(
        &client,
        &request(server),
        &server.url(path),
        range,
        &config(),
        &CancellationToken::new(),
        sink,
        |_| {},
    )
    .await
}

#[tokio::test]
async fn whole_files_and_ranges() {
    let server = TestServer::start().await.unwrap();
    let spec = FileSpec::new(100_000).content_type("video/mp2t");
    server.set_file("/a.ts", spec.clone());

    let mut whole = Vec::new();
    let fetched = get(&server, "/a.ts", None, &mut whole)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(whole, spec.bytes(0, 100_000));
    assert_eq!(fetched.size, 100_000);
    assert_eq!(fetched.content_type.as_deref(), Some("video/mp2t"));
    assert_eq!(fetched.final_url, server.url("/a.ts"));

    let mut part = Vec::new();
    get(&server, "/a.ts", Some((1000, 1999)), &mut part)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(part, spec.bytes(1000, 1000));

    // The captured page's headers go along.
    let sent = server.requests();
    assert!(
        sent.iter()
            .all(|r| r.header("referer") == Some(server.url("/page").as_str()))
    );
    assert_eq!(
        sent.last().unwrap().range.as_deref(),
        Some("bytes=1000-1999")
    );
}

#[tokio::test]
async fn a_server_without_ranges_still_gives_the_right_part() {
    let server = TestServer::start().await.unwrap();
    let spec = FileSpec::new(50_000).no_ranges();
    server.set_file("/all.ts", spec.clone());
    let mut part = Vec::new();
    get(&server, "/all.ts", Some((40_000, 40_999)), &mut part)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(part, spec.bytes(40_000, 1000));
}

#[tokio::test]
async fn continues_from_what_is_held() {
    let server = TestServer::start().await.unwrap();
    let spec = FileSpec::new(80_000);
    server.set_file("/big.mp4", spec.clone());

    let mut held = spec.bytes(0, 30_000);
    get(&server, "/big.mp4", None, &mut held)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held, spec.bytes(0, 80_000));
    assert_eq!(server.requests()[0].range.as_deref(), Some("bytes=30000-"));

    // Held bytes of a range count too; a finished range needs no request.
    let mut part = spec.bytes(100, 50);
    get(&server, "/big.mp4", Some((100, 199)), &mut part)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(part, spec.bytes(100, 100));
    assert_eq!(server.requests()[1].range.as_deref(), Some("bytes=150-199"));
    let before = server.requests().len();
    get(&server, "/big.mp4", Some((100, 199)), &mut part)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(server.requests().len(), before);
}

#[tokio::test]
async fn dropped_connections_continue_where_they_stopped() {
    let server = TestServer::start().await.unwrap();
    let spec = FileSpec::new(200_000).drop_after(64 * 1024, 2);
    server.set_file("/flaky.ts", spec.clone());
    let mut data = Vec::new();
    get(&server, "/flaky.ts", None, &mut data)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(data, spec.bytes(0, 200_000));
    let ranges: Vec<_> = server.requests().iter().map(|r| r.range.clone()).collect();
    assert_eq!(ranges[0], None);
    assert!(
        ranges[1]
            .as_deref()
            .is_some_and(|r| r.starts_with("bytes=") && r != "bytes=0-")
    );
}

#[tokio::test]
async fn files_on_disk() {
    let server = TestServer::start().await.unwrap();
    let spec = FileSpec::new(70_000);
    server.set_file("/piece.m4s", spec.clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("piece.part");
    std::fs::write(&path, spec.bytes(0, 10_000)).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut heard = 0;
    fetch(
        &omnidm_net::download_client().unwrap(),
        &request(&server),
        &server.url("/piece.m4s"),
        None,
        &config(),
        &CancellationToken::new(),
        &mut file,
        |n| heard += n,
    )
    .await
    .unwrap()
    .unwrap();
    drop(file);
    assert_eq!(std::fs::read(&path).unwrap(), spec.bytes(0, 70_000));
    assert_eq!(heard, 60_000, "only new bytes are reported");
}

#[tokio::test]
async fn errors_that_retrying_wont_fix() {
    let server = TestServer::start().await.unwrap();
    assert!(matches!(
        get(&server, "/missing", None, &mut Vec::new()).await,
        Err(EngineError::Status(404))
    ));
    server.set_file("/login", FileSpec::new(100).html_instead());
    assert!(matches!(
        get(&server, "/login", None, &mut Vec::new()).await,
        Err(EngineError::HtmlInsteadOfFile)
    ));
    // Temporary ones are retried.
    let spec = FileSpec::new(1000).fail(503, 2, None);
    server.set_file("/busy", spec.clone());
    let mut data = Vec::new();
    get(&server, "/busy", None, &mut data)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(data, spec.bytes(0, 1000));
}

#[tokio::test]
async fn cancelling_keeps_what_arrived() {
    let server = TestServer::start().await.unwrap();
    server.set_file("/slow", FileSpec::new(1_000_000).throttle(100_000));
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop.cancel();
    });
    let mut data = Vec::new();
    let result = fetch(
        &omnidm_net::download_client().unwrap(),
        &request(&server),
        &server.url("/slow"),
        None,
        &config(),
        &cancel,
        &mut data,
        |_| {},
    )
    .await
    .unwrap();
    assert!(result.is_none());
    assert!(!data.is_empty() && data.len() < 1_000_000);
}
