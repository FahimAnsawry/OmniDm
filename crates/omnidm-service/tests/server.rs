// SPDX-License-Identifier: GPL-3.0-or-later

//! The protocol over one connection, as the extension (through the host) and the CLI use it.

use std::time::Duration;

use omnidm_core::engine::EngineConfig;
use omnidm_core::{RampMode, RampPolicy, SplitPolicy};
use omnidm_ipc::frame::{read_frame_async, write_frame_async};
use omnidm_ipc::protocol::{
    AddRequest, BrowserCookie, Capture, CaptureResult, CookiesResponse, DownloadRef, Envelope,
    FinishStatus, Hello, MediaFound, MediaQualityPick, Message, PROTOCOL_VERSION,
};
use omnidm_service::manager::{Manager, ManagerConfig, Update};
use omnidm_service::server::handle_connection;
use omnidm_testserver::{FileSpec, TestServer};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

struct Env {
    server: TestServer,
    manager: Manager,
    downloads: tempfile::TempDir,
    _data: tempfile::TempDir,
}

async fn env() -> Env {
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: downloads.path().to_path_buf(),
        max_active: 3,
        engine: EngineConfig {
            split: SplitPolicy {
                min_segment: 128 * KIB,
                align: 4 * KIB,
            },
            ramp: RampPolicy {
                max_connections: 4,
                mode: RampMode::Doubling,
                cooldown: Duration::from_millis(100),
            },
            tick: Duration::from_millis(20),
            checkpoint_interval: Duration::from_millis(50),
            flush_interval: Duration::from_millis(100),
            ..EngineConfig::default()
        },
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();
    Env {
        server: TestServer::start().await.unwrap(),
        manager,
        downloads,
        _data: data,
    }
}

/// One client connected to the app.
struct Client {
    reader: ReadHalf<DuplexStream>,
    writer: WriteHalf<DuplexStream>,
    next: u32,
}

impl Client {
    fn connect(manager: &Manager) -> Self {
        let (ours, theirs) = tokio::io::duplex(1 << 20);
        tokio::spawn(handle_connection(manager.clone(), theirs));
        let (reader, writer) = tokio::io::split(ours);
        Self {
            reader,
            writer,
            next: 0,
        }
    }

    async fn send_raw(&mut self, bytes: &[u8]) {
        write_frame_async(&mut self.writer, bytes).await.unwrap();
    }

    /// Sends `message`; returns its id.
    async fn send(&mut self, message: Message) -> String {
        self.next += 1;
        let id = format!("c{}", self.next);
        let envelope = Envelope::new(id.clone(), message);
        self.send_raw(&serde_json::to_vec(&envelope).unwrap()).await;
        id
    }

    async fn recv(&mut self) -> Envelope {
        let frame = tokio::time::timeout(
            Duration::from_secs(20),
            read_frame_async(&mut self.reader, 4 << 20),
        )
        .await
        .expect("no message from the app")
        .unwrap()
        .expect("the app closed the connection");
        serde_json::from_slice(&frame).unwrap()
    }

    /// The next message answering `id`, skipping others.
    async fn reply_to(&mut self, id: &str) -> Envelope {
        loop {
            let envelope = self.recv().await;
            if envelope.re.as_deref() == Some(id) {
                return envelope;
            }
        }
    }

    async fn hello(&mut self) {
        let id = self
            .send(Message::Hello(Hello {
                extension_version: "0.1.0".into(),
                browser: "chrome".into(),
                browser_version: "156".into(),
                protocols: vec![1],
            }))
            .await;
        self.reply_to(&id).await;
    }
}

