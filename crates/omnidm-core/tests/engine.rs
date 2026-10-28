// SPDX-License-Identifier: GPL-3.0-or-later

//! Engine tests against the fault-injecting test server, with real files on disk.

use std::path::PathBuf;
use std::time::Duration;

use omnidm_core::engine::{
    Checkpointer, Download, EngineConfig, EngineError, Event, Outcome, RemoteInfo, Request, Resume,
    RetryPolicy, download,
};
use omnidm_core::{RampMode, RampPolicy, Segment, SegmentId, SplitPolicy};
use omnidm_testserver::{FileSpec, TestServer};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

#[derive(Default)]
struct Recorder {
    probed: Vec<RemoteInfo>,
    saves: Vec<Vec<Segment>>,
}

impl Checkpointer for Recorder {
    fn probed(&mut self, info: &RemoteInfo) -> Result<(), String> {
        self.probed.push(info.clone());
        Ok(())
    }
    fn save(&mut self, segments: &[Segment]) -> Result<(), String> {
        self.saves.push(segments.to_vec());
        Ok(())
    }
}

fn written(segments: &[Segment]) -> u64 {
    segments.iter().map(|s| s.written).sum()
}

fn fast_config(max_connections: u32) -> EngineConfig {
    EngineConfig {
        split: SplitPolicy {
            min_segment: 128 * KIB,
            align: 4 * KIB,
        },
        ramp: RampPolicy {
            max_connections,
            mode: RampMode::Doubling,
            cooldown: Duration::from_millis(100),
        },
        tick: Duration::from_millis(20),
        checkpoint_interval: Duration::from_millis(50),
        checkpoint_bytes: 64 * MIB,
        write_buffer: 64 * KIB as usize,
        flush_interval: Duration::from_millis(100),
        retry: RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(100),
        },
        stall_timeout: Duration::from_secs(5),
        speed_limit: None,
        allow_html: false,
        restart_if_changed: true,
    }
}

struct Run {
    result: Result<Outcome, EngineError>,
    recorder: Recorder,
    events: Vec<Event>,
    part: PathBuf,
    dir: tempfile::TempDir,
}

async fn run(server: &TestServer, path: &str, config: EngineConfig) -> Run {
    run_in(
        tempfile::tempdir().unwrap(),
        server,
        path,
        config,
        None,
        None,
    )
    .await
}

/// Runs a download in `dir`, optionally resuming and optionally cancelling after a delay.
async fn run_in(
    dir: tempfile::TempDir,
    server: &TestServer,
    path: &str,
    config: EngineConfig,
    resume: Option<Resume>,
    cancel_after: Option<Duration>,
) -> Run {
    let part = dir.path().join("file.omnidm-part");
    let cancel = CancellationToken::new();
    if let Some(after) = cancel_after {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            cancel.cancel();
        });
    }
    let client = omnidm_net::download_client().unwrap();
    let request = Request {
        url: server.url(path),
        headers: vec![],
    };
    let mut recorder = Recorder::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        download(
            Download {
                client: &client,
                request: &request,
                part_path: &part,
                resume,
                config: &config,
                events: Some(tx),
                cancel,
            },
            &mut recorder,
        ),
    )
    .await
    .expect("download did not finish within 30 s");
    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    Run {
        result,
        recorder,
        events,
        part,
        dir,
    }
}

fn resume_info(server: &TestServer, path: &str, total: u64, etag: Option<&str>) -> RemoteInfo {
    RemoteInfo {
        final_url: server.url(path),
        total: Some(total),
        etag: etag.map(|e| format!("\"{e}\"")),
        last_modified: None,
        resumable: true,
        content_type: Some("application/octet-stream".into()),
        content_disposition: None,
    }
}

/// Creates a part file as an earlier run would have left it: full size, with each
/// segment's written bytes in place and zeros elsewhere.
fn prepare_part(dir: &tempfile::TempDir, spec: &FileSpec, segments: &[Segment]) {
    let mut data = vec![0u8; spec.size as usize];
    for s in segments {
        let (from, n) = (s.start as usize, s.written as usize);
        data[from..from + n].copy_from_slice(&spec.bytes(s.start, n));
    }
    std::fs::write(dir.path().join("file.omnidm-part"), data).unwrap();
}

fn seg(id: u32, start: u64, end: u64, written: u64) -> Segment {
    Segment {
        id: SegmentId(id),
        start,
        end,
        written,
    }
}

