// SPDX-License-Identifier: GPL-3.0-or-later

//! Crash test: the real `omnidm` binary is force-killed at random moments while
//! downloading, then restarted, many times. After every kill, each byte the database
//! counts as saved must really be on disk, and progress must never go backwards. The
//! final file must match the served file byte for byte.
//!
//! Slow, so it is ignored by default:
//!
//! ```text
//! cargo test -p omnidm-cli --test crash -- --ignored --nocapture
//! ```
//!
//! Settings (environment variables): `OMNIDM_CRASH_MB` file size (default 128),
//! `OMNIDM_CRASH_KILLS` (default 20), `OMNIDM_CRASH_KIBPS` speed per connection in KiB/s
//! (default 256), `OMNIDM_CRASH_SEED` to replay the same kill timings.
//!
//! A process kill keeps data already handed to Windows, unlike a power cut; this test
//! checks OmniDM's own ordering of writes and checkpoints, not the disk's.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::Stdio;
use std::time::{Duration, Instant};

use omnidm_store::{Status, Store};
use omnidm_testserver::{FileSpec, TestServer};
use tokio::process::Command;

const MIB: u64 = 1024 * 1024;
const CHUNK: u64 = 4 * MIB;

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Checks that `[start, start + len)` of `file` holds the served content.
fn assert_range_on_disk(file: &mut File, spec: &FileSpec, start: u64, len: u64, context: &str) {
    let mut offset = start;
    let end = start + len;
    let mut buf = Vec::new();
    while offset < end {
        let n = CHUNK.min(end - offset);
        buf.resize(n as usize, 0);
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.read_exact(&mut buf).unwrap();
        assert!(
            buf == spec.bytes(offset, n as usize),
            "{context}: bytes {offset}..{} on disk differ from the served file",
            offset + n
        );
        offset += n;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "slow: run with --ignored"]
async fn downloads_survive_being_killed_again_and_again() {
    let size = setting("OMNIDM_CRASH_MB", 128) * MIB;
    let kills = setting("OMNIDM_CRASH_KILLS", 20);
    let per_connection = setting("OMNIDM_CRASH_KIBPS", 256) * 1024;
    let seed = setting("OMNIDM_CRASH_SEED", fastrand::u64(..));
    let mut rng = fastrand::Rng::with_seed(seed);
    println!(
        "crash test: {} MiB, {kills} kills, {} KiB/s per connection, seed {seed}",
        size / MIB,
        per_connection / 1024
    );

    let spec = FileSpec::new(size).seed(7).etag("crash-v1");
    let server = TestServer::start().await.unwrap();
    server.set_file("/big.bin", spec.clone().throttle(per_connection));
    let downloads = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let part = downloads.path().join("big.bin.omnidm-part");
    let database = data.path().join("omnidm.db");

    let command = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_omnidm"));
        cmd.args(["get", &server.url("/big.bin"), "-c", "8", "-d"])
            .arg(downloads.path())
            .env("OMNIDM_DATA_DIR", data.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        cmd
    };

    let started = Instant::now();
    let mut saved_before = 0;
    let mut kills_with_progress = 0;
    for kill in 1..=kills {
        let mut child = command().spawn().unwrap();
        let delay = Duration::from_millis(rng.u64(500..3500));
        tokio::select! {
            status = child.wait() => panic!(
                "the download finished by itself after {} kills ({status:?}); \
                 use a bigger OMNIDM_CRASH_MB or a lower OMNIDM_CRASH_KIBPS",
                kill - 1
            ),
            _ = tokio::time::sleep(delay) => child.kill().await.unwrap(),
        }

        // What the database says survived the kill must really be on disk.
        let store = Store::open(&database).unwrap();
        let Some(record) = store.list_downloads().unwrap().into_iter().next() else {
            println!("kill {kill:>2} after {delay:?}: killed before the download was recorded");
            continue;
        };
        assert_eq!(
            record.status,
            Status::Running,
            "a killed run can't record its end"
        );
        let segments = store.load_segments(record.id).unwrap();
        let saved: u64 = segments.iter().map(|s| s.written).sum();
        assert!(
            saved >= saved_before,
            "kill {kill}: saved progress went backwards from {saved_before} to {saved}"
        );
        if saved > 0 {
            let mut file = File::open(&part).unwrap();
            for s in &segments {
                assert_range_on_disk(
                    &mut file,
                    &spec,
                    s.start,
                    s.written,
                    &format!("kill {kill}"),
                );
            }
        }
        if saved > saved_before {
            kills_with_progress += 1;
        }
        println!(
            "kill {kill:>2} after {delay:>6.1?}: {:>5.1} MiB saved in {} segments, verified",
            saved as f64 / MIB as f64,
            segments.len()
        );
        saved_before = saved;
    }
    assert!(
        kills_with_progress >= kills / 4,
        "only {kills_with_progress} of {kills} kills happened after new progress was saved; \
         the test is not exercising resume"
    );

    // Last run: no throttle, no kill. It must resume and finish.
    server.set_file("/big.bin", spec.clone());
    let status = tokio::time::timeout(Duration::from_secs(600), command().status())
        .await
        .expect("the final run did not finish within 10 minutes")
        .unwrap();
    assert!(status.success(), "final run failed: {status:?}");

    let finished = downloads.path().join("big.bin");
    assert!(!part.exists(), "the part file must be renamed");
    let mut file = File::open(&finished).unwrap();
    assert_eq!(file.metadata().unwrap().len(), size);
    assert_range_on_disk(&mut file, &spec, 0, size, "final file");
    let store = Store::open(&database).unwrap();
    assert_eq!(store.list_downloads().unwrap()[0].status, Status::Completed);
    println!(
        "finished: {} MiB byte-identical after {kills} kills ({kills_with_progress} with new progress), {:.0?} in total",
        size / MIB,
        started.elapsed()
    );
}
