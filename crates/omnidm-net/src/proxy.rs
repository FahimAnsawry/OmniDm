// SPDX-License-Identifier: GPL-3.0-or-later

//! Which proxy a download goes through: the one Windows is set to use (a fixed proxy with
//! its bypass list, a PAC script, or automatic detection), or none.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

pub use reqwest::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyError(String);

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProxyError {}

/// Reads a proxy address: `host:port` (HTTP) or an `http`/`https` URL.
pub fn parse_proxy_address(text: &str) -> Result<Url, ProxyError> {
    let text = text.trim();
    let bad = || ProxyError(format!("not a usable proxy address: {text:?}"));
    if text.is_empty() {
        return Err(bad());
    }
    let full = if text.contains("://") {
        text.to_string()
    } else {
        format!("http://{text}")
    };
    let url = Url::parse(&full).map_err(|_| bad())?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(bad());
    }
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err(bad()),
    }
}

/// Hosts that are reached directly, never through the proxy. Entries are separated by
/// `;`, `,` or spaces: `example.com` (that host), `*.example.com` or `10.*` (wildcards),
/// `.example.com` (the domain and its subdomains) and `<local>` (names without a dot).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bypass(Vec<String>);

impl Bypass {
    pub fn parse(list: &str) -> Self {
        Self(
            list.split([';', ',', ' ', '\t', '\r', '\n'])
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_ascii_lowercase)
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn matches(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        self.0.iter().any(|entry| {
            if entry == "<local>" {
                !host.contains('.') && !host.contains(':')
            } else if let Some(domain) = entry.strip_prefix('.') {
                host == domain || host.ends_with(entry.as_str())
            } else if entry.contains('*') {
                wildcard_match(entry, &host)
            } else {
                host == *entry
            }
        })
    }
}

/// `*` matches any run of characters, including none.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.is_empty()
}

/// `localhost` and loopback addresses: this computer, never reached through a proxy.
fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The proxy settings of the current Windows user, as Internet Options shows them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsProxySettings {
    /// "Automatically detect settings": look for a PAC script on the network (WPAD).
    pub auto_detect: bool,
    /// "Use setup script": the PAC script's address.
    pub auto_config_url: Option<String>,
    /// "Use a proxy server": `host:port`, or `http=host:port;https=host:port`.
    pub proxy: Option<String>,
    /// Hosts that skip the proxy, `;`-separated (see [`Bypass`]).
    pub bypass: Option<String>,
}

/// A fixed proxy for HTTP and HTTPS, with the hosts that skip it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixedProxy {
    pub http: Option<Url>,
    pub https: Option<Url>,
    pub bypass: Bypass,
}

impl FixedProxy {
    /// Reads the Windows format: one address for both protocols, or `protocol=address`
    /// entries. Protocols other than HTTP and HTTPS (FTP, SOCKS) are ignored.
    pub fn parse(proxy: &str, bypass: &str) -> Self {
        let mut fixed = Self {
            bypass: Bypass::parse(bypass),
            ..Self::default()
        };
        for entry in proxy
            .split([';', ' '])
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            match entry.split_once('=') {
                Some((protocol, address)) => {
                    let address = parse_proxy_address(address).ok();
                    match protocol.trim().to_ascii_lowercase().as_str() {
                        "http" => fixed.http = address,
                        "https" => fixed.https = address,
                        _ => {}
                    }
                }
                None => {
                    if let Ok(address) = parse_proxy_address(entry) {
                        fixed.http.get_or_insert_with(|| address.clone());
                        fixed.https.get_or_insert(address);
                    }
                }
            }
        }
        fixed
    }

    /// No proxy for either protocol.
    pub fn is_direct(&self) -> bool {
        self.http.is_none() && self.https.is_none()
    }

    /// The proxy for `url`, or `None` to go direct.
    pub fn proxy_for(&self, url: &Url) -> Option<Url> {
        if is_loopback(url) || self.bypass.matches(url) {
            return None;
        }
        match url.scheme() {
            "http" => self.http.clone(),
            "https" => self.https.clone(),
            _ => None,
        }
    }
}

/// How downloads reach the internet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyPlan {
    Direct,
    Fixed(FixedProxy),
    /// Ask a PAC script for each host: the one at `script`, or one found on the network
    /// when `None`. If no script can be used, `fallback` applies (as Windows does).
    Pac {
        script: Option<String>,
        fallback: FixedProxy,
    },
}

impl ProxyPlan {
    pub fn from_settings(settings: &WindowsProxySettings) -> Self {
        let fixed = FixedProxy::parse(
            settings.proxy.as_deref().unwrap_or_default(),
            settings.bypass.as_deref().unwrap_or_default(),
        );
        let script = settings
            .auto_config_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        if script.is_some() || settings.auto_detect {
            Self::Pac {
                script,
                fallback: fixed,
            }
        } else if fixed.is_direct() {
            Self::Direct
        } else {
            Self::Fixed(fixed)
        }
    }
}

