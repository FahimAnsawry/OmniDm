// SPDX-License-Identifier: GPL-3.0-or-later

//! The capture prompt: captures held until the user answers, and saved settings.

use std::path::PathBuf;
use std::time::Duration;

use omnidm_ipc::protocol::{
    AskAbout, Capture, CaptureAnswer, CaptureResult, FinishStatus, Finished,
};
use omnidm_service::manager::{Manager, ManagerConfig, Update};
use omnidm_testserver::{FileSpec, TestServer};
use tokio::sync::broadcast;

const KIB: u64 = 1024;

struct Env {
    server: TestServer,
    manager: Manager,
    _data: tempfile::TempDir,
    downloads: tempfile::TempDir,
    elsewhere: tempfile::TempDir,
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
    manager.set_ask_before_download(true);
    Env {
        server,
        manager,
        _data: data,
        downloads,
        elsewhere: tempfile::tempdir().unwrap(),
    }
}

impl Env {
    /// Captures `path`; returns its id and what the prompt would show.
    async fn capture(&self, path: &str) -> (i64, AskAbout) {
        let mut updates = self.manager.subscribe();
        let result = self
            .manager
            .capture(Capture::new(self.server.url(path), 1))
            .await;
        let CaptureResult::Accepted { download_id } = result else {
            panic!("not accepted: {result:?}");
        };
        let about = wait_for(&mut updates, |u| match u {
            Update::Ask(about) if about.download_id == download_id => Some(about.clone()),
            _ => None,
        })
        .await;
        (download_id, about)
    }

    fn status(&self, id: i64) -> Option<String> {
        self.manager
            .list()
            .into_iter()
            .find(|d| d.download_id == id)
            .map(|d| d.status)
    }

