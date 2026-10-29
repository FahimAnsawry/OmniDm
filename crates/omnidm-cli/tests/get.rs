// SPDX-License-Identifier: GPL-3.0-or-later

//! `omnidm get` end to end: test server → engine → store → finished file.

use std::path::Path;
use std::time::Duration;

use omnidm_cli::{CliError, GetOptions, GetResult, get};
use omnidm_core::engine::Method;
use omnidm_store::{Status, Store};
use omnidm_testserver::{FileSpec, TestServer};
use tokio_util::sync::CancellationToken;

const MIB: u64 = 1024 * 1024;

struct Env {
    server: TestServer,
    downloads: tempfile::TempDir,
    data: tempfile::TempDir,
}

async fn env_with(path: &str, spec: FileSpec) -> Env {
    let server = TestServer::start().await.unwrap();
    server.set_file(path, spec);
    Env {
        server,
        downloads: tempfile::tempdir().unwrap(),
        data: tempfile::tempdir().unwrap(),
    }
}

impl Env {
    fn options(&self, path: &str) -> GetOptions {
        GetOptions {
            url: self.server.url(path),
            output: None,
            dir: self.downloads.path().to_path_buf(),
            connections: 4,
            headers: vec![],
            cookies: vec![],
            data: None,
            queue: None,
        }
    }

    async fn get(&self, opts: GetOptions) -> Result<GetResult, CliError> {
        self.get_cancelled_after(opts, None).await
    }

