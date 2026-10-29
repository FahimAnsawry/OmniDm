// SPDX-License-Identifier: GPL-3.0-or-later

//! Settings that apply while running, and editing categories.

use std::path::PathBuf;
use std::time::Duration;

use omnidm_ipc::protocol::{
    AddRequest, Capture, CaptureResult, CategoryEdit, FinishStatus, Finished, Progress,
};
use omnidm_service::manager::{Manager, ManagerConfig, Update};
use omnidm_testserver::{FileSpec, TestServer};
use tokio::sync::broadcast;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

struct Env {
    server: TestServer,
    manager: Manager,
    _data: tempfile::TempDir,
    downloads: tempfile::TempDir,
    elsewhere: tempfile::TempDir,
}

async fn env(max_active: usize) -> Env {
    let server = TestServer::start().await.unwrap();
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: downloads.path().to_path_buf(),
        max_active,
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
    fn add(&self, path: &str) -> i64 {
        self.manager
            .add(AddRequest {
                url: self.server.url(path),
                dir: self.downloads.path().to_string_lossy().into_owned(),
                file_name: None,
                connections: None,
                headers: vec![],
                cookies: vec![],
                data: None,
                queue: None,
            })
            .unwrap()
    }

    fn slow_file(&self, path: &str) {
        self.server
            .set_file(path, FileSpec::new(8 * MIB).etag(path).throttle(512 * KIB));
    }
}

