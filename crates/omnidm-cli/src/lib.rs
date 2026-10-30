// SPDX-License-Identifier: GPL-3.0-or-later

//! The `omnidm` command: downloads through the running OmniDM app, or on its own (with the
//! engine and the store) when the app isn't running.

mod remote;
pub use remote::{get_via_app, list_via_app, request};

use std::fmt;
use std::path::{Path, PathBuf};

use omnidm_core::RampPolicy;
use omnidm_core::engine::{EngineConfig, Event, Method, Request};
use omnidm_net::cookies::{CookieJar, Url};
use omnidm_net::naming::sanitize_file_name;
use omnidm_service::job::{Job, JobError, JobOutcome, run_job};
pub use omnidm_service::job::{PART_SUFFIX, unique_path};
use omnidm_store::checkpoint::resume_from;
use omnidm_store::{NewDownload, Store};
use tokio_util::sync::CancellationToken;

/// What `omnidm get` should download, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetOptions {
    pub url: String,
    /// File name to save as; taken from the URL when absent.
    pub output: Option<String>,
    pub dir: PathBuf,
    pub connections: u32,
    /// Extra request headers. A `Cookie` header is used like `cookies` and never saved.
    pub headers: Vec<(String, String)>,
    /// `name=value; name2=value2` cookies for the URL's site. Never saved, so a resumed
    /// download needs them again.
    pub cookies: Vec<String>,
    /// Form data: the download is fetched with a `POST` carrying it.
    pub data: Option<String>,
    /// Put it in this queue of the running app instead of downloading now.
    pub queue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetResult {
    Completed(PathBuf),
    /// Stopped by the user; running the same command again resumes.
    Paused,
    /// Added to a queue of the app, with this download id.
    Queued(i64),
}

