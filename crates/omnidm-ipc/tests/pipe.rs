// SPDX-License-Identifier: GPL-3.0-or-later

//! The named pipe between the app and its clients (host, CLI).
#![cfg(windows)]

use std::path::PathBuf;

use omnidm_ipc::frame::{read_frame_async, write_frame_async};
use omnidm_ipc::pipe::{PipeError, PipeListener, connect, current_user_sddl, default_pipe_name};

fn unique_name() -> String {
    format!(r"\\.\pipe\OmniDm.test.{:016x}", fastrand::u64(..))
}

/// Accepts one client and answers each frame with `echo:` + the frame.
async fn echo_once(listener: &mut PipeListener) {
    let mut pipe = listener.accept().await.unwrap();
    while let Some(frame) = read_frame_async(&mut pipe, 1024).await.unwrap() {
        let reply = [b"echo:".as_slice(), &frame].concat();
        write_frame_async(&mut pipe, &reply).await.unwrap();
    }
}

#[tokio::test]
async fn frames_travel_both_ways() {
    let name = unique_name();
    let mut listener = PipeListener::bind(&name).unwrap();
    let server = tokio::spawn(async move { echo_once(&mut listener).await });

    let mut client = connect(&name, &[]).await.unwrap();
    write_frame_async(&mut client, b"hello").await.unwrap();
    let reply = read_frame_async(&mut client, 1024).await.unwrap();

    assert_eq!(reply.as_deref(), Some(&b"echo:hello"[..]));
    drop(client);
    server.await.unwrap();
}

#[tokio::test]
async fn clients_can_connect_at_the_same_time() {
    let name = unique_name();
    let mut listener = PipeListener::bind(&name).unwrap();
    tokio::spawn(async move {
        loop {
            let mut pipe = listener.accept().await.unwrap();
            tokio::spawn(async move {
                while let Some(frame) = read_frame_async(&mut pipe, 1024).await.unwrap() {
                    write_frame_async(&mut pipe, &frame).await.unwrap();
                }
            });
        }
    });

    let mut a = connect(&name, &[]).await.unwrap();
    let mut b = connect(&name, &[]).await.unwrap();
    write_frame_async(&mut a, b"from a").await.unwrap();
    write_frame_async(&mut b, b"from b").await.unwrap();

    assert_eq!(
        read_frame_async(&mut b, 1024).await.unwrap().as_deref(),
        Some(&b"from b"[..])
    );
    assert_eq!(
        read_frame_async(&mut a, 1024).await.unwrap().as_deref(),
        Some(&b"from a"[..])
    );
}

#[tokio::test]
async fn a_second_server_learns_one_is_already_running() {
    let name = unique_name();
    let _first = PipeListener::bind(&name).unwrap();

    match PipeListener::bind(&name) {
        Err(PipeError::AlreadyRunning) => {}
        other => panic!("expected AlreadyRunning, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn connecting_without_a_server_says_it_is_not_running() {
    match connect(&unique_name(), &[]).await {
        Err(PipeError::NotRunning) => {}
        other => panic!("expected NotRunning, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn the_server_program_is_checked() {
    let name = unique_name();
    let mut listener = PipeListener::bind(&name).unwrap();
    tokio::spawn(async move {
        loop {
            let _pipe = listener.accept().await.unwrap();
        }
    });
    // The server runs inside this test program.
    let me = std::env::current_exe().unwrap();

    let elsewhere = PathBuf::from(r"C:\Program Files\OmniDm\OmniDm.exe");

    assert!(connect(&name, std::slice::from_ref(&me)).await.is_ok());
    // Any one of several allowed programs will do.
    assert!(
        connect(&name, &[elsewhere.clone(), me.clone()])
            .await
            .is_ok()
    );
    match connect(&name, &[elsewhere]).await {
        Err(PipeError::Impostor { found }) => assert_eq!(found, me),
        other => panic!("expected Impostor, got {:?}", other.err()),
    }
}

#[test]
fn only_the_current_user_may_open_the_pipe() {
    let sddl = current_user_sddl().unwrap();
    // A protected DACL with one entry: full access for the current user's SID.
    assert!(sddl.starts_with("D:P(A;;GA;;;S-1-5-"), "{sddl}");
    assert_eq!(sddl.matches("(A;").count(), 1, "{sddl}");
}

#[test]
fn the_default_name_includes_the_windows_session() {
    let name = default_pipe_name();
    assert!(name.starts_with(r"\\.\pipe\OmniDm."), "{name}");
    assert!(
        name.rsplit('.')
            .next()
            .unwrap()
            .chars()
            .all(|c| c.is_ascii_digit()),
        "{name}"
    );
}
