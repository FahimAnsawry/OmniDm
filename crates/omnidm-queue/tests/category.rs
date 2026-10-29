// SPDX-License-Identifier: GPL-3.0-or-later

use omnidm_queue::PostAction;
use omnidm_queue::category::{CategoryRule, FileFacts, default_categories, host_matches, pick};

fn file<'a>(file_name: &'a str, mime: Option<&'a str>, host: Option<&'a str>) -> FileFacts<'a> {
    FileFacts {
        file_name,
        mime,
        host,
    }
}

fn rule(extensions: &[&str], mime_types: &[&str], hosts: &[&str]) -> CategoryRule {
    let own = |items: &[&str]| items.iter().map(|s| s.to_string()).collect();
    CategoryRule {
        extensions: own(extensions),
        mime_types: own(mime_types),
        hosts: own(hosts),
    }
}

#[test]
fn extension_matches_case_and_dot_insensitively() {
    let compressed = rule(&["zip", ".7Z", "tar.gz"], &[], &[]);
    assert!(compressed.matches(&file("Setup.ZIP", None, None)));
    assert!(compressed.matches(&file("archive.7z", None, None)));
    assert!(compressed.matches(&file("src.tar.gz", None, None)));
    assert!(
        !compressed.matches(&file("zip", None, None)),
        "a bare name has no extension"
    );
    assert!(!compressed.matches(&file("notes.txt", None, None)));
    assert!(!compressed.matches(&file("file.gzip", None, None)));
}

#[test]
fn mime_matches_exact_and_wildcard_ignoring_parameters() {
    let video = rule(&[], &["video/*", "application/pdf"], &[]);
    assert!(video.matches(&file("watch", Some("video/mp4"), None)));
    assert!(video.matches(&file("doc", Some("Application/PDF; charset=binary"), None)));
    assert!(!video.matches(&file("x", Some("videos/mp4"), None)));
    assert!(!video.matches(&file("x", Some("audio/mpeg"), None)));
    assert!(!video.matches(&file("x", None, None)));
}

#[test]
fn host_matches_site_and_subdomains_only() {
    assert!(host_matches("github.com", "github.com"));
    assert!(host_matches("objects.GitHub.com", ".github.com"));
    assert!(!host_matches("notgithub.com", "github.com"));
    assert!(!host_matches("github.com", ""));
    let code = rule(&[], &[], &["github.com"]);
    assert!(code.matches(&file("x.zip", None, Some("codeload.github.com"))));
    assert!(!code.matches(&file("x.zip", None, None)));
}

#[test]
fn first_matching_category_wins() {
    let code = rule(&[], &[], &["github.com"]);
    let compressed = rule(&["zip"], &[], &[]);
    let categories = [(1, &code), (2, &compressed)];
    assert_eq!(
        pick(categories, &file("release.zip", None, Some("github.com"))),
        Some(1)
    );
    assert_eq!(
        pick(categories, &file("release.zip", None, Some("example.com"))),
        Some(2)
    );
    assert_eq!(pick(categories, &file("page.html", None, None)), None);
}

#[test]
fn empty_rule_matches_nothing() {
    let empty = CategoryRule::default();
    assert!(empty.is_empty());
    assert!(!empty.matches(&file("a.zip", Some("video/mp4"), Some("github.com"))));
}

#[test]
fn defaults_sort_common_files() {
    let defaults = default_categories();
    let category_of = |name: &str, mime: Option<&str>| {
        pick(
            defaults.iter().map(|(n, r)| (*n, r)),
            &file(name, mime, Some("example.com")),
        )
    };
    assert_eq!(category_of("linux.iso", None), Some("Programs"));
    assert_eq!(category_of("src.tar.gz", None), Some("Compressed"));
    assert_eq!(category_of("report.PDF", None), Some("Documents"));
    assert_eq!(category_of("song.flac", None), Some("Music"));
    assert_eq!(category_of("clip.mkv", None), Some("Video"));
    assert_eq!(category_of("stream", Some("video/webm")), Some("Video"));
    assert_eq!(category_of("photo.jpeg", None), Some("Pictures"));
    assert_eq!(category_of("index.html", Some("text/html")), None);
}

#[test]
fn rule_stores_as_json_without_empty_lists() {
    let code = rule(&[], &[], &["github.com"]);
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, r#"{"hosts":["github.com"]}"#);
    assert_eq!(serde_json::from_str::<CategoryRule>(&json).unwrap(), code);
}

#[test]
fn post_action_names_round_trip() {
    for action in [
        PostAction::None,
        PostAction::Sleep,
        PostAction::Hibernate,
        PostAction::Shutdown,
        PostAction::Quit,
    ] {
        assert_eq!(PostAction::from_name(action.as_str()), Some(action));
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, format!("\"{}\"", action.as_str()));
    }
    assert_eq!(PostAction::from_name("reboot"), None);
}
