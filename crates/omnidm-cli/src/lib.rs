// SPDX-License-Identifier: GPL-3.0-or-later

//! The `omnidm` command: downloads in the terminal using the engine and the store.

use std::fmt;
use std::path::{Path, PathBuf};

use omnidm_core::RampPolicy;
use omnidm_core::engine::{Download, EngineConfig, Event, Outcome, Request, download};
use omnidm_net::naming::{file_name_from_url, sanitize_file_name};
use omnidm_store::checkpoint::{StoreCheckpointer, resume_from};
use omnidm_store::{NewDownload, Status, Store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// What `omnidm get` should download, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetOptions {
    pub url: String,
    /// File name to save as; taken from the URL when absent.
    pub output: Option<String>,
    pub dir: PathBuf,
    pub connections: u32,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetResult {
    Completed(PathBuf),
    /// Stopped by the user; running the same command again resumes.
    Paused,
}

#[derive(Debug)]
pub enum CliError {
    Store(omnidm_store::StoreError),
    Engine(omnidm_core::engine::EngineError),
    Io(std::io::Error),
    Http(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Engine(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Http(e) => write!(f, "{e}"),
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
    std::fs::create_dir_all(data_dir)?;
    std::fs::create_dir_all(&opts.dir)?;
    let dir = std::path::absolute(&opts.dir)?;
    let dir_text = dir.to_string_lossy().into_owned();
    let mut store = Store::open(&data_dir.join(DATABASE_FILE))?;

    let (id, file_name, headers, resume) = match store.find_unfinished(&opts.url, &dir_text)? {
        Some(record) => {
            let segments = store.load_segments(record.id)?;
            let name = record
                .file_name
                .clone()
                .unwrap_or_else(|| file_name_from_url(&opts.url));
            let resume = resume_from(&record, segments);
            (record.id, name, record.headers, resume)
        }
        None => {
            let name = match &opts.output {
                Some(name) => sanitize_file_name(name),
                None => file_name_from_url(&opts.url),
            };
            let id = store.insert_download(&NewDownload {
                url: opts.url.clone(),
                save_dir: dir_text.clone(),
                file_name: Some(name.clone()),
                referrer: None,
                user_agent: None,
                headers: opts.headers.clone(),
            })?;
            (id, name, opts.headers.clone(), None)
        }
    };

    let part = dir.join(format!("{file_name}{PART_SUFFIX}"));
    let client = omnidm_net::download_client().map_err(|e| CliError::Http(e.to_string()))?;
    let request = Request {
        url: opts.url.clone(),
        headers,
    };
    let config = EngineConfig {
        ramp: RampPolicy {
            max_connections: opts.connections.clamp(1, omnidm_core::ramp::HARD_CAP),
            ..RampPolicy::default()
        },
        ..EngineConfig::default()
    };
    store.set_status(id, Status::Running, None)?;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let result = {
        let mut checkpointer = StoreCheckpointer {
            store: &mut store,
            id,
            file_name: file_name.clone(),
        };
        let job = download(
            Download {
                client: &client,
                request: &request,
                part_path: &part,
                resume,
                config: &config,
                events: Some(tx),
                cancel,
            },
            &mut checkpointer,
        );
        tokio::pin!(job);
        loop {
            tokio::select! {
                result = &mut job => break result,
                Some(event) = rx.recv() => progress(&event),
            }
        }
    };
    while let Ok(event) = rx.try_recv() {
        progress(&event);
    }

    match result {
        Ok(Outcome::Completed { .. }) => {
            let target = unique_path(&dir, &file_name, Path::exists);
            std::fs::rename(&part, &target)?;
            store.set_status(id, Status::Completed, None)?;
            Ok(GetResult::Completed(target))
        }
        Ok(Outcome::Paused) => {
            store.set_status(id, Status::Paused, None)?;
            Ok(GetResult::Paused)
        }
        Err(e) => {
            store.set_status(id, Status::Failed, Some(&e.to_string()))?;
            Err(CliError::Engine(e))
        }
    }
}

/// Name of the database file inside the data folder.
pub const DATABASE_FILE: &str = "omnidm.db";

/// Appended to a file's name while it is being downloaded.
pub const PART_SUFFIX: &str = ".omnidm-part";

/// Where OmniDm keeps its database: `OMNIDM_DATA_DIR` if set, else `%APPDATA%\OmniDm`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OMNIDM_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("OmniDm")
}

/// `dir/name`, or `dir/name (1).ext`, `dir/name (2).ext`… if that is taken.
pub fn unique_path(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    (1u32..)
        .map(|n| match extension {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|candidate| !exists(candidate))
        .expect("some number is free")
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
