// SPDX-License-Identifier: GPL-3.0-or-later

//! Categories decide where browser downloads are saved.

use std::path::{Path, PathBuf};
use std::time::Duration;

use omnidm_ipc::protocol::{
    AddRequest, Capture, CaptureResult, DownloadSummary, FinishStatus, Finished,
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
    /// For category folders.
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
    Env {
        server,
        manager,
        _data: data,
        downloads,
        elsewhere: tempfile::tempdir().unwrap(),
    }
}

impl Env {
    fn folder(&self, name: &str) -> PathBuf {
        self.elsewhere.path().join(name)
    }

    fn set_folder(&self, category: &str, folder: Option<&Path>) {
        self.manager
            .set_category_folder(category, folder.map(|f| f.to_string_lossy().into_owned()))
            .unwrap();
    }

    /// Captures `path` from the browser and waits for it to complete; returns the file.
    async fn capture(&self, path: &str) -> (PathBuf, DownloadSummary) {
        let mut updates = self.manager.subscribe();
        let result = self
            .manager
            .capture(Capture::new(self.server.url(path), 1))
            .await;
        let CaptureResult::Accepted { download_id } = result else {
            panic!("not accepted: {result:?}");
        };
        let done = finished(&mut updates, download_id).await;
        assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
        (PathBuf::from(done.path.unwrap()), self.summary(download_id))
    }

    fn summary(&self, id: i64) -> DownloadSummary {
        self.manager
            .list()
            .into_iter()
            .find(|d| d.download_id == id)
            .unwrap()
    }
}

async fn finished(updates: &mut broadcast::Receiver<Update>, id: i64) -> Finished {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(Update::Finished(done)) = updates.recv().await
                && done.download_id == id
            {
                return done;
            }
        }
    })
    .await
    .expect("the download did not finish in time")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capture_goes_to_its_categorys_folder() {
    let env = env().await;
    env.server.set_file("/a.zip", FileSpec::new(64 * KIB));
    let zips = env.folder("Zips");
    env.set_folder("compressed", Some(&zips));

    let (file, summary) = env.capture("/a.zip").await;
    assert_eq!(file, zips.join("a.zip"), "the folder is made when needed");
    assert_eq!(summary.category.as_deref(), Some("Compressed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_folder_a_subfolder_of_downloads_is_used() {
    let env = env().await;
    env.server.set_file("/b.zip", FileSpec::new(64 * KIB));
    let (file, summary) = env.capture("/b.zip").await;
    assert_eq!(file, env.downloads.path().join("Compressed").join("b.zip"));
    assert_eq!(summary.category.as_deref(), Some("Compressed"));

    // Set, then cleared again.
    env.set_folder("Compressed", Some(&env.folder("Zips")));
    env.set_folder("Compressed", None);
    env.server.set_file("/c.zip", FileSpec::new(64 * KIB));
    let (file, _) = env.capture("/c.zip").await;
    assert_eq!(file, env.downloads.path().join("Compressed").join("c.zip"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_servers_type_decides_when_the_name_doesnt() {
    let env = env().await;
    env.server
        .set_file("/watch", FileSpec::new(64 * KIB).content_type("video/mp4"));
    let videos = env.folder("Videos");
    env.set_folder("Video", Some(&videos));
    let (file, summary) = env.capture("/watch").await;
    assert_eq!(file.parent(), Some(videos.as_path()));
    assert_eq!(summary.category.as_deref(), Some("Video"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_in_no_category_goes_to_the_downloads_folder() {
    let env = env().await;
    env.server.set_file("/data.bin", FileSpec::new(64 * KIB));
    let (file, summary) = env.capture("/data.bin").await;
    assert_eq!(file, env.downloads.path().join("data.bin"));
    assert_eq!(summary.category, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_cant_be_made_falls_back_to_the_downloads_folder() {
    let env = env().await;
    // A folder inside a file can't exist.
    let blocker = env.folder("blocker.txt");
    std::fs::write(&blocker, "not a folder").unwrap();
    env.set_folder("Documents", Some(&blocker.join("Docs")));
    env.server.set_file("/report.pdf", FileSpec::new(64 * KIB));
    let (file, summary) = env.capture("/report.pdf").await;
    assert_eq!(file, env.downloads.path().join("report.pdf"));
    assert_eq!(summary.category.as_deref(), Some("Documents"));
}

#[tokio::test(flavor = "multi_thread")]
async fn several_links_are_placed_by_their_link_names() {
    let env = env().await;
    let music = env.folder("Music");
    env.set_folder("Music", Some(&music));
    let results = env
        .manager
        .capture_many(vec![Capture::new(env.server.url("/songs/one.mp3"), 1)]);
    let CaptureResult::Accepted { download_id } = results[0] else {
        panic!("not accepted: {results:?}");
    };
    let summary = env.summary(download_id);
    assert_eq!(Path::new(&summary.save_dir), music);
    assert_eq!(summary.category.as_deref(), Some("Music"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_keeps_its_own_folder_but_records_the_category() {
    let env = env().await;
    env.set_folder("Compressed", Some(&env.folder("Zips")));
    let mine = env.folder("Mine");
    let id = env
        .manager
        .add(AddRequest {
            url: env.server.url("/d.zip"),
            dir: mine.to_string_lossy().into_owned(),
            file_name: None,
            connections: None,
            headers: vec![],
            cookies: vec![],
            data: None,
            queue: Some("Main".into()),
        })
        .unwrap();
    let summary = env.summary(id);
    assert_eq!(Path::new(&summary.save_dir), mine);
    assert_eq!(summary.category.as_deref(), Some("Compressed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn category_folders_are_checked() {
    let env = env().await;
    let m = &env.manager;
    assert!(
        m.set_category_folder("Video", Some("Videos".into()))
            .unwrap_err()
            .contains("full path")
    );
    assert!(
        m.set_category_folder("Nope", None)
            .unwrap_err()
            .starts_with("no category")
    );
    // Blank means "no folder".
    m.set_category_folder("Video", Some("   ".into())).unwrap();
    let video = m
        .categories()
        .into_iter()
        .find(|c| c.name == "Video")
        .unwrap();
    assert_eq!(video.folder, None);
    assert!(video.extensions.contains(&"mp4".to_string()));
}
