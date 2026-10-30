// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP client layer: proxies and PAC, authentication, cookies, redirects, file naming.

pub mod cookies;
pub mod http;
pub mod naming;
pub mod proxy;
pub mod redirect;

use std::time::Duration;

/// HTTP client for downloads. HTTP/1.1 only: over HTTP/2 every segment would share one
/// TCP connection, which defeats downloading with several connections. Redirects are not
/// followed automatically: the engine follows them itself (see [`redirect`]).
///
/// Connections go through the proxy Windows is set to use (see [`proxy`]).
pub fn download_client() -> reqwest::Result<reqwest::Client> {
    download_client_with(proxy::ProxyPlan::from_settings(&proxy::windows_settings()))
}

/// [`download_client`] with an explicit proxy plan.
pub fn download_client_with(plan: proxy::ProxyPlan) -> reqwest::Result<reqwest::Client> {
    let builder = reqwest::Client::builder();
    // reqwest's own system proxy support is turned off either way: it misses PAC scripts
    // and parts of the Windows format, so the plan decides alone.
    let builder = match plan {
        proxy::ProxyPlan::Direct => builder.no_proxy(),
        plan => {
            let resolver = std::sync::Arc::new(proxy::Resolver::new(plan));
            builder.proxy(reqwest::Proxy::custom(move |url| resolver.proxy_for(url)))
        }
    };
    builder
        .http1_only()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("OmniDM/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .build()
}
