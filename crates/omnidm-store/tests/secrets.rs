// SPDX-License-Identifier: GPL-3.0-or-later

//! Saved cookies: encrypted for the current Windows user, gone once a download is done.
#![cfg(windows)]

use omnidm_core::engine::Method;
use omnidm_store::{NewDownload, Status, Store, StoreError};

const COOKIES: &[u8] = br#"[{"name":"session","value":"TOP-SECRET-123"}]"#;

fn new_download() -> NewDownload {
    NewDownload {
        url: "https://e.com/a.zip".into(),
        save_dir: "dl".into(),
        file_name: None,
        referrer: None,
        user_agent: None,
        headers: vec![],
        method: Method::Get,
        body: vec![],
    }
}

fn file_store() -> (tempfile::TempDir, Store, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("omnidm.db");
    let store = Store::open(&path).unwrap();
    (dir, store, path)
}

#[test]
fn cookies_round_trip() {
    let (_dir, mut store, _) = file_store();
    let id = store.insert_download(&new_download()).unwrap();
    assert_eq!(store.load_cookies(id).unwrap(), None);

    store.save_cookies(id, COOKIES).unwrap();

    assert_eq!(store.load_cookies(id).unwrap().as_deref(), Some(COOKIES));
}

#[test]
fn saving_again_replaces_the_cookies() {
    let (_dir, mut store, _) = file_store();
    let id = store.insert_download(&new_download()).unwrap();
    store.save_cookies(id, b"old").unwrap();
    store.save_cookies(id, b"new").unwrap();
    assert_eq!(
        store.load_cookies(id).unwrap().as_deref(),
        Some(&b"new"[..])
    );
}

#[test]
fn the_database_file_never_holds_readable_cookies() {
    let (_dir, mut store, path) = file_store();
    let id = store.insert_download(&new_download()).unwrap();
    store.save_cookies(id, COOKIES).unwrap();
    drop(store);

    let raw: Vec<u8> = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT cookies FROM secrets", [], |r| r.get(0))
        .unwrap();
    assert!(
        !raw.windows(14).any(|w| w == b"TOP-SECRET-123"),
        "cookies are stored in plain text"
    );
}

#[test]
fn completing_a_download_deletes_its_cookies() {
    let (_dir, mut store, _) = file_store();
    let id = store.insert_download(&new_download()).unwrap();
    store.save_cookies(id, COOKIES).unwrap();

    store.set_status(id, Status::Paused, None).unwrap();
    assert!(
        store.load_cookies(id).unwrap().is_some(),
        "kept while unfinished"
    );
    store.set_status(id, Status::Completed, None).unwrap();

    assert_eq!(store.load_cookies(id).unwrap(), None);
}

#[test]
fn deleting_a_download_deletes_its_cookies() {
    let (_dir, mut store, path) = file_store();
    let id = store.insert_download(&new_download()).unwrap();
    store.save_cookies(id, COOKIES).unwrap();

    store.delete_download(id).unwrap();

    let left: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn damaged_saved_cookies_are_an_error_not_a_panic() {
    let (_dir, mut store, path) = file_store();
    let id = store.insert_download(&new_download()).unwrap();
    store.save_cookies(id, COOKIES).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE secrets SET cookies = x'00010203'", [])
        .unwrap();

    match store.load_cookies(id) {
        Err(StoreError::Corrupt(_)) => {}
        other => panic!("expected Corrupt, got {other:?}"),
    }
}
