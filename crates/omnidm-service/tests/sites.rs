// SPDX-License-Identifier: GPL-3.0-or-later

//! TikTok, Instagram, Facebook and X through yt-dlp: saved yt-dlp answers read into the
//! right qualities, sound, headers and cookies, and whole downloads with a stand-in yt-dlp
//! (a batch file) whose formats point at the test server.

use std::time::Duration;

use omnidm_core::engine::EngineConfig;
use omnidm_media::{Manifest, Source, StreamKind, Track};
use omnidm_service::manager::{Manager, ManagerConfig};
use omnidm_service::ytdlp::{Site, manifest_from};

fn read(file: &str, site: Site) -> Manifest {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ytdlp")
        .join(file);
    manifest_from(&std::fs::read_to_string(path).unwrap(), site).unwrap()
}

fn track(source: &Source) -> &Track {
    match source {
        Source::Track(track) => track,
        Source::Playlist(_) => panic!("yt-dlp's formats are ready tracks"),
    }
}

fn header<'a>(track: &'a Track, name: &str) -> Option<&'a str> {
    track
        .headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Each quality as (its video number, height, yt-dlp's format id).
fn picked(manifest: &Manifest) -> Vec<(Option<u32>, Option<u32>, String)> {
    manifest
        .qualities
        .iter()
        .map(|q| (q.video, q.height, track(&q.source).id.clone().unwrap()))
        .collect()
}

#[test]
fn tiktok_prefers_no_watermark_and_h264_and_keeps_its_cookies() {
    let manifest = read("tiktok.json", Site::TikTok);
    assert_eq!(manifest.kind, StreamKind::Site("tiktok"));
    assert_eq!(manifest.duration, Some(15.0));
    assert_eq!(
        picked(&manifest),
        [
            (None, Some(1280), "h264_720p_1500000-0".to_owned()),
            (None, Some(1024), "h264_540p_1147254-0".to_owned()),
        ]
    );
    // Picture and sound in one file.
    assert!(manifest.audio.is_empty());
    assert!(manifest.qualities.iter().all(|q| q.audio_group.is_none()));
    let best = track(&manifest.qualities[0].source);
    assert_eq!(header(best, "cookie"), Some("tt_chain_token=abc123"));
    assert_eq!(header(best, "referer"), Some("https://www.tiktok.com/"));
    // Its size is known, so it comes in pieces.
    assert_eq!(best.pieces.len(), 1);
    assert!(best.pieces[0].range.is_some());
}

#[test]
fn instagram_prefers_a_file_with_its_sound() {
    let manifest = read("instagram.json", Site::Instagram);
    assert_eq!(manifest.kind, StreamKind::Site("instagram"));
    // The plain copies name no codecs: picture and sound, as yt-dlp assumes.
    assert_eq!(
        picked(&manifest),
        [
            (None, Some(1280), "1".to_owned()),
            (None, Some(854), "2".to_owned()),
            (None, None, "dash-987654321a".to_owned()),
        ]
    );
    assert_eq!(manifest.qualities[0].audio_group, None);
    let best = track(&manifest.qualities[0].source);
    assert_eq!(
        header(best, "user-agent"),
        Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
    );
    // No size: one piece.
    assert_eq!((best.pieces.len(), best.pieces[0].range), (1, None));
}

#[test]
fn facebook_offers_full_hd_with_separate_sound_and_hd_in_one_file() {
    let manifest = read("facebook.json", Site::Facebook);
    assert_eq!(manifest.kind, StreamKind::Site("facebook"));
    assert_eq!(
        picked(&manifest),
        [
            (None, Some(1080), "2233445566v".to_owned()),
            (None, Some(720), "hd".to_owned()),
            (None, None, "3344556677a".to_owned()),
        ],
        "the plain copy of unknown size is left out"
    );
    assert_eq!(manifest.qualities[0].audio_group.as_deref(), Some("audio"));
    assert_eq!(manifest.qualities[1].audio_group, None);
    assert_eq!(manifest.audio.len(), 1);
    assert_eq!(
        manifest
            .default_audio(&manifest.qualities[0])
            .map(|a| a.bandwidth),
        Some(Some(96_000))
    );
}

#[test]
fn an_x_post_with_two_videos_offers_both() {
    let manifest = read("x.json", Site::X);
    assert_eq!(manifest.kind, StreamKind::Site("x"));
    assert_eq!(
        manifest.title.as_deref(),
        Some("NASA - Two views of the launch")
    );
    // The HLS copy is left out; the plain MP4s name no codecs.
    assert_eq!(
        picked(&manifest),
        [
            (Some(1), Some(720), "http-2176".to_owned()),
            (Some(1), Some(270), "http-832".to_owned()),
            (Some(2), Some(1280), "http-950".to_owned()),
        ]
    );
    assert!(manifest.audio.is_empty());
    // Each keeps its own length, for its size.
    assert_eq!(track(&manifest.qualities[0].source).duration, Some(30.0));
    assert_eq!(track(&manifest.qualities[2].source).duration, Some(12.5));
}

#[tokio::test]
async fn links_from_other_sites_are_not_read_with_yt_dlp() {
    let data = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: data.path().to_path_buf(),
        max_active: 1,
        engine: EngineConfig::default(),
        probe_timeout: Duration::from_secs(1),
    })
    .unwrap();
    // yt-dlp isn't there; a look-alike host must not try it.
    manager.set_ytdlp(data.path().join("missing").join("yt-dlp.exe"), None);
    let error = manager
        .look_at_media(omnidm_service::manager::MediaAdd {
            url: "https://www.tiktok.com.invalid/@a/video/6718335390845095173".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(!error.contains("yt-dlp"), "{error}");
}

#[cfg(windows)]
mod downloads {
    use std::time::Duration;

    use omnidm_core::engine::EngineConfig;
    use omnidm_ipc::protocol::{BrowserCookie, FinishStatus, Finished, Progress};
    use omnidm_service::manager::{Manager, ManagerConfig, MediaAdd, Update};
    use omnidm_service::media::{Choice, QualityChoice};
    use omnidm_service::ytdlp::PIECE_SIZE;
    use omnidm_testserver::{FileSpec, TestServer};
    use tokio::sync::broadcast;

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

    impl Env {
        /// Answers like yt-dlp: prints `answer`, or fails with `error` on stderr.
        fn fake_ytdlp(&self, answer: Result<&str, &str>) {
            let dir = self.data.path();
            let script = dir.join("yt-dlp.cmd");
            let body = match answer {
                Ok(json) => {
                    let file = dir.join("answer.json");
                    std::fs::write(&file, json).unwrap();
                    format!("@echo off\r\ntype \"{}\"\r\n", file.display())
                }
                Err(error) => format!("@echo off\r\necho ERROR: {error} 1>&2\r\nexit /b 1\r\n"),
            };
            std::fs::write(&script, body).unwrap();
            self.manager.set_ytdlp(script, None);
        }

        fn saved(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.downloads.path().join(name)).unwrap()
        }

        fn requests_for(&self, path: &str) -> Vec<Option<String>> {
            self.server
                .requests()
                .into_iter()
                .filter(|r| r.path == path)
                .map(|r| r.range)
                .collect()
        }

        fn add(&self, url: &str) -> MediaAdd {
            MediaAdd {
                url: url.into(),
                dir: Some(self.downloads.path().to_path_buf()),
                ..MediaAdd::default()
            }
        }
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
        tokio::time::timeout(Duration::from_secs(60), wait)
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

    const TIKTOK: &str = "https://www.tiktok.com/@scout2015/video/6718335390845095173";

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tiktok_video_downloads_in_pieces_with_its_cookies_and_resumes() {
        let env = env().await;
        // Served only with the cookie yt-dlp names; slow, so it can be paused partway.
        let video = FileSpec::new(2 * PIECE_SIZE + 4321)
            .seed(31)
            .content_type("video/mp4")
            .require_cookie("tt_chain_token=abc123")
            .throttle(12 * 1024 * 1024);
        env.server.set_file("/tos/h264_540.mp4", video.clone());
        let answer = serde_json::json!({
            "title": "Scramble: a cat?",
            "duration": 15,
            "formats": [{
                "format_id": "h264_540p_1147254-0",
                "url": env.server.url("/tos/h264_540.mp4?expire=1&signature=y"),
                "protocol": "https",
                "ext": "mp4",
                "vcodec": "h264",
                "acodec": "aac",
                "width": 576,
                "height": 1024,
                "tbr": 1147.254,
                "filesize": video.size,
                "http_headers": {"Referer": "https://www.tiktok.com/"},
                "cookies": "tt_chain_token=abc123; Domain=.tiktok.com; Path=/; Secure; Expires=1790000000"
            }]
        })
        .to_string();
        env.fake_ytdlp(Ok(&answer));

        let info = env.manager.look_at_media(env.add(TIKTOK)).await.unwrap();
        assert_eq!(info.kind, "tiktok");
        assert_eq!(info.title, "Scramble_ a cat_");
        assert_eq!(info.qualities.len(), 1);
        assert!(info.qualities[0].audio.is_empty(), "sound is in the file");

        env.manager.set_max_connections(1);
        let mut updates = env.manager.subscribe();
        let id = env.manager.add_media(env.add(TIKTOK)).await.unwrap();
        // Paused once the first piece is in.
        wait_for(&mut updates, |u| match u {
            Update::Progress(Progress {
                download_id,
                downloaded,
                ..
            }) if *download_id == id && *downloaded > PIECE_SIZE + 1024 * 1024 => Some(()),
            _ => None,
        })
        .await;
        assert!(env.manager.pause(id));
        let paused = finished(&mut updates, id).await;
        assert_eq!(paused.status, FinishStatus::Paused, "{paused:?}");

        env.manager.resume(id).await.unwrap();
        let done = finished(&mut updates, id).await;
        assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
        assert_eq!(
            env.saved("Scramble_ a cat_.mp4"),
            video.bytes(0, video.size as usize)
        );
        let ranges = env.requests_for("/tos/h264_540.mp4");
        let first = format!("bytes=0-{}", PIECE_SIZE - 1);
        assert_eq!(
            ranges
                .iter()
                .filter(|r| r.as_deref() == Some(&first))
                .count(),
            1,
            "the first piece was complete; it isn't downloaded again: {ranges:?}"
        );
        assert!(ranges.contains(&Some(format!(
            "bytes={}-{}",
            2 * PIECE_SIZE,
            video.size - 1
        ))));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_video_picked_from_a_post_with_several_is_downloaded() {
        let env = env().await;
        let first = FileSpec::new(300_000).seed(41).content_type("video/mp4");
        let second = FileSpec::new(200_000).seed(42).content_type("video/mp4");
        env.server.set_file("/vid/1.mp4", first);
        env.server.set_file("/vid/2.mp4", second.clone());
        let entry = |n: u32, height: u32| {
            serde_json::json!({
                "id": format!("179000000000000000{n}"),
                "duration": 10.0 * f64::from(n),
                "formats": [{
                    "format_id": format!("http-{n}"),
                    "url": env.server.url(&format!("/vid/{n}.mp4")),
                    "protocol": "https",
                    "ext": "mp4",
                    "width": height * 16 / 9,
                    "height": height,
                    "tbr": 800.0
                }]
            })
        };
        let answer = serde_json::json!({
            "_type": "playlist",
            "title": "Two views",
            "entries": [entry(1, 720), entry(2, 720)]
        })
        .to_string();
        env.fake_ytdlp(Ok(&answer));
        let post = "https://x.com/NASA/status/1790000000000000000";

        let info = env.manager.look_at_media(env.add(post)).await.unwrap();
        assert_eq!(info.kind, "x");
        let videos: Vec<_> = info.qualities.iter().map(|q| q.video).collect();
        assert_eq!(videos, [Some(1), Some(2)]);
        // Sizes from each video's own length: 800 kbit/s for 10 and 20 seconds.
        let sizes: Vec<_> = info.qualities.iter().map(|q| q.size).collect();
        assert_eq!(sizes, [Some(1_000_000), Some(2_000_000)]);

        let mut add = env.add(post);
        add.choice = Choice {
            quality: Some(QualityChoice {
                height: Some(720),
                bandwidth: 800_000,
                video: Some(2),
            }),
            audio: None,
        };
        let mut updates = env.manager.subscribe();
        let id = env.manager.add_media(add).await.unwrap();
        let done = finished(&mut updates, id).await;
        assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
        assert_eq!(
            env.saved("Two views.mp4"),
            second.bytes(0, second.size as usize)
        );
        assert!(env.requests_for("/vid/1.mp4").is_empty(), "only video 2");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pasted_link_to_a_login_only_post_says_to_use_the_browser() {
        let env = env().await;
        env.fake_ytdlp(Err(
            "[Instagram] C1a2B3c4D5e: Requested content is not available, rate-limit reached or login required. Use --cookies-from-browser or --cookies for the authentication.",
        ));
        let reel = "https://www.instagram.com/reel/C1a2B3c4D5e/";

        // Pasted into Add download: no cookies.
        let error = env.manager.add_media(env.add(reel)).await.unwrap_err();
        assert_eq!(
            error,
            "Instagram: this post needs you to be logged in. Open it in your browser and use the Download button there"
        );

        // From the browser, with its cookies: yt-dlp's reason, without its advice.
        let mut from_browser = env.add(reel);
        from_browser.cookies = vec![BrowserCookie {
            name: "sessionid".into(),
            value: "s".into(),
            domain: ".instagram.com".into(),
            host_only: false,
            path: "/".into(),
            secure: true,
            http_only: true,
            expiration_date: None,
        }];
        let error = env.manager.add_media(from_browser).await.unwrap_err();
        assert_eq!(
            error,
            "Instagram: Requested content is not available, rate-limit reached or login required."
        );
        assert!(env.manager.list().is_empty());
    }
}
