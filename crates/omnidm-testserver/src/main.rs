// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs the OmniDM test server by hand, with a few sample files, until Ctrl+C.
//!
//! `--e2e` serves the browser end-to-end fixtures instead (see `extension/e2e/`) and
//! prints `E2E <base URL>` once ready.

use omnidm_testserver::{FileSpec, TestServer};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let server = TestServer::start().await?;
    if std::env::args().any(|arg| arg == "--e2e") {
        e2e_fixtures(&server);
        println!("E2E {}", server.url(""));
    } else {
        sample_files(&server);
    }
    tokio::signal::ctrl_c().await
}

fn sample_files(server: &TestServer) {
    let files = [
        ("/10m.bin", FileSpec::new(10 * MIB).etag("v1")),
        ("/1g.bin", FileSpec::new(1024 * MIB).etag("v1")),
        (
            "/slow.bin",
            FileSpec::new(50 * MIB).etag("v1").throttle(MIB),
        ),
        ("/no-ranges.bin", FileSpec::new(10 * MIB).no_ranges()),
        (
            "/unknown-size.bin",
            FileSpec::new(10 * MIB).unknown_length(),
        ),
        (
            "/flaky.bin",
            FileSpec::new(10 * MIB).etag("v1").fail(503, 2, Some(1)),
        ),
        ("/login.bin", FileSpec::new(MIB).html_instead()),
    ];
    println!("OmniDM test server running. Press Ctrl+C to stop.");
    for (path, spec) in files {
        println!("  {}", server.url(path));
        server.set_file(path, spec);
    }
}

/// A page of download links, each set up to prove one part of the browser handoff.
fn e2e_fixtures(server: &TestServer) {
    let page = server.url("/e2e/page.html");
    // Slow enough that the extension pauses the browser's copy before it finishes.
    let file = |name: &str| {
        FileSpec::new(3 * MIB)
            .etag(name)
            .throttle(512 * KIB)
            .content_disposition(&format!("attachment; filename=\"{name}\""))
    };
    server.set_file("/e2e/plain.bin", file("plain.bin"));
    // Only with the login cookie: success proves cookies were passed on.
    server.set_file(
        "/e2e/protected.bin",
        file("protected.bin").require_cookie("session=e2e"),
    );
    // Only from the page: success proves the referrer was passed on.
    server.set_file(
        "/e2e/referred.bin",
        file("referred.bin").require_header("referer", &page),
    );
    // Only as a POST with the form's data.
    server.set_file("/e2e/form.bin", file("form.bin").post_only("id=7"));
    // Works once: OmniDM's probe is refused, so the browser must keep the download.
    server.set_file("/e2e/expiring.bin", file("expiring.bin").expire_after(1));
    // Logging in sets the cookie, then shows the page.
    server.redirect_setting_cookie("/e2e/login", "/e2e/page.html", 302, "session=e2e; Path=/");
    server.set_text(
        "/e2e/page.html",
        "text/html; charset=utf-8",
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>OmniDM e2e</title></head>
<body>
  <h1>OmniDM end-to-end fixtures</h1>
  <p><a id="plain" href="/e2e/plain.bin">Plain file</a></p>
  <p><a id="protected" href="/e2e/protected.bin">Needs the login cookie</a></p>
  <p><a id="referred" href="/e2e/referred.bin">Needs this page as referrer</a></p>
  <p><a id="expiring" href="/e2e/expiring.bin">Works only once</a></p>
  <form method="post" action="/e2e/form.bin">
    <input type="hidden" name="id" value="7">
    <button id="form" type="submit">Download from a form</button>
  </form>
</body>
</html>
"#,
    );
}
