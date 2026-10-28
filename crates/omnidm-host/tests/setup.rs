// SPDX-License-Identifier: GPL-3.0-or-later

//! Finding and starting the app, and registering the host with the browsers.
#![cfg(windows)]

use std::path::Path;
use std::time::Duration;

use omnidm_host::{
    BROWSER_KEYS, CHROMIUM_EXTENSION_IDS, FIREFOX_EXTENSION_ID, HOST_NAME, app_paths,
    chromium_manifest, connect_app, firefox_manifest, register, unregister,
};
use omnidm_ipc::pipe::{PipeError, PipeListener};
use serde_json::{Value, json};

#[test]
fn the_apps_next_to_the_host_are_found_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let host = dir.path().join("omnidm-host.exe");
    assert!(app_paths(&host).is_empty());

    std::fs::write(dir.path().join("omnidm-app.exe"), b"").unwrap();
    assert_eq!(app_paths(&host), [dir.path().join("omnidm-app.exe")]);

    // An older installed copy is still allowed, but the newer build is started.
    let installed = dir.path().join("OmniDm.exe");
    std::fs::write(&installed, b"").unwrap();
    let an_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&installed)
        .unwrap()
        .set_modified(an_hour_ago)
        .unwrap();
    assert_eq!(
        app_paths(&host),
        [dir.path().join("omnidm-app.exe"), installed]
    );
}

#[test]
fn the_manifests_name_the_host_and_allow_only_omnidm() {
    let host = Path::new(r"C:\Users\me\AppData\Local\OmniDm\omnidm-host.exe");

    let chromium: Value = serde_json::from_str(&chromium_manifest(host)).unwrap();
    let origins: Vec<String> = CHROMIUM_EXTENSION_IDS
        .iter()
        .map(|id| format!("chrome-extension://{id}/"))
        .collect();
    assert_eq!(
        chromium,
        json!({
            "name": "io.github.fahimansawry.omnidm.host",
            "description": "OmniDm native messaging host",
            "path": host.to_str().unwrap(),
            "type": "stdio",
            "allowed_origins": origins,
        })
    );
    let firefox: Value = serde_json::from_str(&firefox_manifest(host)).unwrap();
    assert_eq!(
        firefox["allowed_extensions"],
        json!(["omnidm@fahimansawry.github.io"])
    );
    assert_eq!(firefox["name"], json!(HOST_NAME));
    assert_eq!(FIREFOX_EXTENSION_ID, "omnidm@fahimansawry.github.io");
}

#[test]
fn every_chromium_id_is_a_valid_extension_id() {
    assert!(!CHROMIUM_EXTENSION_IDS.is_empty());
    for id in CHROMIUM_EXTENSION_IDS {
        assert_eq!(id.len(), 32, "{id}");
        assert!(id.chars().all(|c| ('a'..='p').contains(&c)), "{id}");
    }
}

/// Reads the default value of `HKCU\<key>`, if the key exists.
fn registry_default(key: &str) -> Option<String> {
    let output = std::process::Command::new("reg")
        .args(["query", &format!(r"HKCU\{key}"), "/ve"])
        .output()
        .unwrap();
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.lines()
        .find(|l| l.contains("REG_SZ"))
        .and_then(|l| l.split("REG_SZ").nth(1))
        .map(|v| v.trim().to_string())
}

#[test]
fn registering_writes_manifests_and_keys_and_unregistering_removes_them() {
    let dir = tempfile::tempdir().unwrap();
    let host = dir.path().join("omnidm-host.exe");
    std::fs::write(&host, b"").unwrap();
    // A private registry area, so the real browser registration is never touched.
    let root = format!(r"Software\OmniDm-Test-{:016x}", fastrand::u64(..));
    let keys: Vec<String> = BROWSER_KEYS
        .iter()
        .map(|(_, key)| format!(r"{root}\{key}"))
        .collect();
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();

    register(&host, &key_refs).unwrap();

    let chromium = dir.path().join("host-chromium.json");
    let firefox = dir.path().join("host-firefox.json");
    assert!(chromium.exists() && firefox.exists());
    for (key, (browser, _)) in keys.iter().zip(BROWSER_KEYS) {
        let value = registry_default(&format!(r"{key}\{HOST_NAME}")).expect(browser);
        let expected = if *browser == "firefox" {
            &firefox
        } else {
            &chromium
        };
        assert_eq!(Path::new(&value), expected, "{browser}");
    }

    unregister(&host, &key_refs).unwrap();

    for key in &keys {
        assert_eq!(registry_default(&format!(r"{key}\{HOST_NAME}")), None);
    }
    assert!(!chromium.exists() && !firefox.exists());
    let _ = std::process::Command::new("reg")
        .args(["delete", &format!(r"HKCU\{root}"), "/f"])
        .output();
}

#[tokio::test]
async fn a_stopped_app_is_started_and_then_connected() {
    let name = format!(r"\\.\pipe\OmniDm.test.{:016x}", fastrand::u64(..));
    let started_name = name.clone();
    // "Starting the app" here means opening its pipe a little later, as the real app does.
    let start = move || {
        let name = started_name.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut listener = PipeListener::bind(&name).unwrap();
            let _pipe = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        Ok(())
    };

    let pipe = connect_app(&name, &[], start, Duration::from_secs(5)).await;

    assert!(pipe.is_ok(), "{:?}", pipe.err());
}

#[tokio::test]
async fn an_app_that_never_starts_is_given_up_on() {
    let name = format!(r"\\.\pipe\OmniDm.test.{:016x}", fastrand::u64(..));
    let started = std::time::Instant::now();

    let result = connect_app(&name, &[], || Ok(()), Duration::from_millis(500)).await;

    assert!(
        matches!(result, Err(PipeError::NotRunning)),
        "{:?}",
        result.err()
    );
    assert!(started.elapsed() >= Duration::from_millis(500));
}
