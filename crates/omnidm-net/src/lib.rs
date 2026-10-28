// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP client layer: proxies and PAC, authentication, cookies, redirects, file naming.

pub mod http;
pub mod naming;

use std::time::Duration;

/// HTTP client for downloads. HTTP/1.1 only: over HTTP/2 every segment would share one
/// TCP connection, which defeats downloading with several connections.
pub fn download_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .http1_only()
        .user_agent(concat!("OmniDm/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .build()
}
