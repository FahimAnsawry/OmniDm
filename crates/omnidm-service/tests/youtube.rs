// SPDX-License-Identifier: GPL-3.0-or-later

//! YouTube through yt-dlp: fetching and checking yt-dlp itself, and a whole download with a
//! stand-in yt-dlp (a batch file) whose formats point at the test server.

use std::path::Path;
use std::time::Duration;

use omnidm_core::engine::EngineConfig;
use omnidm_ipc::protocol::{FinishStatus, Finished};
use omnidm_service::manager::{Manager, ManagerConfig, MediaAdd, Update};
use omnidm_service::ytdlp::{PIECE_SIZE, PROGRAM, YtDlp};
use omnidm_testserver::{FileSpec, TestServer};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

const WATCH: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

struct Env {
    server: TestServer,
    manager: Manager,
    data: tempfile::TempDir,
    downloads: tempfile::TempDir,
}

async fn env() -> Env {
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: downloads.path().to_path_buf(),
        max_active: 3,
        engine: EngineConfig::default(),
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();
    Env {
        server: TestServer::start().await.unwrap(),
        manager,
        data,
        downloads,
    }
}

async fn finished(updates: &mut broadcast::Receiver<Update>, id: i64) -> Finished {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Ok(Update::Finished(done)) = updates.recv().await
                && done.download_id == id
            {
                return done;
            }
        }
    })
    .await
    .expect("the download didn't finish")
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn yt_dlp_is_fetched_once_and_checked() {
    let env = env().await;
    let program = b"pretend this is yt-dlp".to_vec();
    env.server.set_bytes(
        &format!("/rel/{PROGRAM}"),
        "application/octet-stream",
        program.clone(),
    );
    let sums = format!("0000  yt-dlp_linux\n{}  {PROGRAM}\n", sha256(&program));
    env.server
        .set_text("/rel/SHA2-256SUMS", "text/plain", &sums);

    let client = omnidm_net::download_client().unwrap();
    let path = env.data.path().join("tools").join(PROGRAM);
    let ytdlp = YtDlp::new(path.clone(), Some(env.server.url("/rel/")));
    let cancel = CancellationToken::new();
    assert!(
        ytdlp
            .ensure(&client, &EngineConfig::default(), &cancel)
            .await
            .unwrap()
    );
    assert_eq!(std::fs::read(&path).unwrap(), program);
    // Kept: not fetched again.
    let asked = env.server.requests().len();
    assert!(
        ytdlp
            .ensure(&client, &EngineConfig::default(), &cancel)
            .await
            .unwrap()
    );
    assert_eq!(env.server.requests().len(), asked);

    // A copy that doesn't match its checksum is thrown away.
    env.server.set_text(
        "/rel/SHA2-256SUMS",
        "text/plain",
        &format!("{}  {PROGRAM}\n", sha256(b"other")),
    );
    let elsewhere = env.data.path().join("other").join(PROGRAM);
    let tampered = YtDlp::new(elsewhere.clone(), Some(env.server.url("/rel/")));
    let error = tampered
        .ensure(&client, &EngineConfig::default(), &cancel)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("checksum"), "{error}");
    assert!(!elsewhere.exists());
    assert!(!elsewhere.with_extension("download").exists());

    // Without a release to fetch from, a missing copy is an error saying where it was looked for.
    let missing = YtDlp::new(env.data.path().join("nowhere").join(PROGRAM), None);
    let error = missing
        .ensure(&client, &EngineConfig::default(), &cancel)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("yt-dlp"), "{error}");
}