fn edit(name: &str) -> CategoryEdit {
    CategoryEdit {
        name: name.into(),
        extensions: vec![],
        mime_types: vec![],
        hosts: vec![],
        folder: None,
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
async fn raising_the_limit_starts_waiting_downloads() {
    let env = env(1).await;
    env.slow_file("/a.bin");
    env.slow_file("/b.bin");
    env.slow_file("/c.bin");
    let ids = [env.add("/a.bin"), env.add("/b.bin"), env.add("/c.bin")];
    let active = || ids.iter().filter(|&&id| env.manager.is_active(id)).count();
    let running = || {
        env.manager
            .list()
            .iter()
            .filter(|d| ids.contains(&d.download_id) && d.status == "running")
            .count()
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(active(), 3, "all are running or waiting");
    assert_eq!(env.manager.max_active(), 1);
    assert!(running() <= 1);

    env.manager.set_max_active(3);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(running(), 3);
    assert_eq!(env.manager.max_active(), 3);
    env.manager.set_max_active(100);
    assert_eq!(env.manager.max_active(), 16, "at most 16");
    env.manager.pause_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_per_download_apply_to_new_downloads() {
    let env = env(3).await;
    env.manager.set_max_connections(1);
    assert_eq!(env.manager.max_connections(), 1);
    env.slow_file("/one.bin");
    let mut updates = env.manager.subscribe();
    let id = env.add("/one.bin");
    let mut most = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let seen = tokio::time::timeout(Duration::from_millis(500), async {
            wait_for(&mut updates, |u| match u {
                Update::Progress(Progress {
                    download_id,
                    connections,
                    ..
                }) if *download_id == id => Some(*connections),
                _ => None,
            })
            .await
        })
        .await;
        if let Ok(connections) = seen {
            most = most.max(connections);
        }
    }
    assert_eq!(most, 1, "one connection only");
    env.manager.pause_all();
    env.manager.set_max_connections(0);
    assert_eq!(env.manager.max_connections(), 1, "at least 1");
    env.manager.set_max_connections(1000);
    assert_eq!(env.manager.max_connections(), 32, "at most 32");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_downloads_folder_is_used_by_captures() {
    let env = env(3).await;
    let moved = env.elsewhere.path().join("New downloads");
    assert!(
        env.manager
            .set_downloads_folder(PathBuf::from("relative"))
            .is_err()
    );
    env.manager.set_downloads_folder(moved.clone()).unwrap();
    assert_eq!(env.manager.downloads_folder(), moved);

    env.server.set_file("/c.bin", FileSpec::new(64 * KIB));
    let mut updates = env.manager.subscribe();
    let result = env
        .manager
        .capture(Capture::new(env.server.url("/c.bin"), 1))
        .await;
    let CaptureResult::Accepted { download_id } = result else {
        panic!("not accepted: {result:?}");
    };
    let done = finished(&mut updates, download_id).await;
    assert_eq!(done.status, FinishStatus::Completed, "{done:?}");
    assert_eq!(PathBuf::from(done.path.unwrap()), moved.join("c.bin"));
    let (_, suggested) = env.manager.suggest_place("https://example.com/x.bin", None);
    assert_eq!(suggested, moved);
}

#[tokio::test(flavor = "multi_thread")]
async fn categories_can_be_added_edited_moved_and_deleted() {
    let env = env(3).await;
    let m = &env.manager;
    let books = m
        .create_category(CategoryEdit {
            extensions: vec![".FB2".into(), "azw3".into(), " ".into(), "fb2".into()],
            hosts: vec![
                "https://Books.Example.com/shelf".into(),
                "*.library.org".into(),
            ],
            mime_types: vec!["Application/EPUB+zip".into()],
            ..edit(" Books ")
        })
        .unwrap();
    let all = m.categories();
    let added = all.last().unwrap();
    assert_eq!(added.category_id, books);
    assert_eq!(added.name, "Books");
    assert_eq!(added.extensions, ["fb2", "azw3"]);
    assert_eq!(added.hosts, ["books.example.com", "library.org"]);
    assert_eq!(added.mime_types, ["application/epub+zip"]);
    assert_eq!(added.folder, None);

    // Its rule decides now.
    let (category, _) = m.suggest_place("https://example.com/novel.fb2", None);
    assert_eq!(category.as_deref(), Some("Books"));

    // Renamed, given a folder, and a pdf rule that Documents also has.
    let folder = env.elsewhere.path().join("Books");
    m.edit_category(
        books,
        CategoryEdit {
            extensions: vec!["fb2".into(), "pdf".into()],
            folder: Some(folder.to_string_lossy().into_owned()),
            ..edit("Reading")
        },
    )
    .unwrap();
    let (category, place) = m.suggest_place("https://example.com/paper.pdf", None);
    assert_eq!(
        category.as_deref(),
        Some("Documents"),
        "Documents comes first"
    );
    assert_ne!(place, folder);
    m.move_category(books, 0).unwrap();
    assert_eq!(m.categories()[0].name, "Reading");
    let (category, place) = m.suggest_place("https://example.com/paper.pdf", None);
    assert_eq!(category.as_deref(), Some("Reading"));
    assert_eq!(place, folder);

    m.delete_category(books).unwrap();
    assert!(m.categories().iter().all(|c| c.name != "Reading"));
    assert!(
        m.delete_category(books)
            .unwrap_err()
            .starts_with("no category")
    );
    assert!(
        m.move_category(books, 0)
            .unwrap_err()
            .starts_with("no category")
    );
    assert!(
        m.edit_category(books, edit("X"))
            .unwrap_err()
            .starts_with("no category")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn category_edits_are_checked() {
    let env = env(3).await;
    let m = &env.manager;
    let bad = [
        (edit("  "), "needs a name"),
        (edit(&"x".repeat(41)), "too long"),
        (edit("video"), "already a category"),
        (
            CategoryEdit {
                extensions: vec!["a b".into()],
                ..edit("A")
            },
            "isn't a file extension",
        ),
        (
            CategoryEdit {
                mime_types: vec!["video".into()],
                ..edit("A")
            },
            "isn't a file type",
        ),
        (
            CategoryEdit {
                hosts: vec!["https:///".into()],
                ..edit("A")
            },
            "isn't a site",
        ),
        (
            CategoryEdit {
                folder: Some("Videos".into()),
                ..edit("A")
            },
            "full path",
        ),
    ];
    for (given, expected) in bad {
        let error = m.create_category(given.clone()).unwrap_err();
        assert!(error.contains(expected), "{given:?}: {error}");
    }
    // Keeping its own name (in another case) is fine.
    let video = m
        .categories()
        .into_iter()
        .find(|c| c.name == "Video")
        .unwrap();
    m.edit_category(
        video.category_id,
        CategoryEdit {
            extensions: video.extensions.clone(),
            mime_types: video.mime_types.clone(),
            ..edit("VIDEO")
        },
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn copied_links_that_look_like_files() {
    let env = env(3).await;
    let m = &env.manager;
    assert!(m.looks_like_file("https://example.com/files/setup.exe"));
    assert!(m.looks_like_file("  https://example.com/a/song.MP3?token=1  "));
    assert!(!m.looks_like_file("https://example.com/article"));
    assert!(!m.looks_like_file("https://example.com/page.html"));
    assert!(!m.looks_like_file("ftp://example.com/setup.exe"));
    assert!(!m.looks_like_file("just some text.zip"));
}
