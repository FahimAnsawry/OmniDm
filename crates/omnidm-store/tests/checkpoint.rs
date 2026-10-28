// SPDX-License-Identifier: GPL-3.0-or-later

use omnidm_core::engine::{Checkpointer, RemoteInfo};
use omnidm_core::{Segment, SegmentId};
use omnidm_store::checkpoint::{StoreCheckpointer, resume_from};
use omnidm_store::{NewDownload, Status, Store};

fn new_download(url: &str, dir: &str) -> NewDownload {
    NewDownload {
        url: url.into(),
        save_dir: dir.into(),
        file_name: None,
        referrer: None,
        user_agent: None,
        headers: vec![],
    }
}

fn info() -> RemoteInfo {
    RemoteInfo {
        final_url: "https://cdn.e.com/a.zip".into(),
        total: Some(1000),
        etag: Some("\"v1\"".into()),
        last_modified: Some("Tue, 14 Nov 2023 22:13:20 GMT".into()),
        resumable: true,
        content_type: Some("application/zip".into()),
        content_disposition: None,
    }
}

fn seg(id: u32, start: u64, end: u64, written: u64) -> Segment {
    Segment {
        id: SegmentId(id),
        start,
        end,
        written,
    }
}

// --- find_unfinished ---

#[test]
fn an_unfinished_download_of_the_same_url_and_folder_is_found() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
        .unwrap();
    store.set_status(id, Status::Paused, None).unwrap();
    let found = store
        .find_unfinished("https://e.com/a.zip", r"C:\dl")
        .unwrap();
    assert_eq!(found.map(|d| d.id), Some(id));
}

#[test]
fn completed_and_cancelled_downloads_are_not_resumed() {
    let mut store = Store::open_in_memory().unwrap();
    for status in [Status::Completed, Status::Cancelled] {
        let id = store
            .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
            .unwrap();
        store.set_status(id, status, None).unwrap();
    }
    assert!(
        store
            .find_unfinished("https://e.com/a.zip", r"C:\dl")
            .unwrap()
            .is_none()
    );
}

#[test]
fn another_url_or_folder_is_a_different_download() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
        .unwrap();
    assert!(
        store
            .find_unfinished("https://e.com/b.zip", r"C:\dl")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .find_unfinished("https://e.com/a.zip", r"D:\other")
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_newest_unfinished_download_wins() {
    let mut store = Store::open_in_memory().unwrap();
    let older = store
        .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
        .unwrap();
    let newer = store
        .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
        .unwrap();
    store
        .set_status(older, Status::Failed, Some("boom"))
        .unwrap();
    let found = store
        .find_unfinished("https://e.com/a.zip", r"C:\dl")
        .unwrap();
    assert_eq!(found.map(|d| d.id), Some(newer));
}

// --- StoreCheckpointer ---

#[test]
fn probe_and_segments_from_the_engine_are_stored() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
        .unwrap();
    {
        let mut cp = StoreCheckpointer {
            store: &mut store,
            id,
            file_name: "a.zip".into(),
        };
        cp.probed(&info()).unwrap();
        cp.save(&[seg(0, 0, 499, 500), seg(1, 500, 999, 10)])
            .unwrap();
    }
    let d = store.get_download(id).unwrap().unwrap();
    assert_eq!(d.final_url.as_deref(), Some("https://cdn.e.com/a.zip"));
    assert_eq!(d.file_name.as_deref(), Some("a.zip"));
    assert_eq!(d.total_size, Some(1000));
    assert_eq!(d.etag.as_deref(), Some("\"v1\""));
    assert_eq!(
        d.last_modified.as_deref(),
        Some("Tue, 14 Nov 2023 22:13:20 GMT")
    );
    assert!(d.resumable);
    assert_eq!(
        store.load_segments(id).unwrap(),
        vec![seg(0, 0, 499, 500), seg(1, 500, 999, 10)]
    );
}

// --- resume_from ---

#[test]
fn a_probed_download_becomes_resume_state() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
        .unwrap();
    StoreCheckpointer {
        store: &mut store,
        id,
        file_name: "a.zip".into(),
    }
    .probed(&info())
    .unwrap();
    let record = store.get_download(id).unwrap().unwrap();
    let segments = vec![seg(0, 0, 999, 300)];

    let resume = resume_from(&record, segments.clone()).expect("resumable");

    assert_eq!(resume.segments, segments);
    assert_eq!(resume.info.final_url, "https://cdn.e.com/a.zip");
    assert_eq!(resume.info.total, Some(1000));
    assert_eq!(resume.info.etag.as_deref(), Some("\"v1\""));
    assert_eq!(
        resume.info.last_modified.as_deref(),
        Some("Tue, 14 Nov 2023 22:13:20 GMT")
    );
    assert!(resume.info.resumable);
}

#[test]
fn a_download_never_probed_has_nothing_to_resume() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a.zip", r"C:\dl"))
        .unwrap();
    let record = store.get_download(id).unwrap().unwrap();
    assert!(resume_from(&record, vec![]).is_none());
}
