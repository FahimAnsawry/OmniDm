// SPDX-License-Identifier: GPL-3.0-or-later

//! File names: taken from the server's `Content-Disposition` or the URL, given an
//! extension from the file type when they lack one, and made safe to use on Windows.

/// Used when nothing better is available.
pub const FALLBACK_NAME: &str = "download";

/// Longest file name kept, in characters (Windows allows 255 UTF-16 units).
const MAX_NAME_CHARS: usize = 200;

/// Extensions of pages that generate downloads; they say nothing about the file itself.
const SCRIPT_EXTENSIONS: [&str; 10] = [
    "php", "asp", "aspx", "jsp", "jspx", "cgi", "pl", "do", "action", "cfm",
];

/// The name to save a download as, from the server's `Content-Disposition` if it gives
/// one, else from the URL (§5.8). When the name has no extension, or only a script's
/// (`download.php`), the extension for `content_type` is used.
pub fn choose_file_name(
    content_disposition: Option<&str>,
    url: &str,
    content_type: Option<&str>,
) -> String {
    let from_server = content_disposition
        .and_then(file_name_from_disposition)
        .map(|name| {
            // RFC 6266: only the last path component may be used.
            let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
            sanitize_file_name(base)
        })
        .filter(|name| name != FALLBACK_NAME);
    let name = from_server.unwrap_or_else(|| file_name_from_url(url));
    let Some(extension) = content_type.and_then(extension_for_mime) else {
        return name;
    };
    let named = match name.rsplit_once('.') {
        Some((stem, current))
            if !stem.is_empty() && !current.bytes().all(|b| b.is_ascii_digit()) =>
        {
            if SCRIPT_EXTENSIONS.contains(&current.to_ascii_lowercase().as_str()) {
                format!("{stem}.{extension}")
            } else {
                name
            }
        }
        _ => format!("{name}.{extension}"),
    };
    sanitize_file_name(&named)
}

/// The file name in a `Content-Disposition` header, unsanitised. `filename*` (RFC 5987,
/// UTF-8 or ISO-8859-1) wins over `filename`.
pub fn file_name_from_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    let (_, params) = value.split_once(';')?;
    let mut rest = params;
    while !rest.is_empty() {
        rest = rest.trim_start_matches([';', ' ', '\t']);
        let name_end = rest.find(['=', ';']).unwrap_or(rest.len());
        let name = rest[..name_end].trim().to_ascii_lowercase();
        rest = &rest[name_end..];
        let Some(after_eq) = rest.strip_prefix('=') else {
            continue;
        };
        let (param, remaining) = parameter_value(after_eq.trim_start());
        rest = remaining;
        match name.as_str() {
            "filename" => plain = Some(param),
            "filename*" => extended = decode_extended(&param).or(extended),
            _ => {}
        }
    }
    extended.or(plain).filter(|name| !name.trim().is_empty())
}

/// Reads a quoted string or a token; returns it and what follows. In a quoted string `\`
/// escapes only `"` and `\`: servers often send Windows paths with unescaped backslashes.
fn parameter_value(text: &str) -> (String, &str) {
    let Some(quoted) = text.strip_prefix('"') else {
        let end = text.find(';').unwrap_or(text.len());
        return (text[..end].trim().to_string(), &text[end..]);
    };
    let mut value = String::new();
    let mut chars = quoted.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some(&(_, next @ ('"' | '\\'))) => {
                    value.push(next);
                    chars.next();
                }
                _ => value.push('\\'),
            },
            '"' => return (value, &quoted[i + 1..]),
            _ => value.push(c),
        }
    }
    (value, "")
}

/// Decodes an RFC 5987 value, `charset'language'percent-encoded`.
fn decode_extended(value: &str) -> Option<String> {
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next()?.to_ascii_lowercase();
    let _language = parts.next()?;
    let bytes = percent_decode_bytes(parts.next()?);
    match charset.as_str() {
        "utf-8" => Some(String::from_utf8_lossy(&bytes).into_owned()),
        "iso-8859-1" => Some(bytes.iter().map(|&b| char::from(b)).collect()),
        _ => None,
    }
}

