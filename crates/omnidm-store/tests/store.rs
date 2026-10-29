// SPDX-License-Identifier: GPL-3.0-or-later

use omnidm_core::engine::Method;
use omnidm_core::{Segment, SegmentId};
use omnidm_store::{
    DownloadId, NewDownload, ProbeResult, SCHEMA_VERSION, Status, Store, StoreError,
};

fn new_download(url: &str) -> NewDownload {
    NewDownload {
        url: url.into(),
        save_dir: r"C:\Users\me\Downloads".into(),
        file_name: None,
        referrer: Some("https://example.com/page".into()),
        user_agent: Some("TestAgent/1.0".into()),
        headers: vec![("Accept-Language".into(), "en".into())],
        method: Method::Get,
        body: vec![],
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

#[test]
fn inserted_download_reads_back_as_queued() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://example.com/a.zip"))
        .unwrap();
    let d = store.get_download(id).unwrap().expect("download exists");
    assert_eq!(d.id, id);
    assert_eq!(d.url, "https://example.com/a.zip");
    assert_eq!(d.save_dir, r"C:\Users\me\Downloads");
    assert_eq!(d.file_name, None);
    assert_eq!(d.referrer.as_deref(), Some("https://example.com/page"));
    assert_eq!(d.user_agent.as_deref(), Some("TestAgent/1.0"));
    assert_eq!(
        d.headers,
        vec![("Accept-Language".to_string(), "en".to_string())]
    );
    assert_eq!(d.status, Status::Queued);
    assert_eq!(d.total_size, None);
    assert!(!d.resumable);
    assert!(
        d.created_at > 1_700_000_000,
        "created_at is a current Unix time"
    );
    assert_eq!(d.completed_at, None);
}

#[test]
fn each_download_gets_its_own_id() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    let b = store
        .insert_download(&new_download("https://e.com/b"))
        .unwrap();
    assert_ne!(a, b);
    assert_eq!(
        store.get_download(b).unwrap().unwrap().url,
        "https://e.com/b"
    );
}

#[test]
fn unknown_download_is_none() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.get_download(DownloadId(42)).unwrap().is_none());
}

#[test]
fn list_returns_downloads_oldest_first() {
    let mut store = Store::open_in_memory().unwrap();
    for name in ["a", "b", "c"] {
        store
            .insert_download(&new_download(&format!("https://e.com/{name}")))
            .unwrap();
    }
    let urls: Vec<String> = store
        .list_downloads()
        .unwrap()
        .into_iter()
        .map(|d| d.url)
        .collect();
    assert_eq!(
        urls,
        ["https://e.com/a", "https://e.com/b", "https://e.com/c"]
    );
}

#[test]
fn probe_result_is_stored() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    store
        .record_probe(
            id,
            &ProbeResult {
                final_url: "https://cdn.e.com/a.zip".into(),
                file_name: "a.zip".into(),
                total_size: Some(5_000_000_000),
                etag: Some("\"abc\"".into()),
                last_modified: Some("Tue, 14 Nov 2023 22:13:20 GMT".into()),
                resumable: true,
                final_method: Method::Get,
            },
        )
        .unwrap();
    let d = store.get_download(id).unwrap().unwrap();
    assert_eq!(d.final_url.as_deref(), Some("https://cdn.e.com/a.zip"));
    assert_eq!(d.file_name.as_deref(), Some("a.zip"));
    assert_eq!(d.total_size, Some(5_000_000_000));
    assert_eq!(d.etag.as_deref(), Some("\"abc\""));
    assert_eq!(
        d.last_modified.as_deref(),
        Some("Tue, 14 Nov 2023 22:13:20 GMT")
    );
    assert!(d.resumable);
}

#[test]
fn failed_status_keeps_its_error_and_others_clear_it() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    store
        .set_status(id, Status::Failed, Some("server returned 404"))
        .unwrap();
    let d = store.get_download(id).unwrap().unwrap();
    assert_eq!(d.status, Status::Failed);
    assert_eq!(d.error.as_deref(), Some("server returned 404"));

    store.set_status(id, Status::Running, None).unwrap();
    let d = store.get_download(id).unwrap().unwrap();
    assert_eq!(d.status, Status::Running);
    assert_eq!(d.error, None);
}

