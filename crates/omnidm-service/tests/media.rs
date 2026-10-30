// SPDX-License-Identifier: GPL-3.0-or-later

//! Video streams: HLS and DASH playlists read, their pieces downloaded (some locked with
//! AES-128), paused and resumed, and joined.

use std::path::PathBuf;
use std::time::Duration;

use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use omnidm_ipc::protocol::{FinishStatus, Finished, Progress};
use omnidm_service::manager::{Manager, ManagerConfig, MediaAdd, Update};
use omnidm_service::media::{Choice, QualityChoice};
use omnidm_testserver::{FileSpec, TestServer};
use tokio::sync::broadcast;

const KIB: u64 = 1024;
const KEY: [u8; 16] = *b"0123456789abcdef";

struct Env {
    server: TestServer,
    manager: Manager,
    _data: tempfile::TempDir,
    downloads: tempfile::TempDir,
}

async fn env() -> Env {
    let server = TestServer::start().await.unwrap();
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: downloads.path().to_path_buf(),
        max_active: 3,
        engine: Default::default(),
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();
    Env {
        server,
        manager,
        _data: data,
        downloads,
    }
}

impl Env {
    fn media(&self, path: &str, title: &str) -> MediaAdd {
        MediaAdd {
            url: self.server.url(path),
            title: Some(title.into()),
            referrer: Some(self.server.url("/watch")),
            dir: Some(self.downloads.path().to_path_buf()),
            ..MediaAdd::default()
        }
    }

    /// Serves `count` plain pieces at `<prefix><n>.ts`; returns their bytes, joined.
    fn pieces(
        &self,
        prefix: &str,
        count: u64,
        size: u64,
        spec: impl Fn(FileSpec) -> FileSpec,
    ) -> Vec<u8> {
        let mut all = Vec::new();
        for n in 0..count {
            let file = spec(
                FileSpec::new(size + n)
                    .seed(1000 + n)
                    .content_type("video/mp2t"),
            );
            all.extend(file.bytes(0, file.size as usize));
            self.server.set_file(&format!("{prefix}{n}.ts"), file);
        }
        all
    }

    fn file(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.downloads.path().join(name)).unwrap()
    }

    /// What the downloads folder holds, leaving out the empty Video category folder that
    /// is made when the category's place is worked out (these tests save beside it).
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.downloads.path())
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| {
                !(e.file_name() == "Video" && std::fs::read_dir(e.path()).unwrap().next().is_none())
            })
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn summary(&self, id: i64) -> omnidm_ipc::protocol::DownloadSummary {
        self.manager
            .list()
            .into_iter()
            .find(|d| d.download_id == id)
            .unwrap()
    }

    fn requests_for(&self, path: &str) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|r| r.path == path)
            .count()
    }
}

fn media_playlist(lines: &[String]) -> String {
    format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n{}#EXT-X-ENDLIST\n",
        lines.join("")
    )
}

fn lock(plain: &[u8], sequence: u64) -> Vec<u8> {
    let iv = u128::from(sequence).to_be_bytes();
    cbc::Encryptor::<aes::Aes128>::new(&KEY.into(), &iv.into()).encrypt_padded_vec::<Pkcs7>(plain)
}

