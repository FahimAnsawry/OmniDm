// SPDX-License-Identifier: GPL-3.0-or-later

//! One download's cookies: the ones it was started with, plus what servers set along the
//! way. Each request gets only the cookies that belong to its URL.

use std::fmt;
use std::sync::{Arc, Mutex};

use cookie_store::{CookieStore, RawCookie};
pub use reqwest::Url;

/// A cookie as a browser reports it, with the rules for where it may be sent.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserCookieSpec {
    pub name: String,
    pub value: String,
    /// A leading dot is allowed and ignored.
    pub domain: String,
    /// Only for exactly `domain`, not its subdomains.
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds; `None` for a session cookie.
    pub expires: Option<f64>,
}

/// A cookie jar shared by every connection of one download. Clones share the same jar.
#[derive(Clone, Default)]
pub struct CookieJar(Arc<Mutex<CookieStore>>);

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().iter_unexpired().next().is_none()
    }

    /// Adds `name=value; name2=value2` (a `Cookie` header's format) as cookies of `url`'s
    /// host, for every path. Pairs without a name or `=` are skipped.
    pub fn add_pairs(&self, url: &Url, pairs: &str) {
        let mut store = self.0.lock().unwrap();
        for pair in pairs.split(';') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() {
                continue;
            }
            // A bad cookie is skipped rather than failing the download.
            let _ = store.parse(&format!("{name}={value}; Path=/"), url);
        }
    }

    /// Adds a cookie from the browser, keeping its domain, path, `Secure` and expiry
    /// rules. Expired or malformed cookies are skipped.
    pub fn add_browser_cookie(&self, cookie: &BrowserCookieSpec) {
        let host = cookie.domain.trim_start_matches('.');
        let path = if cookie.path.starts_with('/') {
            cookie.path.as_str()
        } else {
            "/"
        };
        let Ok(origin) = Url::parse(&format!("https://{host}{path}")) else {
            return;
        };
        let mut text = format!("{}={}; Path={path}", cookie.name, cookie.value);
        if !cookie.host_only {
            text.push_str(&format!("; Domain={host}"));
        }
        if cookie.secure {
            text.push_str("; Secure");
        }
        if cookie.http_only {
            text.push_str("; HttpOnly");
        }
        if let Some(expires) = cookie.expires {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            if expires <= now {
                return;
            }
            text.push_str(&format!("; Max-Age={}", (expires - now).ceil() as u64));
        }
        // A cookie the jar refuses is skipped rather than failing the download.
        let _ = self.0.lock().unwrap().parse(&text, &origin);
    }

    /// Stores the `Set-Cookie` values of a response from `url`.
    pub fn store_response<'a>(&self, url: &Url, set_cookies: impl Iterator<Item = &'a str>) {
        let cookies = set_cookies.filter_map(|value| RawCookie::parse(value.to_string()).ok());
        self.0.lock().unwrap().store_response_cookies(cookies, url);
    }

    /// The `Cookie` header value for a request to `url`, if any cookie applies.
    pub fn header_for(&self, url: &Url) -> Option<String> {
        let store = self.0.lock().unwrap();
        let pairs: Vec<String> = store
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }
}