#[test]
fn completing_a_download_records_when() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    store.set_status(id, Status::Completed, None).unwrap();
    let d = store.get_download(id).unwrap().unwrap();
    assert_eq!(d.status, Status::Completed);
    assert!(d.completed_at.is_some_and(|t| t >= d.created_at));
}

#[test]
fn every_status_round_trips() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    for status in [
        Status::Queued,
        Status::Probing,
        Status::Running,
        Status::Paused,
        Status::Completed,
        Status::Failed,
        Status::Cancelled,
    ] {
        store.set_status(id, status, None).unwrap();
        assert_eq!(store.get_download(id).unwrap().unwrap().status, status);
    }
}

#[test]
fn segments_round_trip_in_file_order() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    // Saved out of order on purpose; loaded back sorted by start.
    let saved = [
        seg(3, 6_000_000_000, 9_999_999_999, 0),
        seg(0, 0, 5_999_999_999, 123),
    ];
    store.save_segments(id, &saved).unwrap();
    assert_eq!(
        store.load_segments(id).unwrap(),
        vec![
            seg(0, 0, 5_999_999_999, 123),
            seg(3, 6_000_000_000, 9_999_999_999, 0)
        ]
    );
}

#[test]
fn saving_segments_replaces_the_previous_set() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    store.save_segments(id, &[seg(0, 0, 999, 10)]).unwrap();
    store
        .save_segments(id, &[seg(0, 0, 499, 500), seg(1, 500, 999, 20)])
        .unwrap();
    assert_eq!(
        store.load_segments(id).unwrap(),
        vec![seg(0, 0, 499, 500), seg(1, 500, 999, 20)]
    );
}

#[test]
fn a_failed_save_keeps_the_previous_segments() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    store
        .save_segments(id, &[seg(0, 0, 499, 500), seg(1, 500, 999, 20)])
        .unwrap();
    // Two segments with the same id can't be stored: the whole save must roll back.
    let bad = [seg(0, 0, 499, 500), seg(0, 500, 999, 30)];
    assert!(store.save_segments(id, &bad).is_err());
    assert_eq!(
        store.load_segments(id).unwrap(),
        vec![seg(0, 0, 499, 500), seg(1, 500, 999, 20)]
    );
}

#[test]
fn segments_belong_to_their_own_download() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    let b = store
        .insert_download(&new_download("https://e.com/b"))
        .unwrap();
    store.save_segments(a, &[seg(0, 0, 99, 1)]).unwrap();
    store.save_segments(b, &[seg(0, 0, 199, 2)]).unwrap();
    assert_eq!(store.load_segments(a).unwrap(), vec![seg(0, 0, 99, 1)]);
    assert_eq!(store.load_segments(b).unwrap(), vec![seg(0, 0, 199, 2)]);
}

#[test]
fn deleting_a_download_removes_its_segments() {
    let mut store = Store::open_in_memory().unwrap();
    let a = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    store.save_segments(a, &[seg(0, 0, 99, 1)]).unwrap();
    store.delete_download(a).unwrap();
    assert!(store.get_download(a).unwrap().is_none());
    assert!(store.load_segments(a).unwrap().is_empty());
}

#[test]
fn data_survives_closing_and_reopening_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("omnidm.db");
    let id = {
        let mut store = Store::open(&path).unwrap();
        let id = store
            .insert_download(&new_download("https://e.com/a"))
            .unwrap();
        store.save_segments(id, &[seg(0, 0, 999, 640)]).unwrap();
        id
    };
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.get_download(id).unwrap().unwrap().url,
        "https://e.com/a"
    );
    assert_eq!(store.load_segments(id).unwrap(), vec![seg(0, 0, 999, 640)]);
}

#[test]
fn a_database_from_a_newer_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("omnidm.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
    }
    match Store::open(&path) {
        Err(StoreError::TooNew { found, supported }) => {
            assert_eq!(found, SCHEMA_VERSION + 1);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(_) => panic!("a newer database must be refused"),
    }
}