fn get_requests_with_if_range(server: &TestServer) -> Vec<(Option<String>, Option<String>)> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .map(|r| (r.range, r.if_range))
        .collect()
}

/// Start offsets of all `bytes=N-…` range requests.
fn requested_starts(server: &TestServer) -> Vec<u64> {
    get_requests(server)
        .into_iter()
        .flatten()
        .filter_map(|r| r.strip_prefix("bytes=")?.split('-').next()?.parse().ok())
        .collect()
}

fn assert_file_matches(run: &Run, spec: &FileSpec) {
    let data = std::fs::read(&run.part).unwrap();
    assert_eq!(data.len() as u64, spec.size, "file size");
    assert!(
        data == spec.bytes(0, spec.size as usize),
        "file content differs from the served file"
    );
}

fn get_requests(server: &TestServer) -> Vec<Option<String>> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .map(|r| r.range)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_every_byte_and_reports_what_it_probed() {
    let spec = FileSpec::new(3 * MIB).etag("v1");
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(4)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 3 * MIB }
    );
    assert_file_matches(&run, &spec);
    let info = &run.recorder.probed[0];
    assert_eq!(info.total, Some(3 * MIB));
    assert!(info.resumable);
    assert_eq!(info.etag.as_deref(), Some("\"v1\""));
    assert_eq!(info.final_url, server.url("/f.bin"));
    // The first request asks for everything, so data flows without a separate probe.
    assert_eq!(get_requests(&server)[0].as_deref(), Some("bytes=0-"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_file_is_split_across_more_connections() {
    let spec = FileSpec::new(4 * MIB).etag("v1").throttle(2 * MIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(4)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 4 * MIB }
    );
    assert_file_matches(&run, &spec);
    let ranges = get_requests(&server);
    assert!(
        ranges.len() >= 3,
        "only {} requests: {ranges:?}",
        ranges.len()
    );
    for range in &ranges[1..] {
        let r = range.as_deref().unwrap_or("");
        assert!(
            r.starts_with("bytes=") && !r.ends_with('-'),
            "added connections ask for bounded ranges, got {r:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_connection_maximum_means_one_request() {
    let spec = FileSpec::new(2 * MIB).etag("v1").throttle(4 * MIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(1)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 2 * MIB }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(get_requests(&server).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_without_ranges_is_downloaded_in_one_stream() {
    let spec = FileSpec::new(2 * MIB).no_ranges().throttle(4 * MIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(4)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 2 * MIB }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(get_requests(&server).len(), 1);
    assert!(!run.recorder.probed[0].resumable);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_of_unknown_size_is_downloaded_to_the_end() {
    let spec = FileSpec::new(MIB + 123).unknown_length();
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(4)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: MIB + 123 }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(run.recorder.probed[0].total, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_file_fails_without_retrying() {
    let server = TestServer::start().await.unwrap();

    let run = run(&server, "/missing.bin", fast_config(4)).await;

    assert!(
        matches!(run.result, Err(EngineError::Status(404))),
        "{:?}",
        run.result
    );
    assert_eq!(get_requests(&server).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_web_page_instead_of_the_file_is_an_error() {
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", FileSpec::new(MIB).html_instead());

    let run = run(&server, "/f.bin", fast_config(4)).await;

    assert!(
        matches!(run.result, Err(EngineError::HtmlInsteadOfFile)),
        "{:?}",
        run.result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_is_checkpointed_while_downloading_and_at_the_end() {
    let spec = FileSpec::new(2 * MIB).etag("v1").throttle(4 * MIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(1)).await;

    assert!(run.result.is_ok(), "{:?}", run.result);
    let saves = &run.recorder.saves;
    assert!(saves.len() >= 3, "{} checkpoints", saves.len());
    assert!(
        saves.iter().any(|s| written(s) > 0 && written(s) < 2 * MIB),
        "no checkpoint captured partial progress"
    );
    assert_eq!(written(saves.last().unwrap()), 2 * MIB);
    // Progress only moves forward between checkpoints.
    for pair in saves.windows(2) {
        assert!(written(&pair[1]) >= written(&pair[0]));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn events_report_the_probe_and_reach_the_full_size() {
    let spec = FileSpec::new(MIB).etag("v1").throttle(4 * MIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec);

    let run = run(&server, "/f.bin", fast_config(2)).await;

    assert!(run.result.is_ok(), "{:?}", run.result);
    assert!(matches!(run.events.first(), Some(Event::Probed(_))));
    let last = run
        .events
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::Progress {
                downloaded, total, ..
            } => Some((*downloaded, *total)),
            _ => None,
        })
        .expect("at least one progress event");
    assert_eq!(last, (MIB, Some(MIB)));
}

// --- resume, changed files, pause ---

#[tokio::test(flavor = "multi_thread")]
async fn resume_fetches_only_the_missing_bytes_and_checks_the_etag() {
    let spec = FileSpec::new(2 * MIB).etag("v1");
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let dir = tempfile::tempdir().unwrap();
    let segments = vec![seg(0, 0, 2 * MIB - 1, MIB)];
    prepare_part(&dir, &spec, &segments);
    let resume = Resume {
        info: resume_info(&server, "/f.bin", 2 * MIB, Some("v1")),
        segments,
    };

    let run = run_in(dir, &server, "/f.bin", fast_config(1), Some(resume), None).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 2 * MIB }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(
        get_requests_with_if_range(&server),
        vec![(
            Some("bytes=1048576-2097151".to_string()),
            Some("\"v1\"".to_string())
        )]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_skips_complete_segments() {
    let spec = FileSpec::new(3 * MIB).etag("v1");
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let dir = tempfile::tempdir().unwrap();
    let segments = vec![
        seg(0, 0, MIB - 1, MIB / 2),
        seg(1, MIB, 2 * MIB - 1, MIB),
        seg(2, 2 * MIB, 3 * MIB - 1, 0),
    ];
    prepare_part(&dir, &spec, &segments);
    let resume = Resume {
        info: resume_info(&server, "/f.bin", 3 * MIB, Some("v1")),
        segments,
    };

    let run = run_in(dir, &server, "/f.bin", fast_config(4), Some(resume), None).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 3 * MIB }
    );
    assert_file_matches(&run, &spec);
    let starts = requested_starts(&server);
    assert!(starts.contains(&(MIB / 2)), "{starts:?}");
    assert!(starts.contains(&(2 * MIB)), "{starts:?}");
    assert!(
        starts.iter().all(|&s| !(MIB..2 * MIB).contains(&s)),
        "the complete segment was requested again: {starts:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_with_a_new_etag_is_downloaded_again_from_zero() {
    let old = FileSpec::new(2 * MIB).seed(1).etag("v1");
    let new = FileSpec::new(2 * MIB).seed(2).etag("v2");
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", new.clone());
    let dir = tempfile::tempdir().unwrap();
    let segments = vec![seg(0, 0, 2 * MIB - 1, MIB)];
    prepare_part(&dir, &old, &segments);
    let resume = Resume {
        info: resume_info(&server, "/f.bin", 2 * MIB, Some("v1")),
        segments,
    };

    let run = run_in(dir, &server, "/f.bin", fast_config(2), Some(resume), None).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 2 * MIB }
    );
    assert_file_matches(&run, &new);
    assert_eq!(
        run.recorder.probed.last().unwrap().etag.as_deref(),
        Some("\"v2\"")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_file_is_an_error_when_restarting_is_off() {
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", FileSpec::new(2 * MIB).seed(2).etag("v2"));
    let dir = tempfile::tempdir().unwrap();
    let segments = vec![seg(0, 0, 2 * MIB - 1, MIB)];
    prepare_part(&dir, &FileSpec::new(2 * MIB), &segments);
    let resume = Resume {
        info: resume_info(&server, "/f.bin", 2 * MIB, Some("v1")),
        segments,
    };
    let config = EngineConfig {
        restart_if_changed: false,
        ..fast_config(1)
    };

    let run = run_in(dir, &server, "/f.bin", config, Some(resume), None).await;

    assert!(
        matches!(run.result, Err(EngineError::RemoteChanged)),
        "{:?}",
        run.result
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_changed_size_is_downloaded_again() {
    // No ETag or date to send as If-Range: the new size is the only sign of change.
    let new = FileSpec::new(3 * MIB).seed(2);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", new.clone());
    let dir = tempfile::tempdir().unwrap();
    let segments = vec![seg(0, 0, 2 * MIB - 1, MIB)];
    prepare_part(&dir, &FileSpec::new(2 * MIB), &segments);
    let resume = Resume {
        info: resume_info(&server, "/f.bin", 2 * MIB, None),
        segments,
    };

    let run = run_in(dir, &server, "/f.bin", fast_config(2), Some(resume), None).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 3 * MIB }
    );
    assert_file_matches(&run, &new);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_part_file_means_starting_over() {
    let spec = FileSpec::new(MIB).etag("v1");
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let resume = Resume {
        info: resume_info(&server, "/f.bin", MIB, Some("v1")),
        segments: vec![seg(0, 0, MIB - 1, MIB / 2)],
    };

    let run = run_in(
        tempfile::tempdir().unwrap(),
        &server,
        "/f.bin",
        fast_config(1),
        Some(resume),
        None,
    )
    .await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: MIB }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(get_requests(&server)[0].as_deref(), Some("bytes=0-"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_that_could_not_resume_starts_over() {
    let spec = FileSpec::new(MIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let dir = tempfile::tempdir().unwrap();
    prepare_part(&dir, &spec, &[]);
    let mut info = resume_info(&server, "/f.bin", MIB, None);
    info.resumable = false;
    let resume = Resume {
        info,
        segments: vec![seg(0, 0, MIB - 1, MIB / 2)],
    };

    let run = run_in(dir, &server, "/f.bin", fast_config(1), Some(resume), None).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: MIB }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(get_requests(&server)[0].as_deref(), Some("bytes=0-"));
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_saves_progress_that_a_resume_completes() {
    let spec = FileSpec::new(4 * MIB).etag("v1").throttle(MIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let paused = run_in(
        tempfile::tempdir().unwrap(),
        &server,
        "/f.bin",
        fast_config(2),
        None,
        Some(Duration::from_millis(700)),
    )
    .await;

    assert_eq!(paused.result.as_ref().unwrap(), &Outcome::Paused);
    let saved = paused.recorder.saves.last().unwrap().clone();
    let done = written(&saved);
    assert!(done > 0 && done < 4 * MIB, "{done} bytes saved");
    // Every byte the checkpoint counts is really on disk.
    let data = std::fs::read(&paused.part).unwrap();
    for s in &saved {
        let (from, n) = (s.start as usize, s.written as usize);
        assert!(
            data[from..from + n] == spec.bytes(s.start, n)[..],
            "segment {s:?} on disk differs"
        );
    }

    server.set_file("/f.bin", FileSpec::new(4 * MIB).etag("v1"));
    let resume = Resume {
        info: paused.recorder.probed[0].clone(),
        segments: saved,
    };
    let resumed = run_in(
        paused.dir,
        &server,
        "/f.bin",
        fast_config(2),
        Some(resume),
        None,
    )
    .await;

    assert_eq!(
        resumed.result.as_ref().unwrap(),
        &Outcome::Completed { size: 4 * MIB }
    );
    assert_file_matches(&resumed, &spec);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_connection_still_saves_progress_every_flush_interval() {
    // At 256 KiB/s a 1 MiB write buffer takes 4 s to fill. Progress must not wait for it.
    let spec = FileSpec::new(4 * MIB).etag("v1").throttle(256 * KIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let config = EngineConfig {
        write_buffer: MIB as usize,
        flush_interval: Duration::from_millis(300),
        checkpoint_interval: Duration::from_millis(100),
        ..fast_config(1)
    };

    let paused = run_in(
        tempfile::tempdir().unwrap(),
        &server,
        "/f.bin",
        config,
        None,
        Some(Duration::from_millis(1500)),
    )
    .await;

    assert_eq!(paused.result.as_ref().unwrap(), &Outcome::Paused);
    let saved = written(paused.recorder.saves.last().unwrap());
    assert!(
        saved >= 128 * KIB,
        "only {saved} bytes saved after 1.5 s at 256 KiB/s"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_stream_without_ranges_still_reports_progress() {
    let spec = FileSpec::new(4 * MIB).no_ranges().throttle(256 * KIB);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec);
    let config = EngineConfig {
        write_buffer: MIB as usize,
        flush_interval: Duration::from_millis(300),
        ..fast_config(1)
    };

    let paused = run_in(
        tempfile::tempdir().unwrap(),
        &server,
        "/f.bin",
        config,
        None,
        Some(Duration::from_millis(1500)),
    )
    .await;

    let reported = paused
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Progress { downloaded, .. } => Some(*downloaded),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    assert!(
        reported >= 128 * KIB,
        "progress showed only {reported} bytes after 1.5 s at 256 KiB/s"
    );
}

/// Writing far into a fully sized file makes Windows zero-fill everything before the
/// write first: seconds of stalls and double the disk writes on big files. A sparse part
/// file avoids it.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn the_part_file_is_sparse_on_windows() {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;

    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", FileSpec::new(64 * MIB).etag("v1").throttle(MIB));

    let paused = run_in(
        tempfile::tempdir().unwrap(),
        &server,
        "/f.bin",
        fast_config(2),
        None,
        Some(Duration::from_millis(300)),
    )
    .await;

    assert_eq!(paused.result.as_ref().unwrap(), &Outcome::Paused);
    let meta = std::fs::metadata(&paused.part).unwrap();
    assert_eq!(
        meta.len(),
        64 * MIB,
        "the part file still has the full size"
    );
    assert_ne!(
        meta.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE,
        0,
        "the part file is not sparse"
    );
}

// --- retries ---

#[tokio::test(flavor = "multi_thread")]
async fn dropped_connections_are_retried_until_the_file_is_complete() {
    let spec = FileSpec::new(2 * MIB).etag("v1").drop_after(300 * KIB, 3);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(2)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 2 * MIB }
    );
    assert_file_matches(&run, &spec);
    assert!(get_requests(&server).len() >= 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_that_keeps_dropping_finishes_while_it_makes_progress() {
    // Every response breaks after 200 KiB, far more often than the 3 allowed attempts,
    // but each attempt saves progress, so the download must still finish.
    let spec = FileSpec::new(MIB).etag("v1").drop_after(200 * KIB, 1000);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let config = EngineConfig {
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        },
        ..fast_config(1)
    };

    let run = run(&server, "/f.bin", config).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: MIB }
    );
    assert_file_matches(&run, &spec);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_connection_is_restarted_where_it_stopped() {
    let spec = FileSpec::new(2 * MIB).etag("v1").stall_after(300 * KIB, 1);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let config = EngineConfig {
        stall_timeout: Duration::from_millis(300),
        ..fast_config(1)
    };

    let run = run(&server, "/f.bin", config).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: 2 * MIB }
    );
    assert_file_matches(&run, &spec);
    let starts = requested_starts(&server);
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert!(
        starts[1] > 0 && starts[1] <= 300 * KIB,
        "the retry should continue where the stall happened, not restart: {starts:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn server_errors_are_retried() {
    let spec = FileSpec::new(MIB).etag("v1").fail(500, 2, None);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(1)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: MIB }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(get_requests(&server).len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_after_is_honoured() {
    let spec = FileSpec::new(MIB).etag("v1").fail(503, 1, Some(1));
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());
    let started = std::time::Instant::now();

    let run = run(&server, "/f.bin", fast_config(1)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: MIB }
    );
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "retried after {:?}, server asked for 1 s",
        started.elapsed()
    );
    assert_eq!(get_requests(&server).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_stops_after_the_maximum_attempts() {
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", FileSpec::new(MIB).fail(500, 100, None));
    let config = EngineConfig {
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        },
        ..fast_config(1)
    };

    let run = run(&server, "/f.bin", config).await;

    assert!(
        matches!(run.result, Err(EngineError::Status(500))),
        "{:?}",
        run.result
    );
    assert_eq!(get_requests(&server).len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_segment_that_never_progresses_stops_after_the_maximum_attempts() {
    let spec = FileSpec::new(MIB).etag("v1");
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone().fail(500, 100, None));
    let dir = tempfile::tempdir().unwrap();
    let segments = vec![seg(0, 0, MIB - 1, MIB / 2)];
    prepare_part(&dir, &spec, &segments);
    let resume = Resume {
        info: resume_info(&server, "/f.bin", MIB, Some("v1")),
        segments,
    };
    let config = EngineConfig {
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        },
        ..fast_config(1)
    };

    let run = run_in(dir, &server, "/f.bin", config, Some(resume), None).await;

    assert!(
        matches!(run.result, Err(EngineError::Status(500))),
        "{:?}",
        run.result
    );
    assert_eq!(get_requests(&server).len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_stream_without_ranges_starts_over() {
    let spec = FileSpec::new(MIB).no_ranges().drop_after(300 * KIB, 1);
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", spec.clone());

    let run = run(&server, "/f.bin", fast_config(1)).await;

    assert_eq!(
        run.result.as_ref().unwrap(),
        &Outcome::Completed { size: MIB }
    );
    assert_file_matches(&run, &spec);
    assert_eq!(get_requests(&server).len(), 2);
}