/// The first usable proxy in a PAC answer as Windows returns it (`host:port` entries
/// separated by `;` or spaces; an empty answer means direct).
fn first_pac_proxy(list: &str) -> Option<Url> {
    list.split([';', ' '])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .find_map(|entry| parse_proxy_address(entry).ok())
}

/// Decides the proxy for each connection a download client makes.
pub(crate) struct Resolver {
    plan: ProxyPlan,
    /// PAC answers per `scheme://host:port`: a script runs once per site, not per request.
    pac_cache: Mutex<HashMap<String, Option<Url>>>,
    #[cfg(windows)]
    pac_session: Option<win::PacSession>,
}

impl Resolver {
    pub(crate) fn new(plan: ProxyPlan) -> Self {
        Self {
            #[cfg(windows)]
            pac_session: matches!(plan, ProxyPlan::Pac { .. })
                .then(win::PacSession::open)
                .flatten(),
            plan,
            pac_cache: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn proxy_for(&self, url: &Url) -> Option<Url> {
        if is_loopback(url) {
            return None;
        }
        match &self.plan {
            ProxyPlan::Direct => None,
            ProxyPlan::Fixed(fixed) => fixed.proxy_for(url),
            ProxyPlan::Pac { script, fallback } => {
                let key = format!(
                    "{}://{}:{}",
                    url.scheme(),
                    url.host_str().unwrap_or_default(),
                    url.port_or_known_default().unwrap_or_default()
                );
                if let Some(known) = self.pac_cache.lock().unwrap().get(&key) {
                    return known.clone();
                }
                let answer = match self.ask_pac(url, script.as_deref()) {
                    Some(list) => list.as_deref().and_then(first_pac_proxy),
                    None => fallback.proxy_for(url),
                };
                self.pac_cache.lock().unwrap().insert(key, answer.clone());
                answer
            }
        }
    }

    /// `Some(None)` for DIRECT, `Some(Some(list))` for proxies, `None` if no script ran.
    #[cfg(windows)]
    fn ask_pac(&self, url: &Url, script: Option<&str>) -> Option<Option<String>> {
        let session = self.pac_session.as_ref()?;
        // Fetching and running the script can take a while the first time: let the
        // runtime move other work off this thread meanwhile.
        let ask = || session.proxy_for(url.as_str(), script);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(ask)
            }
            _ => ask(),
        }
    }

    #[cfg(not(windows))]
    fn ask_pac(&self, _url: &Url, _script: Option<&str>) -> Option<Option<String>> {
        None
    }
}

/// The current user's Windows proxy settings; empty if they can't be read.
#[cfg(windows)]
pub fn windows_settings() -> WindowsProxySettings {
    win::settings().unwrap_or_default()
}

