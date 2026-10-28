// SPDX-License-Identifier: GPL-3.0-or-later

//! File names: taken from a URL and made safe to use on Windows.

/// Used when nothing better is available.
pub const FALLBACK_NAME: &str = "download";

/// Longest file name kept, in characters (Windows allows 255 UTF-16 units).
const MAX_NAME_CHARS: usize = 200;

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
    String::from_utf8_lossy(&out).into_owned()
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