    async fn get_cancelled_after(
        &self,
        opts: GetOptions,
        after: Option<Duration>,
    ) -> Result<GetResult, CliError> {
        let cancel = CancellationToken::new();
        if let Some(after) = after {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                cancel.cancel();
            });
        }
        tokio::time::timeout(
            Duration::from_secs(30),
            get(opts, self.data.path(), cancel, |_| {}),
        )
        .await
        .expect("get did not finish within 30 s")
    }

    fn store(&self) -> Store {
        Store::open(&self.data.path().join("omnidm.db")).unwrap()
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.downloads.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn assert_content(path: &Path, spec: &FileSpec) {
    let data = std::fs::read(path).unwrap();
    assert!(
        data == spec.bytes(0, spec.size as usize),
        "{} differs from the served file",
        path.display()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_downloaded_and_named_after_the_url() {
    let spec = FileSpec::new(2 * MIB).etag("v1");
    let env = env_with("/files/report.bin", spec.clone()).await;

    let result = env.get(env.options("/files/report.bin")).await.unwrap();

    let path = env.downloads.path().join("report.bin");
    assert_eq!(result, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
    assert_eq!(
        env.files(),
        ["report.bin"],
        "no part file may be left behind"
    );
    let downloads = env.store().list_downloads().unwrap();
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].status, Status::Completed);
    assert_eq!(downloads[0].file_name.as_deref(), Some("report.bin"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_output_option_names_the_file() {
    let spec = FileSpec::new(MIB).etag("v1");
    let env = env_with("/f.bin", spec.clone()).await;
    let opts = GetOptions {
        output: Some("chosen name.dat".into()),
        ..env.options("/f.bin")
    };

    let result = env.get(opts).await.unwrap();

    let path = env.downloads.path().join("chosen name.dat");
    assert_eq!(result, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_name_the_server_gives_is_used() {
    let spec = FileSpec::new(MIB)
        .etag("v1")
        .content_type("application/zip")
        .content_disposition("attachment; filename=\"Holiday Photos.zip\"");
    let env = env_with("/download.php", spec.clone()).await;

    let result = env.get(env.options("/download.php")).await.unwrap();

    let path = env.downloads.path().join("Holiday Photos.zip");
    assert_eq!(result, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
    let downloads = env.store().list_downloads().unwrap();
    assert_eq!(
        downloads[0].file_name.as_deref(),
        Some("Holiday Photos.zip")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_extension_comes_from_the_file_type() {
    let spec = FileSpec::new(MIB)
        .etag("v1")
        .content_type("application/pdf");
    let env = env_with("/files/report", spec).await;

    let result = env.get(env.options("/files/report")).await.unwrap();

    assert_eq!(
        result,
        GetResult::Completed(env.downloads.path().join("report.pdf"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_output_option_beats_the_server_name() {
    let spec = FileSpec::new(MIB)
        .etag("v1")
        .content_disposition("attachment; filename=\"server.zip\"");
    let env = env_with("/f.bin", spec).await;
    let opts = GetOptions {
        output: Some("mine.zip".into()),
        ..env.options("/f.bin")
    };

    let result = env.get(opts).await.unwrap();

    assert_eq!(
        result,
        GetResult::Completed(env.downloads.path().join("mine.zip"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_download_keeps_the_server_name_when_resumed() {
    let spec = FileSpec::new(4 * MIB)
        .etag("v1")
        .content_disposition("attachment; filename=\"real.bin\"");
    let env = env_with("/dl.php", spec.clone().throttle(2 * MIB)).await;

    let first = env
        .get_cancelled_after(env.options("/dl.php"), Some(Duration::from_millis(600)))
        .await
        .unwrap();

    assert_eq!(first, GetResult::Paused);
    assert_eq!(env.files(), ["real.bin.omnidm-part"]);
    let requests_before = env.server.requests().len();

    env.server.set_file("/dl.php", spec.clone());
    let second = env.get(env.options("/dl.php")).await.unwrap();

    let path = env.downloads.path().join("real.bin");
    assert_eq!(second, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
    let restarted = env.server.requests()[requests_before..]
        .iter()
        .any(|r| r.range.as_deref() == Some("bytes=0-"));
    assert!(
        !restarted,
        "the second run started over instead of resuming"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_file_is_never_overwritten() {
    let spec = FileSpec::new(MIB).etag("v1");
    let env = env_with("/report.bin", spec.clone()).await;
    let existing = env.downloads.path().join("report.bin");
    std::fs::write(&existing, b"my own file").unwrap();

    let result = env.get(env.options("/report.bin")).await.unwrap();

    let path = env.downloads.path().join("report (1).bin");
    assert_eq!(result, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
    assert_eq!(std::fs::read(&existing).unwrap(), b"my own file");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_download_resumes_on_the_next_get() {
    let spec = FileSpec::new(4 * MIB).etag("v1");
    let env = env_with("/big.bin", spec.clone().throttle(2 * MIB)).await;

    let first = env
        .get_cancelled_after(env.options("/big.bin"), Some(Duration::from_millis(600)))
        .await
        .unwrap();

    assert_eq!(first, GetResult::Paused);
    assert_eq!(env.files(), ["big.bin.omnidm-part"]);
    let paused = env.store().list_downloads().unwrap();
    assert_eq!(paused[0].status, Status::Paused);
    let requests_before = env.server.requests().len();

    env.server.set_file("/big.bin", spec.clone());
    let second = env.get(env.options("/big.bin")).await.unwrap();

    let path = env.downloads.path().join("big.bin");
    assert_eq!(second, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
    let resumed_ranges: Vec<String> = env.server.requests()[requests_before..]
        .iter()
        .filter_map(|r| r.range.clone())
        .collect();
    assert!(
        !resumed_ranges.iter().any(|r| r == "bytes=0-"),
        "the second run started over instead of resuming: {resumed_ranges:?}"
    );
    let downloads = env.store().list_downloads().unwrap();
    assert_eq!(downloads.len(), 1, "resuming reuses the same download");
    assert_eq!(downloads[0].status, Status::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_download_is_recorded_with_its_error() {
    let env = env_with("/other", FileSpec::new(1)).await;

    let result = env.get(env.options("/missing.bin")).await;

    assert!(matches!(result, Err(CliError::Engine(_))), "{result:?}");
    let downloads = env.store().list_downloads().unwrap();
    assert_eq!(downloads[0].status, Status::Failed);
    assert!(
        downloads[0].error.as_deref().unwrap_or("").contains("404"),
        "{:?}",
        downloads[0].error
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn extra_headers_are_sent_and_remembered() {
    let spec = FileSpec::new(MIB).etag("v1");
    let env = env_with("/f.bin", spec).await;
    let opts = GetOptions {
        headers: vec![("Referer".into(), "https://example.com/page".into())],
        ..env.options("/f.bin")
    };

    env.get(opts).await.unwrap();

    let downloads = env.store().list_downloads().unwrap();
    assert_eq!(
        downloads[0].headers,
        vec![(
            "Referer".to_string(),
            "https://example.com/page".to_string()
        )]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cookies_given_with_b_open_a_protected_file() {
    let spec = FileSpec::new(2 * MIB)
        .etag("v1")
        .require_cookie("session=abc");
    let env = env_with("/private.bin", spec.clone()).await;
    let opts = GetOptions {
        cookies: vec!["theme=dark; session=abc".into()],
        ..env.options("/private.bin")
    };

    let result = env.get(opts).await.unwrap();

    let path = env.downloads.path().join("private.bin");
    assert_eq!(result, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cookie_header_works_like_b_and_is_never_saved() {
    let spec = FileSpec::new(MIB).etag("v1").require_cookie("session=abc");
    let env = env_with("/private.bin", spec).await;
    let opts = GetOptions {
        headers: vec![
            ("Referer".into(), "https://example.com/".into()),
            ("cookie".into(), "session=abc".into()),
        ],
        ..env.options("/private.bin")
    };

    let result = env.get(opts).await;

    assert!(matches!(result, Ok(GetResult::Completed(_))), "{result:?}");
    let downloads = env.store().list_downloads().unwrap();
    assert_eq!(
        downloads[0].headers,
        vec![("Referer".to_string(), "https://example.com/".to_string())],
        "cookies must not be stored in the database"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn data_sends_a_post_and_is_remembered() {
    let spec = FileSpec::new(2 * MIB).etag("v1").post_only("id=7&agree=1");
    let env = env_with("/dl", spec.clone()).await;
    let opts = GetOptions {
        data: Some("id=7&agree=1".into()),
        queue: None,
        output: Some("file.bin".into()),
        ..env.options("/dl")
    };

    let result = env.get(opts).await.unwrap();

    let path = env.downloads.path().join("file.bin");
    assert_eq!(result, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
    let first = &env.server.requests()[0];
    assert_eq!(
        first.header("content-type"),
        Some("application/x-www-form-urlencoded"),
        "form data gets the usual type unless one is given"
    );
    let saved = &env.store().list_downloads().unwrap()[0];
    assert_eq!(saved.method, Method::Post);
    assert_eq!(saved.body, b"id=7&agree=1");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_link_leaves_the_download_paused_to_continue_later() {
    let spec = FileSpec::new(4 * MIB).etag("v1");
    let env = env_with("/f.bin", spec.clone().throttle(4 * MIB).expire_after(1)).await;

    let first = env.get(env.options("/f.bin")).await;

    assert!(
        matches!(
            first,
            Err(CliError::Engine(
                omnidm_core::engine::EngineError::LinkExpired
            ))
        ),
        "{first:?}"
    );
    assert_eq!(
        env.store().list_downloads().unwrap()[0].status,
        Status::Paused
    );
    assert_eq!(env.files(), ["f.bin.omnidm-part"]);

    // The link works again (say, with fresh cookies): the same command continues.
    let before = env.server.requests().len();
    env.server.set_file("/f.bin", spec.clone());
    let second = env.get(env.options("/f.bin")).await.unwrap();

    let path = env.downloads.path().join("f.bin");
    assert_eq!(second, GetResult::Completed(path.clone()));
    assert_content(&path, &spec);
    let restarted = env.server.requests()[before..]
        .iter()
        .any(|r| r.range.as_deref() == Some("bytes=0-"));
    assert!(!restarted, "continued instead of starting over");
}
