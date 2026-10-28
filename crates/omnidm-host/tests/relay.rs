// SPDX-License-Identifier: GPL-3.0-or-later

//! The host relays frames between the browser and the app without reading them, except
//! to enforce the size limits.

use std::time::Duration;

use omnidm_host::relay;
use omnidm_ipc::frame::{read_frame_async, write_frame_async};
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

const MIB: usize = 1024 * 1024;

/// The browser's and the app's ends of a running relay.
struct Ends {
    browser_in: WriteHalf<DuplexStream>,
    browser_out: ReadHalf<DuplexStream>,
    app: DuplexStream,
    relay: tokio::task::JoinHandle<std::io::Result<()>>,
}

fn start() -> Ends {
    let (browser_in, host_stdin) = tokio::io::duplex(8 * MIB);
    let (host_stdout, browser_out) = tokio::io::duplex(8 * MIB);
    let (host_pipe, app) = tokio::io::duplex(8 * MIB);
    let relay = tokio::spawn(relay(host_stdin, host_stdout, host_pipe));
    let (_, browser_in) = tokio::io::split(browser_in);
    let (browser_out, _) = tokio::io::split(browser_out);
    Ends {
        browser_in,
        browser_out,
        app,
        relay,
    }
}

async fn read(r: &mut (impl tokio::io::AsyncRead + Unpin)) -> Option<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(5), read_frame_async(r, 8 * MIB as u32))
        .await
        .expect("nothing arrived")
        .unwrap()
}

#[tokio::test]
async fn frames_pass_both_ways_unchanged() {
    let mut ends = start();

    write_frame_async(&mut ends.browser_in, br#"{"type":"ping"}"#)
        .await
        .unwrap();
    assert_eq!(
        read(&mut ends.app).await.as_deref(),
        Some(&br#"{"type":"ping"}"#[..])
    );

    write_frame_async(&mut ends.app, br#"{"type":"pong"}"#)
        .await
        .unwrap();
    assert_eq!(
        read(&mut ends.browser_out).await.as_deref(),
        Some(&br#"{"type":"pong"}"#[..])
    );
}

#[tokio::test]
async fn a_frame_too_big_for_the_browser_is_dropped_and_the_rest_flow() {
    let mut ends = start();

    write_frame_async(&mut ends.app, &vec![b'x'; MIB + 1])
        .await
        .unwrap();
    write_frame_async(&mut ends.app, b"small").await.unwrap();

    assert_eq!(
        read(&mut ends.browser_out).await.as_deref(),
        Some(&b"small"[..])
    );
}

#[tokio::test]
async fn a_frame_over_the_limit_from_the_browser_ends_the_relay() {
    let mut ends = start();

    ends.browser_in
        .write_all(&(5 * MIB as u32).to_le_bytes())
        .await
        .unwrap();

    let result = tokio::time::timeout(Duration::from_secs(5), ends.relay)
        .await
        .expect("the relay should stop")
        .unwrap();
    assert!(result.is_err());
}

#[tokio::test]
async fn the_relay_ends_when_the_browser_goes_away() {
    let ends = start();
    drop(ends.browser_in);

    let result = tokio::time::timeout(Duration::from_secs(5), ends.relay)
        .await
        .expect("the relay should stop");
    assert!(result.unwrap().is_ok());
}

#[tokio::test]
async fn the_relay_ends_when_the_app_goes_away() {
    let ends = start();
    drop(ends.app);

    let result = tokio::time::timeout(Duration::from_secs(5), ends.relay)
        .await
        .expect("the relay should stop");
    assert!(result.unwrap().is_ok());
}
