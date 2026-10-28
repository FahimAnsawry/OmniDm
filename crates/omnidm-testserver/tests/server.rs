// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, Instant, SystemTime};

use omnidm_testserver::{FileSpec, TestServer, fill};
use reqwest::{StatusCode, header};

const SIZE: u64 = 100_000;

async fn server_with(path: &str, spec: FileSpec) -> TestServer {
    let server = TestServer::start().await.unwrap();
    server.set_file(path, spec);
    server
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn header_str(resp: &reqwest::Response, name: header::HeaderName) -> Option<String> {
    resp.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

// --- content generator ---

#[test]
fn generated_content_is_deterministic_and_depends_on_the_seed() {
    let a = FileSpec::new(SIZE).seed(1).bytes(0, 4096);
    let b = FileSpec::new(SIZE).seed(1).bytes(0, 4096);
    let c = FileSpec::new(SIZE).seed(2).bytes(0, 4096);
    assert_eq!(a, b);
    assert_ne!(a, c);
    // Not a constant fill: at least 200 distinct byte values in 4 KiB.
    let mut seen = [false; 256];
    a.iter().for_each(|&x| seen[x as usize] = true);
    assert!(seen.iter().filter(|&&s| s).count() > 200);
}

#[test]
fn generated_content_is_the_same_whatever_the_start_offset() {
    let whole = FileSpec::new(SIZE).bytes(0, 1000);
    for start in [0usize, 1, 7, 8, 9, 500, 993] {
        let mut part = vec![0; 1000 - start];
        fill(1, start as u64, &mut part);
        assert_eq!(part, whole[start..], "offset {start}");
    }
}

// --- normal behaviour ---

#[tokio::test]
async fn full_get_returns_the_whole_file_with_its_length() {
    let server = server_with("/f.bin", FileSpec::new(SIZE)).await;
    let resp = client().get(server.url("/f.bin")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        header_str(&resp, header::CONTENT_LENGTH).as_deref(),
        Some("100000")
    );
    assert_eq!(
        header_str(&resp, header::ACCEPT_RANGES).as_deref(),
        Some("bytes")
    );
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.as_ref(), FileSpec::new(SIZE).bytes(0, SIZE as usize));
}

#[tokio::test]
async fn bounded_range_returns_206_with_that_slice() {
    let server = server_with("/f.bin", FileSpec::new(SIZE)).await;
    let resp = client()
        .get(server.url("/f.bin"))
        .header(header::RANGE, "bytes=1000-1999")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        header_str(&resp, header::CONTENT_RANGE).as_deref(),
        Some("bytes 1000-1999/100000")
    );
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.as_ref(), FileSpec::new(SIZE).bytes(1000, 1000));
}

#[tokio::test]
async fn open_ended_and_suffix_ranges_are_supported() {
    let server = server_with("/f.bin", FileSpec::new(SIZE)).await;
    for (range, want) in [
        ("bytes=99000-", "bytes 99000-99999/100000"),
        ("bytes=-500", "bytes 99500-99999/100000"),
        ("bytes=99000-200000", "bytes 99000-99999/100000"),
    ] {
        let resp = client()
            .get(server.url("/f.bin"))
            .header(header::RANGE, range)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT, "{range}");
        assert_eq!(
            header_str(&resp, header::CONTENT_RANGE).as_deref(),
            Some(want)
        );
    }
}

#[tokio::test]
async fn range_starting_past_the_end_is_416() {
    let server = server_with("/f.bin", FileSpec::new(SIZE)).await;
    let resp = client()
        .get(server.url("/f.bin"))
        .header(header::RANGE, "bytes=100000-")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        header_str(&resp, header::CONTENT_RANGE).as_deref(),
        Some("bytes */100000")
    );
}

#[tokio::test]
async fn head_returns_headers_without_a_body() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).etag("v1")).await;
    let resp = client().head(server.url("/f.bin")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        header_str(&resp, header::CONTENT_LENGTH).as_deref(),
        Some("100000")
    );
    assert_eq!(header_str(&resp, header::ETAG).as_deref(), Some("\"v1\""));
    assert!(resp.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_paths_are_404() {
    let server = TestServer::start().await.unwrap();
    let resp = client().get(server.url("/missing")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// --- validators ---

#[tokio::test]
async fn if_range_with_the_current_etag_gets_the_range() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).etag("v1")).await;
    let resp = client()
        .get(server.url("/f.bin"))
        .header(header::RANGE, "bytes=10-19")
        .header(header::IF_RANGE, "\"v1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
}

#[tokio::test]
async fn if_range_with_an_old_etag_gets_the_whole_new_file() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).etag("v1")).await;
    server.set_file("/f.bin", FileSpec::new(SIZE).seed(9).etag("v2"));
    let resp = client()
        .get(server.url("/f.bin"))
        .header(header::RANGE, "bytes=10-19")
        .header(header::IF_RANGE, "\"v1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.bytes().await.unwrap();
    assert_eq!(
        body.as_ref(),
        FileSpec::new(SIZE).seed(9).bytes(0, SIZE as usize)
    );
}