/// Two jars are equal when they are the same jar.
impl PartialEq for CookieJar {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CookieJar {}

/// Never prints cookie values.
impl fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.0.lock().unwrap().iter_unexpired().count();
        write!(f, "CookieJar({count} cookies)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    /// The cookies sent to `to`, sorted, so tests don't depend on their order.
    fn sent(jar: &CookieJar, to: &str) -> Vec<String> {
        let mut pairs: Vec<String> = jar
            .header_for(&url(to))
            .map(|h| h.split("; ").map(str::to_string).collect())
            .unwrap_or_default();
        pairs.sort();
        pairs
    }

    #[test]
    fn a_new_jar_sends_nothing() {
        let jar = CookieJar::new();
        assert!(jar.is_empty());
        assert_eq!(jar.header_for(&url("https://example.com/")), None);
    }

    #[test]
    fn pairs_are_sent_to_their_site_on_every_path() {
        let jar = CookieJar::new();
        jar.add_pairs(
            &url("https://example.com/files/a.zip"),
            "session=abc; theme = dark ",
        );
        assert!(!jar.is_empty());
        assert_eq!(
            sent(&jar, "https://example.com/"),
            ["session=abc", "theme=dark"]
        );
        assert_eq!(
            sent(&jar, "https://example.com/other/b.zip"),
            ["session=abc", "theme=dark"]
        );
    }

    #[test]
    fn pairs_are_not_sent_to_other_sites() {
        let jar = CookieJar::new();
        jar.add_pairs(&url("https://example.com/"), "session=abc");
        assert!(sent(&jar, "https://cdn.example.net/").is_empty());
        assert!(
            sent(&jar, "https://sub.example.com/").is_empty(),
            "host-only"
        );
    }

    #[test]
    fn malformed_pairs_are_skipped() {
        let jar = CookieJar::new();
        jar.add_pairs(&url("https://example.com/"), "novalue; =x; ok=1;;");
        assert_eq!(sent(&jar, "https://example.com/"), ["ok=1"]);
    }

    #[test]
    fn set_cookie_from_a_response_is_sent_back() {
        let jar = CookieJar::new();
        jar.store_response(
            &url("https://example.com/login"),
            ["token=xyz; Path=/"].into_iter(),
        );
        assert_eq!(sent(&jar, "https://example.com/file.zip"), ["token=xyz"]);
    }

    #[test]
    fn a_domain_cookie_reaches_subdomains() {
        let jar = CookieJar::new();
        jar.store_response(
            &url("https://www.example.com/"),
            ["id=1; Domain=example.com; Path=/"].into_iter(),
        );
        assert_eq!(sent(&jar, "https://dl.example.com/x"), ["id=1"]);
        assert!(sent(&jar, "https://example.org/x").is_empty());
    }

    #[test]
    fn a_secure_cookie_is_never_sent_over_plain_http() {
        let jar = CookieJar::new();
        jar.store_response(
            &url("https://example.com/"),
            ["s=1; Secure; Path=/"].into_iter(),
        );
        assert_eq!(sent(&jar, "https://example.com/"), ["s=1"]);
        assert!(sent(&jar, "http://example.com/").is_empty());
    }

    #[test]
    fn a_path_cookie_stays_on_its_path() {
        let jar = CookieJar::new();
        jar.store_response(&url("https://example.com/a/"), ["p=1; Path=/a"].into_iter());
        assert_eq!(sent(&jar, "https://example.com/a/file"), ["p=1"]);
        assert!(sent(&jar, "https://example.com/b/file").is_empty());
    }

    #[test]
    fn a_server_can_replace_or_delete_a_cookie() {
        let jar = CookieJar::new();
        let site = url("https://example.com/");
        jar.add_pairs(&site, "a=old; b=keep");
        jar.store_response(&site, ["a=new; Path=/"].into_iter());
        assert_eq!(sent(&jar, "https://example.com/"), ["a=new", "b=keep"]);
        jar.store_response(&site, ["b=gone; Path=/; Max-Age=0"].into_iter());
        assert_eq!(sent(&jar, "https://example.com/"), ["a=new"]);
    }

    #[test]
    fn a_cookie_for_another_domain_is_refused() {
        let jar = CookieJar::new();
        jar.store_response(
            &url("https://evil.example/"),
            ["steal=1; Domain=example.com; Path=/"].into_iter(),
        );
        assert!(sent(&jar, "https://example.com/").is_empty());
    }

    #[test]
    fn clones_share_one_jar() {
        let jar = CookieJar::new();
        let copy = jar.clone();
        copy.add_pairs(&url("https://example.com/"), "a=1");
        assert_eq!(sent(&jar, "https://example.com/"), ["a=1"]);
        assert_eq!(jar, copy);
        assert_ne!(jar, CookieJar::new());
    }

    // --- cookies from the browser ---

    fn browser(name: &str, domain: &str, host_only: bool) -> BrowserCookieSpec {
        BrowserCookieSpec {
            name: name.into(),
            value: "1".into(),
            domain: domain.into(),
            host_only,
            path: "/".into(),
            secure: false,
            http_only: false,
            expires: None,
        }
    }

    #[test]
    fn a_host_only_browser_cookie_stays_on_its_host() {
        let jar = CookieJar::new();
        jar.add_browser_cookie(&browser("a", "example.com", true));
        assert_eq!(sent(&jar, "https://example.com/x"), ["a=1"]);
        assert!(sent(&jar, "https://dl.example.com/x").is_empty());
    }

    #[test]
    fn a_domain_browser_cookie_reaches_subdomains() {
        let jar = CookieJar::new();
        jar.add_browser_cookie(&browser("a", ".example.com", false));
        assert_eq!(sent(&jar, "https://dl.example.com/x"), ["a=1"]);
        assert_eq!(sent(&jar, "https://example.com/x"), ["a=1"]);
        assert!(sent(&jar, "https://example.org/x").is_empty());
    }

    #[test]
    fn browser_cookie_flags_are_kept() {
        let jar = CookieJar::new();
        jar.add_browser_cookie(&BrowserCookieSpec {
            secure: true,
            path: "/files".into(),
            ..browser("s", "example.com", true)
        });
        assert_eq!(sent(&jar, "https://example.com/files/a.zip"), ["s=1"]);
        assert!(
            sent(&jar, "http://example.com/files/a.zip").is_empty(),
            "secure"
        );
        assert!(sent(&jar, "https://example.com/other").is_empty(), "path");
    }

    #[test]
    fn an_expired_browser_cookie_is_ignored_and_a_future_one_kept() {
        let jar = CookieJar::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        jar.add_browser_cookie(&BrowserCookieSpec {
            expires: Some(now - 60.0),
            ..browser("old", "example.com", true)
        });
        jar.add_browser_cookie(&BrowserCookieSpec {
            expires: Some(now + 3600.0),
            ..browser("new", "example.com", true)
        });
        assert_eq!(sent(&jar, "https://example.com/"), ["new=1"]);
    }
}