#[test]
fn a_post_download_keeps_its_method_and_form_data() {
    let mut store = Store::open_in_memory().unwrap();
    let get = store
        .insert_download(&new_download("https://e.com/a"))
        .unwrap();
    let post = store
        .insert_download(&NewDownload {
            method: Method::Post,
            body: b"id=7&agree=1".to_vec(),
            ..new_download("https://e.com/form")
        })
        .unwrap();

    let get = store.get_download(get).unwrap().unwrap();
    assert_eq!((get.method, get.body.as_slice()), (Method::Get, &b""[..]));
    assert_eq!(get.final_method, None, "not probed yet");
    let post = store.get_download(post).unwrap().unwrap();
    assert_eq!(post.method, Method::Post);
    assert_eq!(post.body, b"id=7&agree=1");
}

#[test]
fn the_probe_records_the_method_after_redirects() {
    let mut store = Store::open_in_memory().unwrap();
    let id = store
        .insert_download(&NewDownload {
            method: Method::Post,
            body: b"id=7".to_vec(),
            ..new_download("https://e.com/form")
        })
        .unwrap();
    store
        .record_probe(
            id,
            &ProbeResult {
                final_url: "https://e.com/file.zip".into(),
                file_name: "file.zip".into(),
                total_size: Some(10),
                etag: None,
                last_modified: None,
                resumable: true,
                final_method: Method::Get,
            },
        )
        .unwrap();
    let d = store.get_download(id).unwrap().unwrap();
    assert_eq!(d.method, Method::Post);
    assert_eq!(d.final_method, Some(Method::Get));
}

/// The version 1 schema exactly as M1 shipped it. Never edit: it stands for databases
/// that already exist on users' disks.
const SCHEMA_V1: &str = r#"
    CREATE TABLE downloads (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        url           TEXT    NOT NULL,
        final_url     TEXT,
        save_dir      TEXT    NOT NULL,
        file_name     TEXT,
        referrer      TEXT,
        user_agent    TEXT,
        total_size    INTEGER,
        etag          TEXT,
        last_modified TEXT,
        resumable     INTEGER NOT NULL DEFAULT 0,
        status        TEXT    NOT NULL,
        error         TEXT,
        created_at    INTEGER NOT NULL,
        completed_at  INTEGER
    );
    CREATE TABLE download_headers (
        download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL,
        name        TEXT    NOT NULL,
        value       TEXT    NOT NULL,
        PRIMARY KEY (download_id, position)
    );
    CREATE TABLE segments (
        download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
        segment_id  INTEGER NOT NULL,
        start       INTEGER NOT NULL,
        "end"       INTEGER NOT NULL,
        written     INTEGER NOT NULL,
        PRIMARY KEY (download_id, segment_id)
    );
"#;

#[test]
fn a_version_1_database_is_upgraded_and_keeps_its_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("omnidm.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute(
            "INSERT INTO downloads (url, final_url, save_dir, file_name, total_size, resumable,
                                    status, created_at)
             VALUES ('https://e.com/a.zip', 'https://cdn.e.com/a.zip', 'dl', 'a.zip',
                     1000, 1, 'paused', 1700000000)",
            [],
        )
        .unwrap();
        conn.execute(
            r#"INSERT INTO segments (download_id, segment_id, start, "end", written)
               VALUES (1, 0, 0, 999, 400)"#,
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }

    let store = Store::open(&path).unwrap();

    let d = store.get_download(DownloadId(1)).unwrap().unwrap();
    assert_eq!(d.url, "https://e.com/a.zip");
    assert_eq!(d.status, Status::Paused);
    assert_eq!(d.method, Method::Get);
    assert!(d.body.is_empty());
    assert_eq!(
        d.final_method,
        Some(Method::Get),
        "M1 only made GET downloads, so a probed one ended as GET"
    );
    assert_eq!(
        store.load_segments(d.id).unwrap(),
        vec![seg(0, 0, 999, 400)]
    );
    assert_eq!(d.category, None);
    assert_eq!(store.list_queues().unwrap()[0].settings.name, "Main");
    assert!(!store.list_categories().unwrap().is_empty());
    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}
