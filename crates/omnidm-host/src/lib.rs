// SPDX-License-Identifier: GPL-3.0-or-later

//! Native messaging host: the browser starts it, and it relays the extension's messages
//! to the OmniDm app over the named pipe (plan §10). It starts the app when needed and
//! registers itself with the browsers.

use std::io;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::{Duration, Instant};

use omnidm_ipc::frame::{
    FrameError, MAX_FROM_EXTENSION, MAX_TO_EXTENSION, read_frame_async, write_frame_async,
};
#[cfg(windows)]
use omnidm_ipc::pipe::{PipeError, connect};
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient;

/// The host's name in browser registrations (plan §3).
pub const HOST_NAME: &str = "io.github.fahimansawry.omnidm.host";
/// The Firefox add-on ID (plan §3).
pub const FIREFOX_EXTENSION_ID: &str = "omnidm@fahimansawry.github.io";
/// Chromium extension IDs allowed to connect: the development ID fixed by the manifest
/// `key` in `extension/wxt.config.ts`. The store ID is added when it is assigned (M7).
pub const CHROMIUM_EXTENSION_IDS: &[&str] = &["mohldghkdbkpfldhfmfndfcamcailkai"];

/// Where each browser looks up native messaging hosts, under `HKEY_CURRENT_USER`.
pub const BROWSER_KEYS: &[(&str, &str)] = &[
    ("chrome", r"Software\Google\Chrome\NativeMessagingHosts"),
    ("edge", r"Software\Microsoft\Edge\NativeMessagingHosts"),
    ("chromium", r"Software\Chromium\NativeMessagingHosts"),
    ("firefox", r"Software\Mozilla\NativeMessagingHosts"),
];

/// Relays frames between the browser (`browser_in`/`browser_out`) and the app until
/// either side closes. Frames are passed on unread. A frame over the browser's 1 MiB limit
/// is dropped (the browser would end the connection); one over 4 MiB from the browser
/// ends the relay with an error.
pub async fn relay<I, O, A>(browser_in: I, browser_out: O, app: A) -> io::Result<()>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    A: AsyncRead + AsyncWrite,
{
    let (mut app_in, mut app_out) = tokio::io::split(app);
    let mut browser_in = browser_in;
    let mut browser_out = browser_out;
    let to_app = async {
        while let Some(frame) = read_frame_async(&mut browser_in, MAX_FROM_EXTENSION)
            .await
            .map_err(into_io)?
        {
            write_frame_async(&mut app_out, &frame)
                .await
                .map_err(into_io)?;
        }
        Ok::<(), io::Error>(())
    };
    let to_browser = async {
        loop {
            match read_frame_async(&mut app_in, u32::MAX)
                .await
                .map_err(into_io)?
            {
                None => return Ok::<(), io::Error>(()),
                Some(frame) if frame.len() > MAX_TO_EXTENSION as usize => {
                    eprintln!("omnidm-host: dropped a {}-byte message", frame.len());
                }
                Some(frame) => write_frame_async(&mut browser_out, &frame)
                    .await
                    .map_err(into_io)?,
            }
        }
    };
    // Whichever direction ends first ends the relay.
    tokio::select! {
        result = to_app => result,
        result = to_browser => result,
    }
}

fn into_io(e: FrameError) -> io::Error {
    match e {
        FrameError::Io(e) => e,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

/// The OmniDm apps next to the host, newest first (shared with the CLI).
pub use omnidm_ipc::app_paths;

/// Connects to the app, starting it with `start` if it isn't running, and waits up to
/// `wait` for it. Unless `allowed` is empty, the pipe must be served by one of those.
#[cfg(windows)]
pub async fn connect_app(
    pipe_name: &str,
    allowed: &[PathBuf],
    start: impl FnOnce() -> io::Result<()>,
    wait: Duration,
) -> Result<NamedPipeClient, PipeError> {
    match connect(pipe_name, allowed).await {
        Err(PipeError::NotRunning) => {}
        other => return other,
    }
    start()?;
    let deadline = Instant::now() + wait;
    loop {
        match connect(pipe_name, allowed).await {
            Err(PipeError::NotRunning) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            other => return other,
        }
    }
}

/// The manifest Chromium browsers read (Chrome, Edge, Brave, Vivaldi, Opera).
pub fn chromium_manifest(host_exe: &Path) -> String {
    let origins: Vec<String> = CHROMIUM_EXTENSION_IDS
        .iter()
        .map(|id| format!("chrome-extension://{id}/"))
        .collect();
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "OmniDm native messaging host",
        "path": host_exe,
        "type": "stdio",
        "allowed_origins": origins,
    });
    serde_json::to_string_pretty(&manifest).expect("a JSON value serialises")
}

/// The manifest Firefox reads.
pub fn firefox_manifest(host_exe: &Path) -> String {
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "OmniDm native messaging host",
        "path": host_exe,
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID],
    });
    serde_json::to_string_pretty(&manifest).expect("a JSON value serialises")
}

/// Writes both manifests next to `host_exe` and points each browser key in `keys`
/// (paths under `HKEY_CURRENT_USER`, as in [`BROWSER_KEYS`]) at the right one. Keys whose
/// path contains `Mozilla` get the Firefox manifest.
#[cfg(windows)]
pub fn register(host_exe: &Path, keys: &[&str]) -> io::Result<()> {
    let dir = host_exe
        .parent()
        .ok_or_else(|| io::Error::other("the host has no folder"))?;
    let chromium = dir.join("host-chromium.json");
    let firefox = dir.join("host-firefox.json");
    std::fs::write(&chromium, chromium_manifest(host_exe))?;
    std::fs::write(&firefox, firefox_manifest(host_exe))?;
    for key in keys {
        let manifest = if key.contains("Mozilla") {
            &firefox
        } else {
            &chromium
        };
        registry::set_default(&format!(r"{key}\{HOST_NAME}"), &manifest.to_string_lossy())?;
    }
    Ok(())
}

/// Undoes [`register`]: removes the browser keys and the manifests.
#[cfg(windows)]
pub fn unregister(host_exe: &Path, keys: &[&str]) -> io::Result<()> {
    for key in keys {
        registry::delete(&format!(r"{key}\{HOST_NAME}"))?;
    }
    if let Some(dir) = host_exe.parent() {
        for name in ["host-chromium.json", "host-firefox.json"] {
            match std::fs::remove_file(dir.join(name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
mod registry {
    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check(status: u32) -> io::Result<()> {
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }

    /// Sets the default value of `HKEY_CURRENT_USER\<key>`, creating the key.
    pub(super) fn set_default(key: &str, value: &str) -> io::Result<()> {
        let key = wide(key);
        let value = wide(value);
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: the strings are NUL-terminated and outlive the calls; the key handle is
        // closed before returning; the value size counts its bytes, NUL included.
        unsafe {
            check(RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            ))?;
            let result = check(RegSetValueExW(
                handle,
                std::ptr::null(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            ));
            RegCloseKey(handle);
            result
        }
    }

    /// Deletes `HKEY_CURRENT_USER\<key>` and everything under it; a missing key is fine.
    pub(super) fn delete(key: &str) -> io::Result<()> {
        let key = wide(key);
        // SAFETY: `key` is NUL-terminated and outlives the call.
        let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(status)
    }
}
