// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs the OmniDm test server by hand, with a few sample files, until Ctrl+C.

use omnidm_testserver::{FileSpec, TestServer};

const MIB: u64 = 1024 * 1024;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let server = TestServer::start().await?;
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
    println!("OmniDm test server running. Press Ctrl+C to stop.");
    for (path, spec) in files {
        println!("  {}", server.url(path));
        server.set_file(path, spec);
    }
    tokio::signal::ctrl_c().await
}
