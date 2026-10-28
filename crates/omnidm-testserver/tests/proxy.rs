// SPDX-License-Identifier: GPL-3.0-or-later

use omnidm_testserver::{FileSpec, TestProxy, TestServer};
use reqwest::StatusCode;

const SIZE: u64 = 50_000;

async fn origin() -> TestServer {
    let server = TestServer::start().await.unwrap();
    server.set_file("/f.bin", FileSpec::new(SIZE));
    server
}

fn client_via(proxy: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy).unwrap())
        .build()
        .unwrap()
}

#[tokio::test]
async fn http_proxy_forwards_and_logs_the_target() {
    let server = origin().await;
    let proxy = TestProxy::start(None).await.unwrap();

    let resp = client_via(&proxy.url())
        .get(server.url("/f.bin"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().len() as u64, SIZE);
    let log = proxy.connections();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].target, server.addr().to_string());
    assert!(log[0].authorized);
}

#[tokio::test]
async fn http_proxy_with_a_password_answers_407_without_it() {
    let server = origin().await;
    let proxy = TestProxy::start(Some(("me", "secret"))).await.unwrap();

    let without = client_via(&proxy.url())
        .get(server.url("/f.bin"))
        .send()
        .await
        .unwrap();
    assert_eq!(without.status(), StatusCode::PROXY_AUTHENTICATION_REQUIRED);

    let with = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all(proxy.url())
                .unwrap()
                .basic_auth("me", "secret"),
        )
        .build()
        .unwrap()
        .get(server.url("/f.bin"))
        .send()
        .await
        .unwrap();
    assert_eq!(with.status(), StatusCode::OK);
    let log = proxy.connections();
    assert_eq!(
        log.iter().map(|c| c.authorized).collect::<Vec<_>>(),
        [false, true]
    );
}

#[tokio::test]
async fn any_host_name_leads_to_this_computer() {
    let server = origin().await;
    let proxy = TestProxy::start(None).await.unwrap();
    let made_up = format!("http://files.omnidm.test:{}/f.bin", server.addr().port());

    let resp = client_via(&proxy.url()).get(&made_up).send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        proxy.connections()[0].target,
        format!("files.omnidm.test:{}", server.addr().port())
    );
}