/// The usual extension for a MIME type, ignoring parameters such as `charset`.
/// `None` for generic types like `application/octet-stream`.
pub fn extension_for_mime(content_type: &str) -> Option<&'static str> {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let extension = match essence.as_str() {
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/pdf" => "pdf",
        "application/x-msdownload"
        | "application/vnd.microsoft.portable-executable"
        | "application/x-msdos-program" => "exe",
        "application/x-msi" | "application/x-ms-installer" => "msi",
        "application/x-7z-compressed" => "7z",
        "application/vnd.rar" | "application/x-rar-compressed" => "rar",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-tar" => "tar",
        "application/x-xz" => "xz",
        "application/x-bzip2" => "bz2",
        "application/x-iso9660-image" => "iso",
        "application/vnd.android.package-archive" => "apk",
        "application/x-bittorrent" => "torrent",
        "application/epub+zip" => "epub",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/msword" => "doc",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/html" => "html",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/x-msvideo" => "avi",
        "video/quicktime" => "mov",
        "video/mp2t" => "ts",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" => "flac",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => return None,
    };
    Some(extension)
}

/// The last path segment of `url`, percent-decoded and sanitised.
pub fn file_name_from_url(url: &str) -> String {
    let segment = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments()?.next_back().map(str::to_string))
        .unwrap_or_default();
    sanitize_file_name(&percent_decode(&segment))
}

/// Makes `name` a valid Windows file name: removes path separators, reserved and control
/// characters, trailing dots and spaces; avoids reserved device names; limits the length.
pub fn sanitize_file_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut name = trim_name(&replaced).to_string();
    if name.is_empty() {
        return FALLBACK_NAME.into();
    }
    if name.chars().count() > MAX_NAME_CHARS {
        name = shorten(&name);
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if is_reserved_device_name(&stem) {
        name.insert(0, '_');
    }
    name
}

/// Windows ignores trailing dots and spaces, and leading spaces are easy to miss.
fn trim_name(name: &str) -> &str {
    name.trim_start_matches(' ').trim_end_matches(['.', ' '])
}

/// Cuts the stem so the whole name fits, keeping a short extension.
fn shorten(name: &str) -> String {
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty() && ext.chars().count() <= 16);
    let keep = MAX_NAME_CHARS - extension.map_or(0, |ext| ext.chars().count() + 1);
    let stem: String = name.chars().take(keep).collect();
    let stem = trim_name(&stem);
    match extension {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem.to_string(),
    }
}