    fn answer(&self, about: &AskAbout) -> CaptureAnswer {
        CaptureAnswer {
            download_id: about.download_id,
            file_name: about.file_name.clone(),
            folder: about.folder.clone(),
            category: about.category.clone(),
            queue: None,
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
    tokio::time::timeout(Duration::from_secs(20), wait)
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
async fn a_capture_waits_for_an_answer() {
    let env = env().await;
    env.server.set_file("/a.zip", FileSpec::new(64 * KIB));
    let (id, about) = env.capture("/a.zip").await;
    assert_eq!(about.file_name, "a.zip");
    assert_eq!(about.size, Some(64 * KIB));
    assert_eq!(about.category.as_deref(), Some("Compressed"));
    assert_eq!(PathBuf::from(&about.folder), env.downloads.path());
    assert_eq!(PathBuf::from(&about.downloads_folder), env.downloads.path());
    assert!(about.resumable);

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !env.manager.is_active(id),
        "nothing starts before the answer"
    );
    assert_eq!(env.status(id).as_deref(), Some("asking"));
    assert_eq!(env.manager.asking(), [about]);
    assert!(
        std::fs::read_dir(env.downloads.path())
            .unwrap()
            .next()
            .is_none()
    );
    assert!(env.manager.resume(id).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_answer_can_rename_and_move_the_download() {
    let env = env().await;
    env.server.set_file("/a.zip", FileSpec::new(64 * KIB));
    let (id, about) = env.capture("/a.zip").await;
    let folder = env.elsewhere.path().join("Mine");
    let mut updates = env.manager.subscribe();
    env.manager
        .answer_capture(CaptureAnswer {
            file_name: "renamed.zip".into(),
            folder: folder.to_string_lossy().into_owned(),
            category: None,
            ..env.answer(&about)
        })
        .unwrap();
    let done = finished(&mut updates, id).await;
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    assert_eq!(
        PathBuf::from(done.path.unwrap()),
        folder.join("renamed.zip")
    );
    let summary = env
        .manager
        .list()
        .into_iter()
        .find(|d| d.download_id == id)
        .unwrap();
    assert_eq!(summary.category, None);
    assert!(env.manager.asking().is_empty());
    // The kept first response was used: the file took one request.
    assert_eq!(env.server.requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn downloading_later_puts_it_in_a_queue() {
    let env = env().await;
    env.server.set_file("/later.bin", FileSpec::new(64 * KIB));
    let (id, about) = env.capture("/later.bin").await;
    env.manager
        .answer_capture(CaptureAnswer {
            queue: Some("main".into()),
            ..env.answer(&about)
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!env.manager.is_active(id));
    assert_eq!(env.status(id).as_deref(), Some("queued"));
    let main = &env.manager.queues()[0];
    assert_eq!(main.download_ids, [id]);

    let mut updates = env.manager.subscribe();
    env.manager.start_queue("Main").unwrap();
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_drops_the_download() {
    let env = env().await;
    env.server.set_file("/no.bin", FileSpec::new(64 * KIB));
    let (id, _) = env.capture("/no.bin").await;
    env.manager.cancel_capture(id).unwrap();
    assert_eq!(env.status(id), None);
    assert!(env.manager.asking().is_empty());
    assert!(env.manager.cancel_capture(id).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_answer_leaves_it_waiting() {
    let env = env().await;
    env.server.set_file("/w.bin", FileSpec::new(64 * KIB));
    let (id, about) = env.capture("/w.bin").await;
    let m = &env.manager;
    let relative = CaptureAnswer {
        folder: "Downloads".into(),
        ..env.answer(&about)
    };
    assert!(
        m.answer_capture(relative)
            .unwrap_err()
            .contains("full path")
    );
    let nameless = CaptureAnswer {
        file_name: "  ".into(),
        ..env.answer(&about)
    };
    assert!(m.answer_capture(nameless).unwrap_err().contains("name"));
    let no_queue = CaptureAnswer {
        queue: Some("Nope".into()),
        ..env.answer(&about)
    };
    assert!(
        m.answer_capture(no_queue)
            .unwrap_err()
            .starts_with("no queue")
    );
    let no_category = CaptureAnswer {
        category: Some("Nope".into()),
        ..env.answer(&about)
    };
    assert!(
        m.answer_capture(no_category)
            .unwrap_err()
            .starts_with("no category")
    );
    assert_eq!(env.status(id).as_deref(), Some("asking"));

    let mut updates = m.subscribe();
    m.answer_capture(env.answer(&about)).unwrap();
    assert_eq!(
        finished(&mut updates, id).await.status,
        FinishStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_waiting_capture_forgets_it() {
    let env = env().await;
    env.server.set_file("/d.bin", FileSpec::new(64 * KIB));
    let (id, _) = env.capture("/d.bin").await;
    env.manager.delete(id, true).await.unwrap();
    assert!(env.manager.asking().is_empty());
    assert_eq!(env.status(id), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_asking_a_capture_starts_at_once() {
    let env = env().await;
    env.manager.set_ask_before_download(false);
    env.server.set_file("/now.bin", FileSpec::new(64 * KIB));
    let mut updates = env.manager.subscribe();
    let result = env
        .manager
        .capture(Capture::new(env.server.url("/now.bin"), 1))
        .await;
    let CaptureResult::Accepted { download_id } = result else {
        panic!("not accepted: {result:?}");
    };
    assert_eq!(
        finished(&mut updates, download_id).await.status,
        FinishStatus::Completed
    );
    assert!(env.manager.asking().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_and_suggested_places() {
    let env = env().await;
    let m = &env.manager;
    assert_eq!(m.setting("ask_before_download"), None);
    m.set_setting("ask_before_download", "false").unwrap();
    assert_eq!(m.setting("ask_before_download").as_deref(), Some("false"));

    let videos = env.elsewhere.path().join("Videos");
    m.set_category_folder("Video", Some(videos.to_string_lossy().into_owned()))
        .unwrap();
    let (category, folder) = m.suggest_place("https://example.com/films/clip.mp4?x=1", None);
    assert_eq!(category.as_deref(), Some("Video"));
    assert_eq!(folder, videos);
    assert!(!videos.exists(), "nothing is created");
    let (category, folder) = m.suggest_place("https://example.com/get", Some("notes.pdf"));
    assert_eq!(category.as_deref(), Some("Documents"));
    assert_eq!(folder, env.downloads.path());
    let (category, _) = m.suggest_place("https://example.com/", None);
    assert_eq!(category, None);
}