#[cfg(not(windows))]
pub fn windows_settings() -> WindowsProxySettings {
    WindowsProxySettings::default()
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTO_DETECT_TYPE_DHCP,
        WINHTTP_AUTO_DETECT_TYPE_DNS_A, WINHTTP_AUTOPROXY_AUTO_DETECT,
        WINHTTP_AUTOPROXY_CONFIG_URL, WINHTTP_AUTOPROXY_OPTIONS,
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO, WinHttpCloseHandle,
        WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen,
    };
    use windows_sys::core::PWSTR;

    use super::WindowsProxySettings;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Copies a string WinHTTP allocated, then frees it.
    ///
    /// # Safety
    /// `p` is null or a NUL-terminated string allocated with `GlobalAlloc`, not used again.
    unsafe fn take_string(p: PWSTR) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: the caller guarantees a NUL-terminated string that we now own.
        unsafe {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            GlobalFree(p.cast());
            Some(text)
        }
    }

    pub(super) fn settings() -> Option<WindowsProxySettings> {
        let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
        // SAFETY: `config` is a valid out-parameter; on success its strings are ours to
        // free, which `take_string` does exactly once each.
        unsafe {
            if WinHttpGetIEProxyConfigForCurrentUser(&mut config) == 0 {
                return None;
            }
            Some(WindowsProxySettings {
                auto_detect: config.fAutoDetect != 0,
                auto_config_url: take_string(config.lpszAutoConfigUrl),
                proxy: take_string(config.lpszProxy),
                bypass: take_string(config.lpszProxyBypass),
            })
        }
    }

    /// A WinHTTP session used only to evaluate PAC scripts.
    pub(super) struct PacSession(*mut c_void);

    // SAFETY: WinHTTP session handles may be used from any thread.
    unsafe impl Send for PacSession {}
    // SAFETY: as above; `WinHttpGetProxyForUrl` may be called concurrently on a session.
    unsafe impl Sync for PacSession {}

    impl PacSession {
        pub(super) fn open() -> Option<Self> {
            let agent = wide("OmniDm");
            // SAFETY: `agent` is NUL-terminated and outlives the call; null proxy strings
            // are allowed with WINHTTP_ACCESS_TYPE_NO_PROXY.
            let handle = unsafe {
                WinHttpOpen(
                    agent.as_ptr(),
                    WINHTTP_ACCESS_TYPE_NO_PROXY,
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                )
            };
            (!handle.is_null()).then_some(Self(handle))
        }

        /// Runs the PAC script at `script` (or one found on the network) for `url`.
        /// `Some(None)` means DIRECT; `None` means no script could be used.
        pub(super) fn proxy_for(&self, url: &str, script: Option<&str>) -> Option<Option<String>> {
            let url = wide(url);
            let script = script.map(wide);
            let mut options = WINHTTP_AUTOPROXY_OPTIONS {
                fAutoLogonIfChallenged: 1,
                ..WINHTTP_AUTOPROXY_OPTIONS::default()
            };
            match &script {
                Some(script) => {
                    options.dwFlags = WINHTTP_AUTOPROXY_CONFIG_URL;
                    options.lpszAutoConfigUrl = script.as_ptr();
                }
                None => {
                    options.dwFlags = WINHTTP_AUTOPROXY_AUTO_DETECT;
                    options.dwAutoDetectFlags =
                        WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
                }
            }
            let mut info = WINHTTP_PROXY_INFO::default();
            // SAFETY: the session is open, `url` and `script` are NUL-terminated and live
            // across the call, and `info` is a valid out-parameter whose strings we free.
            unsafe {
                if WinHttpGetProxyForUrl(self.0, url.as_ptr(), &mut options, &mut info) == 0 {
                    return None;
                }
                let proxy = take_string(info.lpszProxy);
                let _ = take_string(info.lpszProxyBypass);
                if info.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY {
                    Some(None)
                } else {
                    Some(proxy)
                }
            }
        }
    }

    impl Drop for PacSession {
        fn drop(&mut self) {
            // SAFETY: the handle came from WinHttpOpen and is closed once.
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn a_bare_host_and_port_is_an_http_proxy() {
        assert_eq!(
            parse_proxy_address("127.0.0.1:8080").unwrap().as_str(),
            "http://127.0.0.1:8080/"
        );
        assert_eq!(
            parse_proxy_address("proxy.corp:3128").unwrap().as_str(),
            "http://proxy.corp:3128/"
        );
    }

    #[test]
    fn http_and_https_addresses_are_accepted() {
        for text in ["http://p:1", "https://p:443"] {
            let parsed = parse_proxy_address(text).unwrap();
            assert_eq!(parsed.scheme(), text.split(':').next().unwrap(), "{text}");
        }
    }

    #[test]
    fn unusable_proxy_addresses_are_rejected() {
        for text in [
            "",
            "   ",
            "ftp://p:21",
            "socks5://p:1080",
            "http://",
            "p:notaport",
        ] {
            assert!(parse_proxy_address(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn bypass_entries_match_hosts_exactly_or_by_wildcard() {
        let bypass = Bypass::parse("intranet.corp; *.local.test, 10.*");
        assert!(bypass.matches(&url("http://intranet.corp/x")));
        assert!(bypass.matches(&url("https://INTRANET.corp/")));
        assert!(!bypass.matches(&url("http://www.intranet.corp/")));
        assert!(bypass.matches(&url("http://a.local.test/")));
        assert!(bypass.matches(&url("http://a.b.local.test/")));
        assert!(bypass.matches(&url("http://10.1.2.3/")));
        assert!(!bypass.matches(&url("http://110.1.2.3/")));
        assert!(!bypass.matches(&url("http://example.com/")));
    }

    #[test]
    fn a_leading_dot_means_the_domain_and_its_subdomains() {
        let bypass = Bypass::parse(".corp.example");
        assert!(bypass.matches(&url("http://corp.example/")));
        assert!(bypass.matches(&url("http://files.corp.example/")));
        assert!(!bypass.matches(&url("http://notcorp.example/")));
    }

    #[test]
    fn local_means_names_without_a_dot() {
        let bypass = Bypass::parse("<local>");
        assert!(bypass.matches(&url("http://fileserver/share")));
        assert!(!bypass.matches(&url("http://fileserver.corp/share")));
        assert!(
            !bypass.matches(&url("http://10.0.0.1/")),
            "an IP address has dots"
        );
    }

    #[test]
    fn an_empty_bypass_list_matches_nothing() {
        let bypass = Bypass::parse(" ; , ");
        assert!(bypass.is_empty());
        assert!(!bypass.matches(&url("http://fileserver/")));
    }

    // --- a fixed proxy from the Windows settings ---

    fn proxy_of(fixed: &FixedProxy, to: &str) -> Option<String> {
        fixed.proxy_for(&url(to)).map(|u| u.to_string())
    }

    #[test]
    fn one_address_is_used_for_http_and_https() {
        let fixed = FixedProxy::parse("proxy.corp:8080", "");
        assert_eq!(
            proxy_of(&fixed, "http://example.com/a"),
            Some("http://proxy.corp:8080/".into())
        );
        assert_eq!(
            proxy_of(&fixed, "https://example.com/a"),
            Some("http://proxy.corp:8080/".into())
        );
    }

    #[test]
    fn per_protocol_addresses_are_used_for_their_protocol() {
        let fixed = FixedProxy::parse(
            "http=web:3128;https=secure:8443;ftp=old:21;socks=s:1080",
            "",
        );
        assert_eq!(
            proxy_of(&fixed, "http://example.com/"),
            Some("http://web:3128/".into())
        );
        assert_eq!(
            proxy_of(&fixed, "https://example.com/"),
            Some("http://secure:8443/".into())
        );
    }

    #[test]
    fn a_protocol_without_a_proxy_goes_direct() {
        let fixed = FixedProxy::parse("https=secure:443", "");
        assert_eq!(proxy_of(&fixed, "http://example.com/"), None);
        assert!(proxy_of(&fixed, "https://example.com/").is_some());
        // Only SOCKS (not supported) means no usable proxy at all.
        assert!(FixedProxy::parse("socks=s:1080", "").is_direct());
        assert!(FixedProxy::parse("", "").is_direct());
    }

    #[test]
    fn bypassed_hosts_go_direct() {
        let fixed = FixedProxy::parse("p:8080", "*.intranet.corp;<local>");
        assert_eq!(proxy_of(&fixed, "http://files.intranet.corp/"), None);
        assert_eq!(proxy_of(&fixed, "http://nas/share"), None);
        assert!(proxy_of(&fixed, "http://example.com/").is_some());
    }

    #[test]
    fn this_computer_is_always_reached_directly() {
        let fixed = FixedProxy::parse("p:8080", "");
        for to in [
            "http://localhost:5000/",
            "http://127.0.0.1:5000/",
            "http://127.9.9.9/",
            "http://[::1]:5000/",
        ] {
            assert_eq!(proxy_of(&fixed, to), None, "{to}");
        }
    }

    // --- choosing from the Windows settings ---

    #[test]
    fn no_settings_means_direct() {
        assert_eq!(
            ProxyPlan::from_settings(&WindowsProxySettings::default()),
            ProxyPlan::Direct
        );
    }

    #[test]
    fn a_fixed_proxy_alone_is_used_as_is() {
        let settings = WindowsProxySettings {
            proxy: Some("p:8080".into()),
            bypass: Some("<local>".into()),
            ..WindowsProxySettings::default()
        };
        assert_eq!(
            ProxyPlan::from_settings(&settings),
            ProxyPlan::Fixed(FixedProxy::parse("p:8080", "<local>"))
        );
    }

    #[test]
    fn a_pac_script_comes_first_with_the_fixed_proxy_as_fallback() {
        let settings = WindowsProxySettings {
            auto_detect: true,
            auto_config_url: Some("http://wpad.corp/proxy.pac".into()),
            proxy: Some("p:8080".into()),
            bypass: None,
        };
        assert_eq!(
            ProxyPlan::from_settings(&settings),
            ProxyPlan::Pac {
                script: Some("http://wpad.corp/proxy.pac".into()),
                fallback: FixedProxy::parse("p:8080", ""),
            }
        );
    }

    #[test]
    fn automatic_detection_looks_for_a_script() {
        let settings = WindowsProxySettings {
            auto_detect: true,
            ..WindowsProxySettings::default()
        };
        assert_eq!(
            ProxyPlan::from_settings(&settings),
            ProxyPlan::Pac {
                script: None,
                fallback: FixedProxy::default(),
            }
        );
    }

    #[test]
    fn blank_settings_are_ignored() {
        let settings = WindowsProxySettings {
            auto_detect: false,
            auto_config_url: Some("  ".into()),
            proxy: Some("".into()),
            bypass: Some("".into()),
        };
        assert_eq!(ProxyPlan::from_settings(&settings), ProxyPlan::Direct);
    }

    #[test]
    fn the_first_proxy_of_a_pac_answer_is_used() {
        assert_eq!(
            first_pac_proxy("a.corp:8080;b.corp:8080").map(|u| u.to_string()),
            Some("http://a.corp:8080/".into())
        );
        assert_eq!(
            first_pac_proxy("  a.corp:8080 b.corp:8080").map(|u| u.to_string()),
            Some("http://a.corp:8080/".into())
        );
        assert_eq!(first_pac_proxy(""), None);
        assert_eq!(first_pac_proxy(" ; "), None);
    }
}