/// A batch file that answers like yt-dlp: prints `answer` (or, failing, `error` on stderr).
#[cfg(windows)]
fn fake_ytdlp(dir: &Path, answer: Option<&str>, error: Option<&str>) -> std::path::PathBuf {
    let script = dir.join("yt-dlp.cmd");
    let body = match (answer, error) {
        (Some(json), _) => {
            let file = dir.join("answer.json");
            std::fs::write(&file, json).unwrap();
            format!("@echo off\r\ntype \"{}\"\r\n", file.display())
        }
        (None, Some(error)) => format!("@echo off\r\necho ERROR: {error} 1>&2\r\nexit /b 1\r\n"),
        (None, None) => unreachable!(),
    };
    std::fs::write(&script, body).unwrap();
    script
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_youtube_video_downloads_its_picture_and_sound_in_pieces() {
    let env = env().await;
    // 1080p picture without sound, in three pieces; the sound in one.
    let picture = FileSpec::new(2 * PIECE_SIZE + 4321)
        .seed(11)
        .content_type("video/mp4");
    let sound = FileSpec::new(700_000).seed(12).content_type("audio/mp4");
    env.server.set_file(
        "/videoplayback/137",
        picture.clone().require_header("user-agent", "TestTube/1.0"),
    );
    env.server.set_file("/videoplayback/140", sound.clone());
    let format = |id: &str, extra: serde_json::Value| {
        let mut format = serde_json::json!({
            "format_id": id,
            "url": env.server.url(&format!("/videoplayback/{id}?expire=1&sig=abc")),
            "protocol": "https",
            "http_headers": {"User-Agent": "TestTube/1.0"}
        });
        format
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        format
    };
    let answer = serde_json::json!({
        "title": "Never: a song?",
        "duration": 212.0,
        "live_status": "not_live",
        "formats": [
            format("137", serde_json::json!({"ext": "mp4", "vcodec": "avc1.640028", "acodec": "none",
                "width": 1920, "height": 1080, "fps": 25, "tbr": 4000.0, "filesize": picture.size})),
            format("140", serde_json::json!({"ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2",
                "tbr": 129.0, "filesize": sound.size, "language": "en"})),
        ]
    })
    .to_string();
    env.manager
        .set_ytdlp(fake_ytdlp(env.data.path(), Some(&answer), None), None);

    let info = env
        .manager
        .look_at_media(MediaAdd {
            url: WATCH.into(),
            ..MediaAdd::default()
        })
        .await
        .unwrap();
    assert_eq!(info.kind, "youtube");
    assert_eq!(
        info.title, "Never_ a song_",
        "the video's own title, made safe"
    );
    assert_eq!(info.qualities[0].height, Some(1080));
    assert_eq!(info.qualities[0].audio, [0]);

    let mut updates = env.manager.subscribe();
    let id = env
        .manager
        .add_media(MediaAdd {
            url: WATCH.into(),
            dir: Some(env.downloads.path().to_path_buf()),
            ..MediaAdd::default()
        })
        .await
        .unwrap();
    let done = finished(&mut updates, id).await;
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    let saved = |name: &str| std::fs::read(env.downloads.path().join(name)).unwrap();
    assert_eq!(
        saved("Never_ a song_.mp4"),
        picture.bytes(0, picture.size as usize)
    );
    assert_eq!(
        saved("Never_ a song_ (sound).m4a"),
        sound.bytes(0, sound.size as usize)
    );

    // The picture came in pieces, each with the headers yt-dlp named.
    let ranges: Vec<_> = env
        .server
        .requests()
        .into_iter()
        .filter(|r| r.path == "/videoplayback/137")
        .map(|r| r.range.unwrap())
        .collect();
    assert_eq!(ranges.len(), 3, "{ranges:?}");
    assert!(ranges.contains(&format!("bytes={}-{}", 2 * PIECE_SIZE, picture.size - 1)));
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn youtube_problems_are_told_plainly() {
    let env = env().await;
    let fake = fake_ytdlp(
        env.data.path(),
        None,
        Some("[youtube] dQw4w9WgXcQ: Video unavailable"),
    );
    env.manager.set_ytdlp(fake, None);
    let error = env
        .manager
        .add_media(MediaAdd {
            url: WATCH.into(),
            ..MediaAdd::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error, "YouTube: Video unavailable");
    assert!(env.manager.list().is_empty());
}