#[derive(Debug)]
pub enum CliError {
    Store(omnidm_store::StoreError),
    Engine(omnidm_core::engine::EngineError),
    Io(std::io::Error),
    Http(String),
    /// Reported by the running OmniDM app.
    App(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Engine(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Http(e) => write!(f, "{e}"),
            Self::App(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<omnidm_store::StoreError> for CliError {
    fn from(e: omnidm_store::StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Downloads `opts.url`, resuming an unfinished download of the same URL into the same
/// folder if there is one. `progress` is called with every engine event.
pub async fn get(
    opts: GetOptions,
    data_dir: &Path,
    cancel: CancellationToken,
    mut progress: impl FnMut(&Event),
) -> Result<GetResult, CliError> {
    if opts.queue.is_some() {
        return Err(CliError::App(
            "queues belong to the OmniDM app; start it first".into(),
        ));
    }
    std::fs::create_dir_all(data_dir)?;
    std::fs::create_dir_all(&opts.dir)?;
    let dir = std::path::absolute(&opts.dir)?;
    let dir_text = dir.to_string_lossy().into_owned();
    let mut store = Store::open(&data_dir.join(DATABASE_FILE))?;
    let (cookie_headers, mut saved_headers): (Vec<_>, Vec<_>) = opts
        .headers
        .iter()
        .cloned()
        .partition(|(name, _)| name.eq_ignore_ascii_case("cookie"));
    let has_type = saved_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    if opts.data.is_some() && !has_type {
        saved_headers.push((
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
    }
    let cookies = CookieJar::new();
    if let Ok(url) = Url::parse(&opts.url) {
        let given = opts
            .cookies
            .iter()
            .chain(cookie_headers.iter().map(|(_, v)| v));
        for pairs in given {
            cookies.add_pairs(&url, pairs);
        }
    }

    // A download's name is the user's choice, or the one picked when it was first probed.
    let (id, known_name, headers, method, body, resume) =
        match store.find_unfinished(&opts.url, &dir_text)? {
            Some(record) => {
                let segments = store.load_segments(record.id)?;
                let resume = resume_from(&record, segments);
                (
                    record.id,
                    record.file_name,
                    record.headers,
                    record.method,
                    record.body,
                    resume,
                )
            }
            None => {
                let name = opts.output.as_deref().map(sanitize_file_name);
                let (method, body) = match &opts.data {
                    Some(data) => (Method::Post, data.as_bytes().to_vec()),
                    None => (Method::Get, Vec::new()),
                };
                let id = store.insert_download(&NewDownload {
                    url: opts.url.clone(),
                    save_dir: dir_text.clone(),
                    file_name: name.clone(),
                    referrer: None,
                    user_agent: None,
                    headers: saved_headers.clone(),
                    method,
                    body: body.clone(),
                })?;
                (id, name, saved_headers, method, body, None)
            }
        };

    let client = omnidm_net::download_client().map_err(|e| CliError::Http(e.to_string()))?;
    let request = Request {
        url: opts.url.clone(),
        method,
        body,
        headers,
        cookies,
    };
    let config = EngineConfig {
        ramp: RampPolicy {
            max_connections: opts.connections.clamp(1, omnidm_core::ramp::HARD_CAP),
            ..RampPolicy::default()
        },
        ..EngineConfig::default()
    };
    // A download whose name is known resumes; otherwise it starts with a probe.
    let resume = resume.filter(|_| known_name.is_some());
    drop(store);

    let job = Job {
        database: data_dir.join(DATABASE_FILE),
        id,
        dir,
        file_name: known_name,
        probe: None,
        resume,
        client,
        request,
        config,
        cancel,
    };
    match run_job(job, &mut progress).await {
        Ok(JobOutcome::Completed(path)) => Ok(GetResult::Completed(path)),
        Ok(JobOutcome::Paused) => Ok(GetResult::Paused),
        Err(JobError::Engine(e)) => Err(CliError::Engine(e)),
        Err(JobError::Store(e)) => Err(CliError::Store(e)),
        Err(JobError::Io(e)) => Err(CliError::Io(e)),
    }
}

/// Name of the database file inside the data folder.
pub const DATABASE_FILE: &str = "omnidm.db";

/// Where OmniDM keeps its database: `OMNIDM_DATA_DIR` if set, else `%APPDATA%\OmniDm`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OMNIDM_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("OmniDm")
}

/// Parses a `-H "Name: value"` argument.
pub fn parse_header(text: &str) -> Result<(String, String), String> {
    let (name, value) = text
        .split_once(':')
        .ok_or_else(|| format!("expected \"Name: value\", got {text:?}"))?;
    let name = name.trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return Err(format!("invalid header name in {text:?}"));
    }
    Ok((name.to_string(), value.trim().to_string()))
}

/// Human-readable size with binary units: `512 B`, `1.5 KiB`, `3.0 GiB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// One status line: percentage, sizes, speed, connections and time left.
pub fn progress_line(downloaded: u64, total: Option<u64>, speed: u64, connections: u32) -> String {
    let speed_text = format!("{}/s", format_bytes(speed));
    let Some(total) = total else {
        return format!(
            "{}  {speed_text}  {connections} conn",
            format_bytes(downloaded)
        );
    };
    let percent = if total == 0 {
        100.0
    } else {
        downloaded as f64 * 100.0 / total as f64
    };
    let eta = match total.saturating_sub(downloaded).checked_div(speed) {
        Some(secs) => {
            let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
            if h > 0 {
                format!("{h}:{m:02}:{s:02}")
            } else {
                format!("{m}:{s:02}")
            }
        }
        None => "--".into(),
    };
    format!(
        "{percent:.1}%  {} / {}  {speed_text}  {connections} conn  ETA {eta}",
        format_bytes(downloaded),
        format_bytes(total)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const MIB: u64 = 1024 * 1024;

    fn taken(names: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = names.iter().map(|n| Path::new("dl").join(n)).collect();
        move |p| set.contains(p)
    }

    #[test]
    fn a_free_name_is_used_as_is() {
        assert_eq!(
            unique_path(Path::new("dl"), "a.zip", taken(&[])),
            Path::new("dl").join("a.zip")
        );
    }

    #[test]
    fn a_taken_name_gets_the_next_free_number() {
        assert_eq!(
            unique_path(Path::new("dl"), "a.zip", taken(&["a.zip"])),
            Path::new("dl").join("a (1).zip")
        );
        assert_eq!(
            unique_path(Path::new("dl"), "a.zip", taken(&["a.zip", "a (1).zip"])),
            Path::new("dl").join("a (2).zip")
        );
    }

    #[test]
    fn numbering_goes_before_the_last_extension_only() {
        assert_eq!(
            unique_path(Path::new("dl"), "data.tar.gz", taken(&["data.tar.gz"])),
            Path::new("dl").join("data.tar (1).gz")
        );
        assert_eq!(
            unique_path(Path::new("dl"), "README", taken(&["README"])),
            Path::new("dl").join("README (1)")
        );
    }

    #[test]
    fn headers_are_split_at_the_first_colon_and_trimmed() {
        assert_eq!(
            parse_header("Referer: https://e.com/page"),
            Ok(("Referer".into(), "https://e.com/page".into()))
        );
        assert_eq!(
            parse_header("  X-A :  b c "),
            Ok(("X-A".into(), "b c".into()))
        );
    }

    #[test]
    fn malformed_headers_are_rejected() {
        for bad in ["NoColon", ": value", "   : x", "Bad Name: x"] {
            assert!(parse_header(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn sizes_use_binary_units() {
        let cases = [
            (0, "0 B"),
            (1023, "1023 B"),
            (1536, "1.5 KiB"),
            (5 * MIB, "5.0 MiB"),
            (3 * 1024 * MIB + 512 * MIB, "3.5 GiB"),
        ];
        for (bytes, want) in cases {
            assert_eq!(format_bytes(bytes), want, "{bytes}");
        }
    }

    #[test]
    fn progress_line_shows_percentage_speed_and_time_left() {
        assert_eq!(
            progress_line(512 * MIB, Some(1024 * MIB), 10 * MIB, 8),
            "50.0%  512.0 MiB / 1.0 GiB  10.0 MiB/s  8 conn  ETA 0:51"
        );
    }

    #[test]
    fn progress_line_formats_hours() {
        // 3,700 s left: 1 h 1 min 40 s.
        assert_eq!(
            progress_line(0, Some(3700 * MIB), MIB, 1),
            "0.0%  0 B / 3.6 GiB  1.0 MiB/s  1 conn  ETA 1:01:40"
        );
    }

    #[test]
    fn progress_line_without_speed_or_size() {
        assert_eq!(
            progress_line(MIB, Some(2 * MIB), 0, 0),
            "50.0%  1.0 MiB / 2.0 MiB  0 B/s  0 conn  ETA --"
        );
        assert_eq!(
            progress_line(12 * MIB, None, MIB, 1),
            "12.0 MiB  1.0 MiB/s  1 conn"
        );
    }
}