async fn wait_for<T>(
    updates: &mut broadcast::Receiver<Update>,
    mut wanted: impl FnMut(&Update) -> Option<T>,
) -> T {
    let wait = async {
        loop {
            match updates.recv().await {
                Ok(update) => {
                    if let Some(found) = wanted(&update) {
                        return found;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("updates ended: {e}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait)
        .await
        .expect("timed out waiting for an update")
}

async fn finished(updates: &mut broadcast::Receiver<Update>, id: i64) -> Finished {
    wait_for(updates, |u| match u {
        Update::Finished(done) if done.download_id == id => Some(done.clone()),
        _ => None,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn hls_picks_the_best_quality_and_unlocks_locked_pieces() {
    let env = env().await;
    env.server.set_text(
        "/show/master.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
low/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720
high/index.m3u8
",
    );
    // 720p: two plain pieces, then three locked ones.
    let plain = env.pieces("/show/high/p", 2, 20 * KIB, |f| f);
    let mut expected = plain.clone();
    let mut lines: Vec<String> = (0..2).map(|n| format!("#EXTINF:4,\np{n}.ts\n")).collect();
    lines.push("#EXT-X-KEY:METHOD=AES-128,URI=\"/keys/k1\"\n".into());
    for n in 2..5u64 {
        let piece = FileSpec::new(30 * KIB + n).seed(2000 + n);
        let clear = piece.bytes(0, piece.size as usize);
        expected.extend(&clear);
        env.server.set_bytes(
            &format!("/show/high/p{n}.ts"),
            "video/mp2t",
            lock(&clear, n),
        );
        lines.push(format!("#EXTINF:4,\np{n}.ts\n"));
    }
    env.server.set_text(
        "/show/high/index.m3u8",
        "application/vnd.apple.mpegurl",
        &media_playlist(&lines),
    );
    env.server
        .set_bytes("/keys/k1", "application/octet-stream", KEY.to_vec());

    let mut updates = env.manager.subscribe();
    let id = env
        .manager
        .add_media(env.media("/show/master.m3u8", "Cat show"))
        .await
        .unwrap();
    let done = finished(&mut updates, id).await;
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    assert_eq!(env.files(), ["Cat show.ts"], "the work folder is gone");
    assert_eq!(env.file("Cat show.ts"), expected);
    assert_eq!(
        PathBuf::from(done.path.unwrap()),
        env.downloads.path().join("Cat show.ts")
    );

    let summary = env.summary(id);
    assert_eq!(summary.status, "completed");
    assert_eq!(summary.file_name.as_deref(), Some("Cat show.ts"));
    assert_eq!(summary.downloaded, expected.len() as u64);
    assert_eq!(
        env.requests_for("/show/low/index.m3u8"),
        0,
        "only the chosen quality"
    );
    assert_eq!(env.requests_for("/keys/k1"), 1, "a key is fetched once");
    // The page's address goes along with every request.
    let referrer = env.server.url("/watch");
    assert!(
        env.server
            .requests()
            .iter()
            .all(|r| r.header("referer") == Some(referrer.as_str()))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dash_saves_picture_and_sound() {
    let env = env().await;
    let file = |path: &str, size: u64, seed: u64| {
        let spec = FileSpec::new(size).seed(seed).content_type("video/mp4");
        env.server.set_file(path, spec.clone());
        spec.bytes(0, size as usize)
    };
    let mut picture = file("/film/v720/init.mp4", 900, 1);
    for n in 1..=3 {
        picture.extend(file(&format!("/film/v720/{n}.m4s"), 40 * KIB + n, 10 + n));
    }
    let mut sound = file("/film/a/init.mp4", 600, 2);
    for n in 1..=2 {
        sound.extend(file(&format!("/film/a/{n}.m4s"), 8 * KIB + n, 20 + n));
    }
    env.server.set_text(
        "/film/manifest.mpd",
        "application/dash+xml",
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT12S">
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="1" duration="4" startNumber="1" initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/$Number$.m4s"/>
      <Representation id="v360" bandwidth="500000" height="360"/>
      <Representation id="v720" bandwidth="2000000" height="720"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="en">
      <SegmentTemplate timescale="1" duration="6" startNumber="1" initialization="a/init.mp4" media="a/$Number$.m4s"/>
      <Representation id="a" bandwidth="128000"/>
    </AdaptationSet>
  </Period>
</MPD>"#,
    );
    let mut updates = env.manager.subscribe();
    let id = env
        .manager
        .add_media(env.media("/film/manifest.mpd", "Film"))
        .await
        .unwrap();
    let done = finished(&mut updates, id).await;
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    assert_eq!(env.files(), ["Film (sound).m4a", "Film.mp4"]);
    assert_eq!(env.file("Film.mp4"), picture);
    assert_eq!(env.file("Film (sound).m4a"), sound);
    assert_eq!(env.requests_for("/film/v360/init.mp4"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chosen_quality_is_kept() {
    let env = env().await;
    env.server.set_text(
        "/m.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720
high.m3u8
",
    );
    let expected = env.pieces("/low", 2, 4 * KIB, |f| f);
    let lines: Vec<String> = (0..2).map(|n| format!("#EXTINF:4,\nlow{n}.ts\n")).collect();
    env.server.set_text(
        "/low.m3u8",
        "application/vnd.apple.mpegurl",
        &media_playlist(&lines),
    );

    let mut add = env.media("/m.m3u8", "Small");
    add.choice = Choice {
        quality: Some(QualityChoice {
            height: Some(360),
            bandwidth: 800_000,
        }),
        audio: None,
    };
    let mut updates = env.manager.subscribe();
    let id = env.manager.add_media(add).await.unwrap();
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
    assert_eq!(env.file("Small.ts"), expected);
    assert_eq!(env.requests_for("/high.m3u8"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_keeps_the_pieces_and_resuming_skips_them() {
    let env = env().await;
    // Slow pieces, so the download can be paused partway.
    let expected = env.pieces("/slow/p", 8, 24 * KIB, |f| f.throttle(32 * KIB));
    let lines: Vec<String> = (0..8).map(|n| format!("#EXTINF:4,\np{n}.ts\n")).collect();
    env.server.set_text(
        "/slow/index.m3u8",
        "application/vnd.apple.mpegurl",
        &media_playlist(&lines),
    );
    env.manager.set_max_connections(2);

    let mut updates = env.manager.subscribe();
    let id = env
        .manager
        .add_media(env.media("/slow/index.m3u8", "Long"))
        .await
        .unwrap();
    // Wait until a few pieces are complete.
    wait_for(&mut updates, |u| match u {
        Update::Progress(Progress {
            download_id,
            downloaded,
            ..
        }) if *download_id == id && *downloaded > 60 * KIB => Some(()),
        _ => None,
    })
    .await;
    assert!(env.manager.pause(id));
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Paused
    );

    let work = env
        .downloads
        .path()
        .join("Long.mp4.omnidm-media")
        .join("main");
    let kept: Vec<String> = std::fs::read_dir(&work)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".piece"))
        .collect();
    assert!(!kept.is_empty(), "complete pieces are kept");
    let summary = env.summary(id);
    assert_eq!(summary.status, "paused");
    assert!(summary.downloaded > 0, "progress is saved");

    let before: Vec<usize> = (0..8)
        .map(|n| env.requests_for(&format!("/slow/p{n}.ts")))
        .collect();
    env.manager.resume(id).await.unwrap();
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
    assert_eq!(env.file("Long.ts"), expected);
    for name in kept {
        let n: usize = name.trim_end_matches(".piece").parse().unwrap();
        assert_eq!(
            env.requests_for(&format!("/slow/p{n}.ts")),
            before[n],
            "piece {n} was complete; it isn't downloaded again"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn drm_and_live_streams_are_refused_before_anything_is_added() {
    let env = env().await;
    env.server.set_text(
        "/drm.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n",
    );
    env.server.set_text(
        "/live.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\na.ts\n",
    );
    let drm = env
        .manager
        .add_media(env.media("/drm.m3u8", "x"))
        .await
        .unwrap_err();
    assert!(drm.contains("protected with DRM"), "{drm}");
    let live = env
        .manager
        .add_media(env.media("/live.m3u8", "x"))
        .await
        .unwrap_err();
    assert!(live.contains("live stream"), "{live}");
    let missing = env
        .manager
        .add_media(env.media("/gone.m3u8", "x"))
        .await
        .unwrap_err();
    assert!(missing.contains("404"), "{missing}");
    assert!(env.manager.list().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_piece_links_pause_with_a_reason_and_delete_cleans_up() {
    let env = env().await;
    env.pieces("/exp/p", 1, 4 * KIB, |f| f);
    env.server
        .set_file("/exp/p1.ts", FileSpec::new(4 * KIB).expire_after(0));
    let lines: Vec<String> = (0..2).map(|n| format!("#EXTINF:4,\np{n}.ts\n")).collect();
    env.server.set_text(
        "/exp/index.m3u8",
        "application/vnd.apple.mpegurl",
        &media_playlist(&lines),
    );

    let mut updates = env.manager.subscribe();
    let id = env
        .manager
        .add_media(env.media("/exp/index.m3u8", "Gone"))
        .await
        .unwrap();
    let done = finished(&mut updates, id).await;
    assert_eq!(done.status, FinishStatus::Paused, "{done:?}");
    assert!(done.error.unwrap().contains("expired"));
    assert!(env.downloads.path().join("Gone.mp4.omnidm-media").is_dir());

    env.manager.delete(id, false).await.unwrap();
    assert!(env.files().is_empty(), "the work folder is deleted too");
}

#[tokio::test(flavor = "multi_thread")]
async fn looking_at_an_hls_stream_lists_its_qualities_and_sizes() {
    let env = env().await;
    env.server.set_text(
        "/tv/master.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"en.m3u8\"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Français\",LANGUAGE=\"fr\",URI=\"fr.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,AUDIO=\"a\"
low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1920x1080,FRAME-RATE=60,AUDIO=\"a\"
high.m3u8
",
    );
    // The best quality's playlist tells the length: 10 pieces of 6 s.
    let lines: Vec<String> = (0..10).map(|n| format!("#EXTINF:6,\np{n}.ts\n")).collect();
    env.server.set_text(
        "/tv/high.m3u8",
        "application/vnd.apple.mpegurl",
        &media_playlist(&lines),
    );

    let info = env
        .manager
        .look_at_media(MediaAdd {
            url: env.server.url("/tv/master.m3u8"),
            ..MediaAdd::default()
        })
        .await
        .unwrap();
    assert_eq!(info.kind, "hls");
    assert_eq!(
        info.title, "tv",
        "named after the folder: master says nothing"
    );
    assert_eq!(info.duration, Some(60.0));
    let heights: Vec<_> = info.qualities.iter().map(|q| q.height).collect();
    assert_eq!(heights, [Some(1080), Some(360)]);
    let best = &info.qualities[0];
    assert_eq!(best.frame_rate, Some(60.0));
    assert_eq!(best.size, Some(4_000_000 / 8 * 60));
    assert_eq!(best.audio, [0, 1]);
    assert_eq!(best.default_audio, Some(0));
    let names: Vec<_> = info.audio.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["English", "Français"]);
    assert!(env.manager.list().is_empty(), "looking adds nothing");

    let titled = env
        .manager
        .look_at_media(MediaAdd {
            url: env.server.url("/tv/master.m3u8"),
            title: Some("Evening news".into()),
            ..MediaAdd::default()
        })
        .await
        .unwrap();
    assert_eq!(titled.title, "Evening news");
}

#[tokio::test(flavor = "multi_thread")]
async fn looking_at_a_dash_stream_counts_its_sound_in_the_size() {
    let env = env().await;
    env.server.set_text(
        "/d.mpd",
        "application/dash+xml",
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT100S">
  <Period>
    <AdaptationSet mimeType="video/mp4">
      <Representation id="v" bandwidth="1000000" height="720"><BaseURL>v.mp4</BaseURL></Representation>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="de">
      <Representation id="a" bandwidth="200000"><BaseURL>a.mp4</BaseURL></Representation>
    </AdaptationSet>
  </Period>
</MPD>"#,
    );
    let info = env
        .manager
        .look_at_media(MediaAdd {
            url: env.server.url("/d.mpd"),
            ..MediaAdd::default()
        })
        .await
        .unwrap();
    assert_eq!(info.kind, "dash");
    assert_eq!(info.qualities[0].size, Some(1_200_000 / 8 * 100));
    assert_eq!(info.audio[0].language.as_deref(), Some("de"));
    assert_eq!(info.qualities[0].default_audio, Some(0));

    let refused = env
        .manager
        .look_at_media(MediaAdd {
            url: env.server.url("/nothing.mpd"),
            ..MediaAdd::default()
        })
        .await
        .unwrap_err();
    assert!(refused.contains("404"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_can_wait_in_a_queue() {
    let env = env().await;
    let expected = env.pieces("/q/p", 2, 4 * KIB, |f| f);
    let lines: Vec<String> = (0..2).map(|n| format!("#EXTINF:4,\np{n}.ts\n")).collect();
    env.server.set_text(
        "/q/index.m3u8",
        "application/vnd.apple.mpegurl",
        &media_playlist(&lines),
    );
    let queue = env.manager.queues()[0].name.clone();

    let mut add = env.media("/q/index.m3u8", "Later");
    add.queue = Some(queue.clone());
    let id = env.manager.add_media(add).await.unwrap();
    let summary = env.summary(id);
    assert_eq!(summary.queue.as_deref(), Some(queue.as_str()));
    assert_ne!(summary.status, "running", "it waits until the queue starts");

    let mut updates = env.manager.subscribe();
    env.manager.start_queue(&queue).unwrap();
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
    assert_eq!(env.file("Later.ts"), expected);
}
