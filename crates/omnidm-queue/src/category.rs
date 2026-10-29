// SPDX-License-Identifier: GPL-3.0-or-later

//! Which category (and so which folder) a download belongs to.
//!
//! A category matches a file by its extension, its MIME type or the site it comes from;
//! any one is enough. Categories are tried in the user's order and the first match wins,
//! so "everything from github.com" placed above "Compressed" catches GitHub's zips too.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryRule {
    /// File extensions without the dot: `zip`, `tar.gz`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// MIME types: `application/pdf`, or `video/*` for every video type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mime_types: Vec<String>,
    /// Sites, each with its subdomains: `github.com` also matches `api.github.com`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
}

/// What is known about a download when its category is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts<'a> {
    pub file_name: &'a str,
    /// The server's `Content-Type`, parameters allowed (`text/plain; charset=utf-8`).
    pub mime: Option<&'a str>,
    /// The host the file comes from.
    pub host: Option<&'a str>,
}

impl CategoryRule {
    pub fn matches(&self, file: &FileFacts<'_>) -> bool {
        let name = file.file_name.to_lowercase();
        let by_extension = self.extensions.iter().any(|ext| {
            let ext = ext.trim().trim_start_matches('.').to_lowercase();
            !ext.is_empty() && name.ends_with(&format!(".{ext}"))
        });
        let by_mime = file
            .mime
            .is_some_and(|mime| self.mime_types.iter().any(|m| mime_matches(mime, m)));
        let by_host = file
            .host
            .is_some_and(|host| self.hosts.iter().any(|site| host_matches(host, site)));
        by_extension || by_mime || by_host
    }

    /// True if the rule can never match anything.
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty() && self.mime_types.is_empty() && self.hosts.is_empty()
    }
}

/// The first of `categories` whose rule matches `file`.
pub fn pick<'a, T>(
    categories: impl IntoIterator<Item = (T, &'a CategoryRule)>,
    file: &FileFacts<'_>,
) -> Option<T> {
    categories
        .into_iter()
        .find(|(_, rule)| rule.matches(file))
        .map(|(id, _)| id)
}

/// `host` is `site` or one of its subdomains.
pub fn host_matches(host: &str, site: &str) -> bool {
    let wanted = site.trim().trim_start_matches('.').to_lowercase();
    let actual = host.to_lowercase();
    !wanted.is_empty() && (actual == wanted || actual.ends_with(&format!(".{wanted}")))
}

fn mime_matches(mime: &str, pattern: &str) -> bool {
    let mime = mime.split(';').next().unwrap_or("").trim().to_lowercase();
    let pattern = pattern.trim().to_lowercase();
    match pattern.strip_suffix("/*") {
        Some(kind) => mime
            .strip_prefix(kind)
            .is_some_and(|rest| rest.starts_with('/')),
        None => !pattern.is_empty() && mime == pattern,
    }
}

/// The categories a new database starts with, in order.
pub fn default_categories() -> Vec<(&'static str, CategoryRule)> {
    let rule = |extensions: &[&str], mime_types: &[&str]| CategoryRule {
        extensions: extensions.iter().map(|e| e.to_string()).collect(),
        mime_types: mime_types.iter().map(|m| m.to_string()).collect(),
        hosts: Vec::new(),
    };
    vec![
        (
            "Compressed",
            rule(
                &[
                    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "cab",
                ],
                &["application/zip", "application/x-7z-compressed"],
            ),
        ),
        (
            "Documents",
            rule(
                &[
                    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf",
                    "txt", "csv", "epub",
                ],
                &["application/pdf"],
            ),
        ),
        (
            "Music",
            rule(
                &["mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma"],
                &["audio/*"],
            ),
        ),
        (
            "Pictures",
            rule(
                &[
                    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "avif", "tif",
                    "tiff",
                ],
                &["image/*"],
            ),
        ),
        (
            "Programs",
            rule(
                &[
                    "exe", "msi", "msix", "appx", "apk", "dmg", "deb", "rpm", "iso", "img",
                ],
                &[],
            ),
        ),
        (
            "Video",
            rule(
                &["mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v"],
                &["video/*"],
            ),
        ),
    ]
}
