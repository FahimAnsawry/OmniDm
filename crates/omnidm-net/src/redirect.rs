// SPDX-License-Identifier: GPL-3.0-or-later

//! Redirects are followed by hand, so every hop gets the right cookies and credentials,
//! and headers the engine controls are never taken from a captured request.

pub use reqwest::Url;

/// Most redirects followed for one request.
pub const MAX_REDIRECTS: usize = 10;

/// Where a redirect leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub url: Url,
    /// Send the next request as a `GET` without a body (303, and 301/302 like browsers).
    pub switch_to_get: bool,
}

/// The next request after a response with `status` and `location`, if it is a redirect
/// to an `http` or `https` URL.
pub fn next_hop(current: &Url, status: u16, location: Option<&str>) -> Option<Hop> {
    let switch_to_get = match status {
        301..=303 => true,
        307 | 308 => false,
        _ => return None,
    };
    let url = current.join(location?.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(Hop { url, switch_to_get })
}

/// Same scheme, host and port.
pub fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Headers the engine sets itself (ranges, validators, connection handling, cookies from
/// the jar) or that would break byte offsets (compression); a captured value is dropped.
pub fn is_replayable_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !(name.starts_with("if-")
        || name.starts_with("proxy-")
        || matches!(
            name.as_str(),
            "host"
                | "range"
                | "connection"
                | "content-length"
                | "accept-encoding"
                | "upgrade"
                | "transfer-encoding"
                | "te"
                | "keep-alive"
                | "cookie"
        ))
}

/// Whether a header may still be sent after a redirect to another origin. Credentials
/// stay with the site they were meant for.
pub fn is_sent_cross_origin(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "cookie"
    )
}

/// Headers that describe a request body; they are only sent with a `POST`.
pub fn is_body_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "content-type" | "content-encoding" | "content-language" | "content-location"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn redirect_statuses_with_a_location_give_the_next_url() {
        let here = url("https://example.com/a/start?x=1");
        for status in [301, 302, 303, 307, 308] {
            let hop = next_hop(&here, status, Some("https://cdn.example.net/file.zip")).unwrap();
            assert_eq!(
                hop.url.as_str(),
                "https://cdn.example.net/file.zip",
                "{status}"
            );
        }
    }

    #[test]
    fn relative_locations_are_resolved_against_the_current_url() {
        let here = url("https://example.com/a/start?x=1");
        assert_eq!(
            next_hop(&here, 302, Some("/b/file.zip"))
                .unwrap()
                .url
                .as_str(),
            "https://example.com/b/file.zip"
        );
        assert_eq!(
            next_hop(&here, 302, Some("file.zip")).unwrap().url.as_str(),
            "https://example.com/a/file.zip"
        );
    }

    #[test]
    fn other_statuses_or_no_location_are_not_redirects() {
        let here = url("https://example.com/");
        assert!(next_hop(&here, 200, Some("/x")).is_none());
        assert!(next_hop(&here, 304, Some("/x")).is_none());
        assert!(next_hop(&here, 302, None).is_none());
        assert!(next_hop(&here, 302, Some("http://[bad")).is_none());
    }

    #[test]
    fn only_http_and_https_targets_are_followed() {
        let here = url("https://example.com/");
        assert!(next_hop(&here, 302, Some("file:///C:/Windows/win.ini")).is_none());
        assert!(next_hop(&here, 302, Some("ftp://example.com/x")).is_none());
    }

    #[test]
    fn see_other_switches_to_get_and_drops_the_body() {
        let here = url("https://example.com/form");
        assert!(next_hop(&here, 303, Some("/result")).unwrap().switch_to_get);
    }

    #[test]
    fn moved_and_found_switch_to_get_like_browsers_do() {
        let here = url("https://example.com/form");
        assert!(next_hop(&here, 301, Some("/r")).unwrap().switch_to_get);
        assert!(next_hop(&here, 302, Some("/r")).unwrap().switch_to_get);
    }

    #[test]
    fn temporary_and_permanent_redirects_keep_the_method() {
        let here = url("https://example.com/form");
        assert!(!next_hop(&here, 307, Some("/r")).unwrap().switch_to_get);
        assert!(!next_hop(&here, 308, Some("/r")).unwrap().switch_to_get);
    }

    #[test]
    fn same_origin_means_same_scheme_host_and_port() {
        let a = url("https://example.com/a");
        assert!(same_origin(&a, &url("https://example.com/b?c")));
        assert!(same_origin(&a, &url("https://EXAMPLE.com:443/b")));
        assert!(!same_origin(&a, &url("http://example.com/a")));
        assert!(!same_origin(&a, &url("https://cdn.example.com/a")));
        assert!(!same_origin(&a, &url("https://example.com:8443/a")));
    }

    #[test]
    fn engine_controlled_headers_are_not_replayed() {
        for name in [
            "Host",
            "range",
            "If-Range",
            "If-None-Match",
            "If-Modified-Since",
            "Connection",
            "Content-Length",
            "Accept-Encoding",
            "Proxy-Authorization",
            "Upgrade",
            "Transfer-Encoding",
            "TE",
            "Keep-Alive",
            // Cookies go through the download's jar, which knows where each one belongs.
            "Cookie",
        ] {
            assert!(!is_replayable_header(name), "{name}");
        }
    }

    #[test]
    fn ordinary_headers_are_replayed() {
        for name in [
            "Referer",
            "User-Agent",
            "Accept",
            "Accept-Language",
            "Authorization",
            "X-Requested-With",
            "Origin",
        ] {
            assert!(is_replayable_header(name), "{name}");
        }
    }

    #[test]
    fn credentials_do_not_follow_a_redirect_to_another_origin() {
        assert!(is_sent_cross_origin("Referer"));
        assert!(is_sent_cross_origin("User-Agent"));
        assert!(!is_sent_cross_origin("Authorization"));
        assert!(!is_sent_cross_origin("cookie"));
    }

    #[test]
    fn headers_describing_a_body_are_recognised() {
        for name in [
            "Content-Type",
            "content-encoding",
            "Content-Language",
            "Content-Location",
        ] {
            assert!(is_body_header(name), "{name}");
        }
        for name in ["Referer", "Accept", "Content-Disposition"] {
            assert!(!is_body_header(name), "{name}");
        }
    }
}
