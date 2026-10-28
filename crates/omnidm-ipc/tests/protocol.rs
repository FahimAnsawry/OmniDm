// SPDX-License-Identifier: GPL-3.0-or-later

//! The exact JSON shape of protocol messages: the extension (TypeScript) depends on it.

use omnidm_ipc::protocol::{
    BrowserCookie, Capture, CaptureResult, Envelope, Header, Hello, Message, PROTOCOL_VERSION,
    RejectReason,
};
use serde_json::{Value, json};

fn round_trip(envelope: &Envelope) -> Value {
    let text = serde_json::to_string(envelope).unwrap();
    let back: Envelope = serde_json::from_str(&text).unwrap();
    assert_eq!(&back, envelope, "changed in a round trip: {text}");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_envelope_carries_version_id_type_and_payload() {
    let hello = Envelope::new(
        "abc",
        Message::Hello(Hello {
            extension_version: "0.1.0".into(),
            browser: "chrome".into(),
            browser_version: "156".into(),
            protocols: vec![1],
        }),
    );
    assert_eq!(
        round_trip(&hello),
        json!({
            "v": 1,
            "id": "abc",
            "type": "hello",
            "payload": {
                "extensionVersion": "0.1.0",
                "browser": "chrome",
                "browserVersion": "156",
                "protocols": [1]
            }
        })
    );
    assert_eq!(PROTOCOL_VERSION, 1);
}

#[test]
fn a_reply_names_the_message_it_answers() {
    let reply = Envelope::new("r1", Message::Pong).reply_to("abc");
    assert_eq!(
        round_trip(&reply),
        json!({"v": 1, "id": "r1", "re": "abc", "type": "pong"})
    );
}

#[test]
fn a_capture_from_the_extension_is_read() {
    let text = r#"{
        "v": 1, "id": "m7", "type": "capture",
        "payload": {
            "url": "https://example.com/get?id=1",
            "finalUrl": "https://cdn.example.com/a.zip",
            "method": "POST",
            "body": "aWQ9Nw==",
            "referrer": "https://example.com/page",
            "headers": [{"name": "User-Agent", "value": "Mozilla/5.0"}],
            "cookies": [{
                "name": "session", "value": "abc", "domain": ".example.com",
                "hostOnly": false, "path": "/", "secure": true, "httpOnly": true,
                "expirationDate": 1893456000.5
            }],
            "suggestedName": "a.zip",
            "mime": "application/zip",
            "size": 5000000000,
            "private": false,
            "browserDownloadId": 42
        }
    }"#;
    let envelope: Envelope = serde_json::from_str(text).unwrap();
    let Message::Capture(capture) = envelope.message else {
        panic!("not a capture: {:?}", envelope.message);
    };
    assert_eq!(capture.url, "https://example.com/get?id=1");
    assert_eq!(capture.method.as_deref(), Some("POST"));
    assert_eq!(capture.body_bytes().unwrap(), b"id=7");
    assert_eq!(
        capture.headers,
        [Header {
            name: "User-Agent".into(),
            value: "Mozilla/5.0".into()
        }]
    );
    assert_eq!(capture.size, Some(5_000_000_000));
    assert_eq!(capture.browser_download_id, 42);
    let cookie = &capture.cookies[0];
    assert_eq!(
        (cookie.name.as_str(), cookie.domain.as_str()),
        ("session", ".example.com")
    );
    assert!(cookie.secure && cookie.http_only && !cookie.host_only);
    assert_eq!(cookie.expiration_date, Some(1_893_456_000.5));
}

#[test]
fn a_minimal_capture_needs_only_the_url_and_download_id() {
    let text = r#"{"v":1,"id":"m","type":"capture","payload":{"url":"https://e.com/a","browserDownloadId":1}}"#;
    let envelope: Envelope = serde_json::from_str(text).unwrap();
    let Message::Capture(capture) = envelope.message else {
        panic!()
    };
    assert_eq!(capture, Capture::new("https://e.com/a", 1));
    assert_eq!(capture.body_bytes().unwrap(), b"");
}

#[test]
fn a_bad_body_encoding_is_an_error_not_a_panic() {
    let capture = Capture {
        body: Some("%%%not base64".into()),
        ..Capture::new("https://e.com/a", 1)
    };
    assert!(capture.body_bytes().is_err());
}

#[test]
fn capture_results_say_accepted_or_why_not() {
    let accepted = Envelope::new(
        "a",
        Message::CaptureResult(CaptureResult::Accepted { download_id: 9 }),
    );
    assert_eq!(
        round_trip(&accepted)["payload"],
        json!({"outcome": "accepted", "downloadId": 9})
    );
    let rejected = Envelope::new(
        "b",
        Message::CaptureResult(CaptureResult::Rejected {
            reason: RejectReason::ProbeFailed,
            detail: Some("server answered HTTP 403".into()),
        }),
    );
    assert_eq!(
        round_trip(&rejected)["payload"],
        json!({"outcome": "rejected", "reason": "probeFailed", "detail": "server answered HTTP 403"})
    );
}

#[test]
fn unknown_message_types_are_rejected() {
    let text = r#"{"v":1,"id":"x","type":"formatDisk","payload":{}}"#;
    assert!(serde_json::from_str::<Envelope>(text).is_err());
}

#[test]
fn browser_cookies_keep_every_field_the_jar_needs() {
    let cookie = BrowserCookie {
        name: "a".into(),
        value: "1".into(),
        domain: "example.com".into(),
        host_only: true,
        path: "/".into(),
        secure: false,
        http_only: false,
        expiration_date: None,
    };
    let json = serde_json::to_value(&cookie).unwrap();
    assert_eq!(
        json,
        json!({"name": "a", "value": "1", "domain": "example.com", "hostOnly": true,
               "path": "/", "secure": false, "httpOnly": false})
    );
}
