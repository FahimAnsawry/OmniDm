// SPDX-License-Identifier: GPL-3.0-or-later

//! `omnidm get` end to end: test server → engine → store → finished file.

use std::path::Path;
use std::time::Duration;

use omnidm_cli::{CliError, GetOptions, GetResult, get};
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