fn session(value: &str) -> BrowserCookie {
    BrowserCookie {
        name: "session".into(),
        value: value.into(),
        domain: "127.0.0.1".into(),
        host_only: true,
        path: "/".into(),
        secure: false,
        http_only: false,
        expiration_date: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_is_answered_with_the_app_version_and_protocol() {
    let env = env().await;
    let mut client = Client::connect(&env.manager);

    let id = client
        .send(Message::Hello(Hello {
            extension_version: "0.1.0".into(),
            browser: "edge".into(),
            browser_version: "156".into(),
            protocols: vec![1],
        }))
        .await;
    let reply = client.reply_to(&id).await;

    match reply.message {
        Message::HelloAck(ack) => {
            assert_eq!(ack.protocol, PROTOCOL_VERSION);
            assert_eq!(ack.app_version, env!("CARGO_PKG_VERSION"));
        }
        other => panic!("expected helloAck, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ping_gets_a_pong() {
    let env = env().await;
    let mut client = Client::connect(&env.manager);
    let id = client.send(Message::Ping).await;
    assert_eq!(client.reply_to(&id).await.message, Message::Pong);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_message_gets_an_error_and_the_connection_stays_up() {
    let env = env().await;
    let mut client = Client::connect(&env.manager);

    client.send_raw(b"this is not json").await;
    match client.recv().await.message {
        Message::Error(e) => assert_eq!(e.code, "badMessage"),
        other => panic!("expected an error, got {other:?}"),
    }
    let id = client.send(Message::Ping).await;
    assert_eq!(client.reply_to(&id).await.message, Message::Pong);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capture_is_answered_and_downloaded() {
    let env = env().await;
    let spec = FileSpec::new(MIB).etag("v1");
    env.server.set_file("/a.zip", spec.clone());
    let mut client = Client::connect(&env.manager);
    client.hello().await;

    let id = client
        .send(Message::Capture(Capture::new(env.server.url("/a.zip"), 7)))
        .await;

    let reply = client.reply_to(&id).await;
    let Message::CaptureResult(CaptureResult::Accepted { download_id }) = reply.message else {
        panic!("not accepted: {:?}", reply.message);
    };
    // Watch it to the end.
    let watch = client
        .send(Message::Watch(DownloadRef { download_id }))
        .await;
    loop {
        if let Message::Finished(done) = client.reply_to(&watch).await.message {
            assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
            break;
        }
    }
    let saved = std::fs::read(env.downloads.path().join("a.zip")).unwrap();
    assert!(saved == spec.bytes(0, spec.size as usize));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_adds_watches_and_lists_downloads() {
    let env = env().await;
    env.server.set_file(
        "/f.bin",
        FileSpec::new(2 * MIB).etag("v1").throttle(4 * MIB),
    );
    let mut client = Client::connect(&env.manager);

    let id = client
        .send(Message::Add(AddRequest {
            url: env.server.url("/f.bin"),
            dir: env.downloads.path().to_string_lossy().into_owned(),
            file_name: None,
            connections: Some(2),
            headers: vec![],
            cookies: vec![],
            data: None,
            queue: None,
        }))
        .await;
    let Message::Added(added) = client.reply_to(&id).await.message else {
        panic!("not added");
    };

    let watch = client.send(Message::Watch(added)).await;
    let mut progress = 0;
    let done = loop {
        match client.reply_to(&watch).await.message {
            Message::Progress(_) => progress += 1,
            Message::Finished(done) => break done,
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(done.status, FinishStatus::Completed);
    assert!(progress > 0, "progress was reported");

    let list = client.send(Message::List).await;
    let Message::Downloads(downloads) = client.reply_to(&list).await.message else {
        panic!("no list");
    };
    assert_eq!(downloads.items.len(), 1);
    assert_eq!(downloads.items[0].status, "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn watching_a_finished_download_answers_at_once() {
    let env = env().await;
    env.server
        .set_file("/f.bin", FileSpec::new(64 * KIB).etag("v1"));
    let mut client = Client::connect(&env.manager);
    let mut updates = env.manager.subscribe();
    let download_id = env
        .manager
        .add(AddRequest {
            url: env.server.url("/f.bin"),
            dir: env.downloads.path().to_string_lossy().into_owned(),
            file_name: None,
            connections: None,
            headers: vec![],
            cookies: vec![],
            data: None,
            queue: None,
        })
        .unwrap();
    while !matches!(
        updates.recv().await,
        Ok(omnidm_service::manager::Update::Finished(_))
    ) {}

    let watch = client
        .send(Message::Watch(DownloadRef { download_id }))
        .await;

    match client.reply_to(&watch).await.message {
        Message::Finished(done) => assert_eq!(done.status, FinishStatus::Completed),
        other => panic!("expected finished, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_an_unknown_download_is_an_error() {
    let env = env().await;
    let mut client = Client::connect(&env.manager);
    let id = client
        .send(Message::Pause(DownloadRef { download_id: 999 }))
        .await;
    match client.reply_to(&id).await.message {
        Message::Error(e) => assert_eq!(e.code, "notFound"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resuming_gets_fresh_cookies_from_the_connected_browser() {
    let env = env().await;
    let spec = FileSpec::new(2 * MIB).etag("v1");
    env.server.set_file(
        "/f.bin",
        spec.clone().throttle(MIB).require_cookie("session=old"),
    );
    let mut browser = Client::connect(&env.manager);
    browser.hello().await;
    let capture = browser
        .send(Message::Capture(Capture {
            cookies: vec![session("old")],
            ..Capture::new(env.server.url("/f.bin"), 1)
        }))
        .await;
    let Message::CaptureResult(CaptureResult::Accepted { download_id }) =
        browser.reply_to(&capture).await.message
    else {
        panic!("not accepted");
    };
    let mut cli = Client::connect(&env.manager);
    let pause = cli.send(Message::Pause(DownloadRef { download_id })).await;
    assert_eq!(cli.reply_to(&pause).await.message, Message::Ack);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The site now wants the browser's newer session.
    env.server
        .set_file("/f.bin", spec.require_cookie("session=new"));
    let resume = cli.send(Message::Resume(DownloadRef { download_id })).await;

    // The app asks the browser for cookies…
    let request = loop {
        let envelope = browser.recv().await;
        if matches!(envelope.message, Message::CookiesRequest(_)) {
            break envelope;
        }
    };
    let id = request.id.clone();
    let answer = Envelope::new(
        "b1",
        Message::CookiesResponse(CookiesResponse {
            cookies: vec![session("new")],
        }),
    )
    .reply_to(id);
    browser
        .send_raw(&serde_json::to_vec(&answer).unwrap())
        .await;

    // …and the download finishes with them.
    assert_eq!(cli.reply_to(&resume).await.message, Message::Ack);
    let watch = cli.send(Message::Watch(DownloadRef { download_id })).await;
    loop {
        if let Message::Finished(done) = cli.reply_to(&watch).await.message {
            assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
            break;
        }
    }
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn the_app_answers_over_a_real_named_pipe() {
    use omnidm_ipc::pipe::{PipeListener, connect};

    let env = env().await;
    let name = format!(r"\\.\pipe\OmniDm.test.{:016x}", fastrand::u64(..));
    let listener = PipeListener::bind(&name).unwrap();
    tokio::spawn(omnidm_service::server::serve(env.manager.clone(), listener));

    let me = std::env::current_exe().unwrap();
    let mut pipe = connect(&name, &[me]).await.unwrap();
    let ping = Envelope::new("p1", Message::Ping);
    write_frame_async(&mut pipe, &serde_json::to_vec(&ping).unwrap())
        .await
        .unwrap();
    let frame = read_frame_async(&mut pipe, 1 << 20).await.unwrap().unwrap();
    let reply: Envelope = serde_json::from_slice(&frame).unwrap();

    assert_eq!(
        (reply.re.as_deref(), reply.message),
        (Some("p1"), Message::Pong)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn show_asks_the_app_to_open_its_window() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    let env = env().await;
    let shown = Arc::new(AtomicU32::new(0));
    let counter = shown.clone();
    env.manager.set_show_handler(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    let mut client = Client::connect(&env.manager);

    let id = client.send(Message::Show).await;

    assert_eq!(client.reply_to(&id).await.message, Message::Ack);
    assert_eq!(shown.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_found_by_the_extension_waits_for_its_quality_window() {
    let env = env().await;
    let piece = FileSpec::new(8 * KIB).seed(7).require_cookie("session=abc");
    env.server.set_file("/v/p0.ts", piece.clone());
    env.server.set_text(
        "/v/index.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\np0.ts\n#EXT-X-ENDLIST\n",
    );
    let mut updates = env.manager.subscribe();
    let mut client = Client::connect(&env.manager);
    let id = client
        .send(Message::Media(MediaFound {
            kind: "hls".into(),
            url: env.server.url("/v/index.m3u8"),
            page_url: Some(env.server.url("/watch")),
            title: Some("Found it".into()),
            headers: vec![],
            cookies: vec![session("abc")],
            private: false,
            quality: None,
        }))
        .await;
    assert!(matches!(client.reply_to(&id).await.message, Message::Ack));
    let offer = loop {
        if let Ok(Update::MediaOffer { offer }) = updates.recv().await {
            break offer;
        }
    };
    assert!(
        env.manager.list().is_empty(),
        "nothing is added before the user chooses"
    );
    assert_eq!(
        env.manager.offer_quality(offer),
        None,
        "nothing picked on the page"
    );
    assert_eq!(
        env.manager.offer_url(offer),
        Some(env.server.url("/v/index.m3u8"))
    );

    let info = env.manager.look_at_offer(offer).await.unwrap();
    assert_eq!(info.title, "Found it");
    let download = env
        .manager
        .accept_offer(
            offer,
            info.title.clone(),
            env.downloads.path().to_path_buf(),
            Default::default(),
            None,
        )
        .await
        .unwrap();
    let done = loop {
        if let Ok(Update::Finished(done)) = updates.recv().await
            && done.download_id == download
        {
            break done;
        }
    };
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    // The page's cookies went along: the piece needs them.
    let saved = std::fs::read(env.downloads.path().join("Found it.ts")).unwrap();
    assert_eq!(saved, piece.bytes(0, 8 * KIB as usize));
    assert!(
        env.manager.look_at_offer(offer).await.is_err(),
        "taken once"
    );

    // A bad link is refused at once.
    let id = client
        .send(Message::Media(MediaFound {
            kind: "hls".into(),
            url: "file:///C:/x.m3u8".into(),
            page_url: None,
            title: None,
            headers: vec![],
            cookies: vec![],
            private: false,
            quality: None,
        }))
        .await;
    assert!(matches!(
        client.reply_to(&id).await.message,
        Message::Error(_)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_menu_on_a_video_lists_its_qualities_and_keeps_the_pick() {
    let env = env().await;
    env.server.set_text(
        "/m/master.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow.m3u8\n\
         #EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720\nhigh.m3u8\n",
    );
    let found = |quality| MediaFound {
        kind: "hls".into(),
        url: env.server.url("/m/master.m3u8"),
        page_url: None,
        title: Some("Clip".into()),
        headers: vec![],
        cookies: vec![],
        private: false,
        quality,
    };
    let mut updates = env.manager.subscribe();
    let mut client = Client::connect(&env.manager);

    let id = client.send(Message::MediaLook(found(None))).await;
    let Message::MediaInfo(info) = client.reply_to(&id).await.message else {
        panic!("expected the video's qualities");
    };
    let heights: Vec<_> = info.qualities.iter().map(|q| q.height).collect();
    assert_eq!(heights, [Some(720), Some(360)]);

    let picked = MediaQualityPick {
        height: Some(360),
        bandwidth: 800_000,
    };
    let id = client.send(Message::Media(found(Some(picked)))).await;
    assert!(matches!(client.reply_to(&id).await.message, Message::Ack));
    let offer = loop {
        if let Ok(Update::MediaOffer { offer }) = updates.recv().await {
            break offer;
        }
    };
    let quality = env.manager.offer_quality(offer).unwrap();
    assert_eq!((quality.height, quality.bandwidth), (Some(360), 800_000));

    let id = client
        .send(Message::MediaLook(MediaFound {
            url: env.server.url("/m/gone.m3u8"),
            ..found(None)
        }))
        .await;
    assert!(matches!(
        client.reply_to(&id).await.message,
        Message::Error(_)
    ));
}
