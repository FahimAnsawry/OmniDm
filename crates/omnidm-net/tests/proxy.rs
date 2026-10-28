// SPDX-License-Identifier: GPL-3.0-or-later

//! The download client and the Windows proxy plan, against a real (test) proxy.

use omnidm_net::download_client_with;
use omnidm_net::proxy::{FixedProxy, ProxyPlan, windows_settings};
use omnidm_testserver::{FileSpec, TestProxy, TestServer};

const SIZE: u64 = 40_000;

struct Setup {
    server: TestServer,
    proxy: TestProxy,
}

impl Setup {
    async fn new() -> Self {
        let server = TestServer::start().await.unwrap();
        server.set_file("/f.bin", FileSpec::new(SIZE));
        Self {
            server,
            proxy: TestProxy::start(None).await.unwrap(),
        }
    }

    /// An address that only works through the test proxy.
    fn made_up_url(&self) -> String {
        format!(
            "http://files.omnidm.test:{}/f.bin",
            self.server.addr().port()
        )
    }

    fn fixed(&self) -> FixedProxy {
        FixedProxy::parse(&self.proxy.addr().to_string(), "")
    }

    async fn fetch(&self, plan: ProxyPlan) -> reqwest::Result<Vec<u8>> {
        let client = download_client_with(plan).unwrap();
        let resp = client.get(self.made_up_url()).send().await?;
        Ok(resp.error_for_status()?.bytes().await?.to_vec())
    }

    fn assert_went_through_the_proxy(&self) {
        let log = self.proxy.connections();
        assert!(!log.is_empty(), "the proxy was not used");
        assert!(
            log.iter()
                .all(|c| c.target.starts_with("files.omnidm.test:")),
            "{log:?}"
        );
    }
}

#[tokio::test]
async fn a_fixed_proxy_carries_the_download() {
    let setup = Setup::new().await;

    let body = setup.fetch(ProxyPlan::Fixed(setup.fixed())).await.unwrap();

    assert_eq!(body, FileSpec::new(SIZE).bytes(0, SIZE as usize));
    setup.assert_went_through_the_proxy();
}

#[tokio::test]
async fn direct_never_touches_the_proxy() {
    let setup = Setup::new().await;

    let result = setup.fetch(ProxyPlan::Direct).await;

    assert!(
        result.is_err(),
        "the made-up name can't be reached directly"
    );
    assert!(setup.proxy.connections().is_empty());
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_pac_script_picks_the_proxy() {
    let setup = Setup::new().await;
    let script = format!(
        r#"function FindProxyForURL(url, host) {{
            if (shExpMatch(host, "*.omnidm.test")) return "PROXY {}";
            return "DIRECT";
        }}"#,
        setup.proxy.addr()
    );
    setup
        .server
        .set_text("/proxy.pac", "application/x-ns-proxy-autoconfig", &script);

    let body = setup
        .fetch(ProxyPlan::Pac {
            script: Some(setup.server.url("/proxy.pac")),
            fallback: FixedProxy::default(),
        })
        .await
        .unwrap();

    assert_eq!(body.len() as u64, SIZE);
    setup.assert_went_through_the_proxy();
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_broken_pac_script_falls_back_to_the_fixed_proxy() {
    let setup = Setup::new().await;

    let body = setup
        .fetch(ProxyPlan::Pac {
            script: Some(setup.server.url("/missing.pac")),
            fallback: setup.fixed(),
        })
        .await
        .unwrap();

    assert_eq!(body.len() as u64, SIZE);
    setup.assert_went_through_the_proxy();
}

#[test]
fn the_windows_settings_can_be_read() {
    // Whatever this computer is set to, reading it must work and give a usable plan.
    let settings = windows_settings();
    let _ = ProxyPlan::from_settings(&settings);
}