fn is_reserved_device_name(stem: &str) -> bool {
    matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// Decodes `%XX` escapes; invalid UTF-8 becomes U+FFFD rather than failing.
fn percent_decode(text: &str) -> String {
    String::from_utf8_lossy(&percent_decode_bytes(text)).into_owned()
}

fn percent_decode_bytes(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_the_last_path_segment() {
        assert_eq!(
            file_name_from_url("https://example.com/files/setup-1.2.exe"),
            "setup-1.2.exe"
        );
    }

    #[test]
    fn query_and_fragment_are_ignored() {
        assert_eq!(
            file_name_from_url("https://cdn.example.com/a/movie.mp4?token=abc&x=1#t=10"),
            "movie.mp4"
        );
    }

    #[test]
    fn percent_encoding_is_decoded_including_utf8() {
        assert_eq!(
            file_name_from_url("https://example.com/My%20Report%20%282024%29.pdf"),
            "My Report (2024).pdf"
        );
        assert_eq!(
            file_name_from_url("https://example.com/%E0%A6%AC%E0%A6%87.pdf"),
            "বই.pdf"
        );
    }

    #[test]
    fn urls_without_a_file_name_fall_back() {
        for url in [
            "https://example.com",
            "https://example.com/",
            "https://example.com/folder/",
            "https://example.com/?file=a.zip",
            "not a url",
        ] {
            assert_eq!(file_name_from_url(url), FALLBACK_NAME, "{url}");
        }
    }

    #[test]
    fn decoded_separators_cannot_escape_the_folder() {
        // %2F is "/" and %5C is "\": they must not become path separators.
        let name = file_name_from_url("https://example.com/..%2F..%2Fevil%5Cx.exe");
        assert!(!name.contains('/') && !name.contains('\\'), "{name}");
        assert!(name.ends_with("x.exe"), "{name}");
    }

    #[test]
    fn invalid_windows_characters_are_replaced() {
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*h.txt"),
            "a_b_c_d_e_f_g_h.txt"
        );
        assert_eq!(sanitize_file_name("dir/sub\\file.txt"), "dir_sub_file.txt");
        assert_eq!(sanitize_file_name("tab\there\u{7}.txt"), "tab_here_.txt");
    }

    #[test]
    fn trailing_dots_and_spaces_are_removed() {
        assert_eq!(sanitize_file_name("report.pdf. . "), "report.pdf");
        assert_eq!(sanitize_file_name("  name  "), "name");
    }

    #[test]
    fn reserved_device_names_are_avoided() {
        for reserved in ["CON", "nul", "Com1", "LPT9", "aux.txt", "PRN.tar.gz"] {
            let safe = sanitize_file_name(reserved);
            let stem = safe.split('.').next().unwrap().to_ascii_uppercase();
            assert!(
                !["CON", "PRN", "AUX", "NUL", "COM1", "LPT9"].contains(&stem.as_str()),
                "{reserved} became {safe}"
            );
        }
        // Names that only start like a device name are fine.
        assert_eq!(sanitize_file_name("console.log"), "console.log");
    }

    #[test]
    fn empty_or_dot_only_names_fall_back() {
        for name in ["", "   ", ".", "..", "...", " . "] {
            assert_eq!(sanitize_file_name(name), FALLBACK_NAME, "{name:?}");
        }
    }

    // --- Content-Disposition ---

    fn disposition(value: &str) -> Option<String> {
        file_name_from_disposition(value)
    }

    #[test]
    fn disposition_quoted_and_token_names() {
        assert_eq!(
            disposition("attachment; filename=\"report.pdf\"").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            disposition("attachment; filename=report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            disposition("inline;filename=\"a b.txt\"").as_deref(),
            Some("a b.txt")
        );
    }

    #[test]
    fn disposition_names_are_case_insensitive() {
        assert_eq!(
            disposition("Attachment; FILENAME=\"x.zip\"").as_deref(),
            Some("x.zip")
        );
    }

    #[test]
    fn disposition_quoted_strings_keep_escapes_and_semicolons() {
        assert_eq!(
            disposition(r#"attachment; filename="a \"b\"; c.txt"; size=3"#).as_deref(),
            Some(r#"a "b"; c.txt"#)
        );
    }

    #[test]
    fn disposition_extended_utf8_name_is_decoded() {
        assert_eq!(
            disposition("attachment; filename*=UTF-8''%E0%A6%AC%E0%A6%87%20one.pdf").as_deref(),
            Some("বই one.pdf")
        );
        // The language part is ignored.
        assert_eq!(
            disposition("attachment; filename*=utf-8'en'na%C3%AFve.txt").as_deref(),
            Some("naïve.txt")
        );
    }

    #[test]
    fn disposition_extended_latin1_name_is_decoded() {
        assert_eq!(
            disposition("attachment; filename*=iso-8859-1'en'%A3%20rates.txt").as_deref(),
            Some("£ rates.txt")
        );
    }

    #[test]
    fn disposition_extended_name_wins_in_either_order() {
        for value in [
            "attachment; filename=\"fallback.pdf\"; filename*=UTF-8''real%20name.pdf",
            "attachment; filename*=UTF-8''real%20name.pdf; filename=\"fallback.pdf\"",
        ] {
            assert_eq!(
                disposition(value).as_deref(),
                Some("real name.pdf"),
                "{value}"
            );
        }
    }

    #[test]
    fn disposition_unknown_charset_falls_back_to_plain_name() {
        assert_eq!(
            disposition("attachment; filename*=x-klingon''abc; filename=\"plain.txt\"").as_deref(),
            Some("plain.txt")
        );
    }

    #[test]
    fn disposition_without_a_name_gives_none() {
        for value in [
            "attachment",
            "inline",
            "attachment; filename=\"\"",
            "attachment; size=10",
            "",
        ] {
            assert_eq!(disposition(value), None, "{value:?}");
        }
    }

    #[test]
    fn disposition_raw_utf8_in_plain_name_is_kept() {
        // Many servers send UTF-8 directly instead of using filename*.
        assert_eq!(
            disposition("attachment; filename=\"বই.pdf\"").as_deref(),
            Some("বই.pdf")
        );
    }

    // --- choosing the name ---

    #[test]
    fn the_server_name_wins_over_the_url() {
        assert_eq!(
            choose_file_name(
                Some("attachment; filename=\"Holiday Photos.zip\""),
                "https://example.com/download.php?id=8812",
                Some("application/zip"),
            ),
            "Holiday Photos.zip"
        );
    }

    #[test]
    fn the_url_name_is_used_without_a_server_name() {
        assert_eq!(
            choose_file_name(None, "https://example.com/a/setup.exe", None),
            "setup.exe"
        );
        assert_eq!(
            choose_file_name(Some("attachment"), "https://example.com/a/setup.exe", None),
            "setup.exe"
        );
    }

    #[test]
    fn folders_in_a_server_name_are_dropped() {
        for value in [
            "attachment; filename=\"../../Windows/evil.exe\"",
            "attachment; filename=\"C:\\Users\\me\\evil.exe\"",
        ] {
            assert_eq!(
                choose_file_name(Some(value), "https://example.com/x", None),
                "evil.exe",
                "{value}"
            );
        }
    }

    #[test]
    fn a_missing_extension_is_added_from_the_file_type() {
        assert_eq!(
            choose_file_name(
                None,
                "https://example.com/files/report",
                Some("application/pdf")
            ),
            "report.pdf"
        );
        assert_eq!(
            choose_file_name(None, "https://example.com/", Some("video/mp4; codecs=avc1")),
            "download.mp4"
        );
        // A version number is not an extension.
        assert_eq!(
            choose_file_name(
                None,
                "https://example.com/release-2.0",
                Some("application/zip")
            ),
            "release-2.0.zip"
        );
    }

    #[test]
    fn a_script_extension_is_replaced_by_the_file_type() {
        assert_eq!(
            choose_file_name(
                None,
                "https://example.com/download.php?id=1",
                Some("application/zip")
            ),
            "download.zip"
        );
        assert_eq!(
            choose_file_name(
                None,
                "https://example.com/Get.ASPX",
                Some("application/pdf")
            ),
            "Get.pdf"
        );
    }

    #[test]
    fn a_real_extension_is_never_changed() {
        assert_eq!(
            choose_file_name(None, "https://example.com/video.mp4", Some("text/plain")),
            "video.mp4"
        );
        assert_eq!(
            choose_file_name(
                Some("attachment; filename=\"data.bin\""),
                "https://example.com/x",
                Some("application/zip")
            ),
            "data.bin"
        );
    }

    #[test]
    fn generic_or_unknown_types_add_nothing() {
        for content_type in [
            "application/octet-stream",
            "application/x-unheard-of",
            "garbage",
        ] {
            assert_eq!(
                choose_file_name(None, "https://example.com/blob", Some(content_type)),
                "blob",
                "{content_type}"
            );
        }
        // A script name stays as it is when the type gives no better extension.
        assert_eq!(
            choose_file_name(
                None,
                "https://example.com/get.php",
                Some("application/octet-stream")
            ),
            "get.php"
        );
    }

    #[test]
    fn file_types_map_to_their_usual_extension() {
        let cases = [
            ("application/zip", "zip"),
            ("APPLICATION/PDF", "pdf"),
            ("application/x-msdownload", "exe"),
            ("application/x-7z-compressed", "7z"),
            ("video/mp4", "mp4"),
            ("audio/mpeg", "mp3"),
            ("image/jpeg", "jpg"),
            ("text/plain; charset=utf-8", "txt"),
        ];
        for (content_type, want) in cases {
            assert_eq!(
                extension_for_mime(content_type),
                Some(want),
                "{content_type}"
            );
        }
        assert_eq!(extension_for_mime("application/octet-stream"), None);
    }

    #[test]
    fn long_names_are_shortened_keeping_the_extension() {
        let long = format!("{}.iso", "a".repeat(400));
        let safe = sanitize_file_name(&long);
        assert!(
            safe.chars().count() <= 200,
            "{} chars",
            safe.chars().count()
        );
        assert!(safe.ends_with(".iso"));
    }
}
