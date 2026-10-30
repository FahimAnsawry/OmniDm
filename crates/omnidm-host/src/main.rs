// SPDX-License-Identifier: GPL-3.0-or-later

//! Native messaging host that connects the OmniDM browser extension to the app.
//!
//! Browsers start it with their own arguments (Chrome: the extension's origin; Firefox:
//! the manifest path and add-on ID), which it ignores. `--register` and `--unregister`
//! set it up for Chrome, Edge, Chromium and Firefox for the current user.

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    use std::time::Duration;

    use omnidm_host::{BROWSER_KEYS, app_paths, connect_app, register, relay, unregister};

    let Ok(host_exe) = std::env::current_exe() else {
        eprintln!("omnidm-host: can't find its own location");
        return ExitCode::FAILURE;
    };
    let keys: Vec<&str> = BROWSER_KEYS.iter().map(|(_, key)| *key).collect();
    match std::env::args().nth(1).as_deref() {
        Some("--register") => {
            return report(
                register(&host_exe, &keys),
                "registered with Chrome, Edge, Chromium and Firefox",
            );
        }
        Some("--unregister") => {
            return report(unregister(&host_exe, &keys), "unregistered");
        }
        _ => {}
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("omnidm-host: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async {
        // OMNIDM_PIPE is for development and tests; browsers never set it.
        let pipe_name =
            std::env::var("OMNIDM_PIPE").unwrap_or_else(|_| omnidm_ipc::pipe::default_pipe_name());
        // Only an OmniDM app next to the host may serve the pipe; without one, nothing is
        // trusted (an empty list would switch the check off).
        let apps = app_paths(&host_exe);
        let Some(newest) = apps.first().cloned() else {
            eprintln!("omnidm-host: OmniDm.exe is not next to the host");
            return ExitCode::FAILURE;
        };
        let start = || {
            std::process::Command::new(&newest)
                .arg("--background")
                .spawn()
                .map(drop)
        };
        let pipe = match connect_app(&pipe_name, &apps, start, Duration::from_secs(10)).await {
            Ok(pipe) => pipe,
            Err(e) => {
                // Exiting closes the connection, so the extension hands the
                // download back to the browser at once.
                eprintln!("omnidm-host: can't reach OmniDM: {e}");
                return ExitCode::FAILURE;
            }
        };
        match relay(tokio::io::stdin(), tokio::io::stdout(), pipe).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("omnidm-host: {e}");
                ExitCode::FAILURE
            }
        }
    })
}

#[cfg(windows)]
fn report(result: std::io::Result<()>, done: &str) -> ExitCode {
    match result {
        Ok(()) => {
            println!("omnidm-host: {done}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("omnidm-host: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("omnidm-host: only Windows is supported for now");
    ExitCode::FAILURE
}