#[tokio::test]
async fn if_range_with_the_last_modified_date_gets_the_range() {
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let server = server_with("/f.bin", FileSpec::new(SIZE).last_modified(modified)).await;
    let resp = client()
        .get(server.url("/f.bin"))
        .header(header::RANGE, "bytes=10-19")
        .header(header::IF_RANGE, "Tue, 14 Nov 2023 22:13:20 GMT")
        .send()
        .await
        .unwrap();
    assert_eq!(
        header_str(&resp, header::LAST_MODIFIED).as_deref(),
        Some("Tue, 14 Nov 2023 22:13:20 GMT")
    );
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
}

// --- faults ---

#[tokio::test]
async fn no_ranges_file_ignores_range_and_sends_everything() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).no_ranges()).await;
    let resp = client()
        .get(server.url("/f.bin"))
        .header(header::RANGE, "bytes=1000-1999")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(header_str(&resp, header::ACCEPT_RANGES), None);
    assert_eq!(resp.bytes().await.unwrap().len(), SIZE as usize);
}

#[tokio::test]
async fn unknown_length_file_has_no_content_length_but_full_body() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).unknown_length()).await;
    let resp = client().get(server.url("/f.bin")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(header_str(&resp, header::CONTENT_LENGTH), None);
    assert_eq!(resp.bytes().await.unwrap().len(), SIZE as usize);
}

#[tokio::test]
async fn failing_file_answers_the_status_the_given_number_of_times() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).fail(503, 2, Some(3))).await;
    for _ in 0..2 {
        let resp = client().get(server.url("/f.bin")).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(header_str(&resp, header::RETRY_AFTER).as_deref(), Some("3"));
    }
    let resp = client().get(server.url("/f.bin")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn dropping_file_cuts_the_body_short_then_recovers() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).drop_after(5000, 1)).await;
    let first = client().get(server.url("/f.bin")).send().await.unwrap();
    assert!(
        first.bytes().await.is_err(),
        "first body should be cut short"
    );
    let second = client().get(server.url("/f.bin")).send().await.unwrap();
    assert_eq!(second.bytes().await.unwrap().len(), SIZE as usize);
}

#[tokio::test]
async fn stalling_file_goes_silent_then_recovers() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).stall_after(5000, 1)).await;
    let first = client().get(server.url("/f.bin")).send().await.unwrap();
    let read = tokio::time::timeout(Duration::from_millis(500), first.bytes()).await;
    assert!(read.is_err(), "the first body should never finish");
    let second = client().get(server.url("/f.bin")).send().await.unwrap();
    assert_eq!(second.bytes().await.unwrap().len(), SIZE as usize);
}

#[tokio::test]
async fn throttled_file_is_sent_no_faster_than_the_limit() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).throttle(200_000)).await;
    let started = Instant::now();
    let body = client()
        .get(server.url("/f.bin"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(body.len(), SIZE as usize);
    // 100 KB at 200 KB/s takes at least half a second.
    assert!(
        started.elapsed() >= Duration::from_millis(450),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn html_instead_returns_a_web_page() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).html_instead()).await;
    let resp = client().get(server.url("/f.bin")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        header_str(&resp, header::CONTENT_TYPE)
            .unwrap()
            .starts_with("text/html")
    );
    let body = resp.text().await.unwrap();
    assert!(body.to_ascii_lowercase().starts_with("<!doctype html"));
}

#[tokio::test]
async fn redirects_point_to_the_target() {
    let server = server_with("/real.bin", FileSpec::new(SIZE)).await;
    server.redirect("/start", "/middle", 301);
    server.redirect("/middle", "/real.bin", 302);
    let resp = client().get(server.url("/start")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        header_str(&resp, header::LOCATION).as_deref(),
        Some("/middle")
    );
    let followed = reqwest::get(server.url("/start")).await.unwrap();
    assert_eq!(followed.status(), StatusCode::OK);
    assert_eq!(followed.url().path(), "/real.bin");
}

#[tokio::test]
async fn requests_are_logged_with_their_range_headers() {
    let server = server_with("/f.bin", FileSpec::new(SIZE).etag("v1")).await;
    client()
        .get(server.url("/f.bin"))
        .header(header::RANGE, "bytes=5-9")
        .header(header::IF_RANGE, "\"v1\"")
        .send()
        .await
        .unwrap();
    let log = server.requests();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].method, "GET");
    assert_eq!(log[0].path, "/f.bin");
    assert_eq!(log[0].range.as_deref(), Some("bytes=5-9"));
    assert_eq!(log[0].if_range.as_deref(), Some("\"v1\""));
}
