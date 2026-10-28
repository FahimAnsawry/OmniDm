// SPDX-License-Identifier: GPL-3.0-or-later

//! Framing: a 4-byte little-endian length, then that many bytes of UTF-8 JSON, the same
//! as browser native messaging.

use std::io::Cursor;

use omnidm_ipc::frame::{FrameError, read_frame, read_frame_async, write_frame, write_frame_async};

#[test]
fn a_frame_is_a_little_endian_length_then_the_bytes() {
    let mut out = Vec::new();
    write_frame(&mut out, br#"{"a":1}"#).unwrap();
    assert_eq!(out, [&[7, 0, 0, 0][..], br#"{"a":1}"#].concat());
}

#[test]
fn frames_are_read_back_one_by_one() {
    let mut stream = Vec::new();
    write_frame(&mut stream, b"first").unwrap();
    write_frame(&mut stream, b"").unwrap();
    write_frame(&mut stream, b"third").unwrap();
    let mut reader = Cursor::new(stream);
    assert_eq!(
        read_frame(&mut reader, 100).unwrap().as_deref(),
        Some(&b"first"[..])
    );
    assert_eq!(
        read_frame(&mut reader, 100).unwrap().as_deref(),
        Some(&b""[..])
    );
    assert_eq!(
        read_frame(&mut reader, 100).unwrap().as_deref(),
        Some(&b"third"[..])
    );
    assert_eq!(read_frame(&mut reader, 100).unwrap(), None, "a clean end");
}

#[test]
fn a_frame_over_the_limit_is_refused_without_reading_it() {
    let mut reader = Cursor::new([&5_000_000u32.to_le_bytes()[..], b"x"].concat());
    match read_frame(&mut reader, 4 * 1024 * 1024) {
        Err(FrameError::TooLarge { size, limit }) => {
            assert_eq!((size, limit), (5_000_000, 4 * 1024 * 1024));
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn a_frame_cut_short_is_an_error() {
    for bytes in [&[5u8, 0][..], &[5, 0, 0, 0, b'a', b'b'][..]] {
        let mut reader = Cursor::new(bytes.to_vec());
        assert!(
            matches!(read_frame(&mut reader, 100), Err(FrameError::Io(_))),
            "{bytes:?}"
        );
    }
}

#[test]
fn writing_more_than_the_length_field_can_hold_is_refused() {
    struct Sink;
    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // Not allocated: a zero-length slice pretending to be huge is impossible, so check
    // the limit function directly.
    assert!(omnidm_ipc::frame::check_length(u32::MAX as usize + 1).is_err());
    assert!(omnidm_ipc::frame::check_length(10).is_ok());
    write_frame(&mut Sink, b"fine").unwrap();
}

#[tokio::test]
async fn async_frames_match_the_sync_format() {
    let mut out = Vec::new();
    write_frame_async(&mut out, b"hello").await.unwrap();
    let mut sync_out = Vec::new();
    write_frame(&mut sync_out, b"hello").unwrap();
    assert_eq!(out, sync_out);

    let mut reader = &out[..];
    assert_eq!(
        read_frame_async(&mut reader, 100).await.unwrap().as_deref(),
        Some(&b"hello"[..])
    );
    assert_eq!(read_frame_async(&mut reader, 100).await.unwrap(), None);
}
