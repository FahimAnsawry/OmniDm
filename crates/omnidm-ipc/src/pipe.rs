// SPDX-License-Identifier: GPL-3.0-or-later

//! The named pipe `\\.\pipe\OmniDm.<SessionId>` between the app and its clients (plan §8.1).
//!
//! - Only the current Windows user may open it, and remote clients are refused.
//! - The first instance is created exclusively, so it doubles as the single-instance
//!   lock: a second app learns that one is already running.
//! - Clients can check that the server is the expected program before trusting it.

use std::ffi::c_void;
use std::fmt;
use std::io;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::core::PWSTR;

#[derive(Debug)]
pub enum PipeError {
    /// No app is listening on the pipe.
    NotRunning,
    /// Another app already owns the pipe.
    AlreadyRunning,
    /// The program serving the pipe is not the expected one.
    Impostor {
        found: PathBuf,
    },
    Io(io::Error),
}

impl fmt::Display for PipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => write!(f, "OmniDM is not running"),
            Self::AlreadyRunning => write!(f, "OmniDM is already running"),
            Self::Impostor { found } => {
                write!(f, "the pipe is served by {}, not OmniDM", found.display())
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PipeError {}

impl From<io::Error> for PipeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// `\\.\pipe\OmniDm.<SessionId>`: one app per Windows session.
pub fn default_pipe_name() -> String {
    let mut session = 0u32;
    // SAFETY: plain call with a valid out-parameter; on failure `session` stays 0.
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
    }
    format!(r"\\.\pipe\OmniDm.{session}")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copies a string Windows allocated with `LocalAlloc`, then frees it.
///
/// # Safety
/// `p` is a NUL-terminated string from `LocalAlloc` that is not used afterwards.
unsafe fn take_local_string(p: PWSTR) -> String {
    // SAFETY: guaranteed by the caller.
    unsafe {
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        LocalFree(p.cast());
        text
    }
}

/// The pipe's access rules: a protected DACL allowing only the current user.
pub fn current_user_sddl() -> io::Result<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the token handle is closed below; the buffer is sized by the first call
    // and aligned for TOKEN_USER (u64 elements); the SID string is freed by
    // `take_local_string`.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut size = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        );
        let error = io::Error::last_os_error();
        CloseHandle(token);
        if ok == 0 {
            return Err(error);
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid: PWSTR = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(format!("D:P(A;;GA;;;{})", take_local_string(sid)))
    }
}

/// A security descriptor built from SDDL, freed on drop.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    fn for_current_user() -> io::Result<Self> {
        let sddl = wide(&current_user_sddl()?);
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; the descriptor is freed in `drop`.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn create_instance(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let descriptor = SecurityDescriptor::for_current_user()?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: `attributes` and the descriptor it points to live until the call returns,
    // and the pipe keeps its own copy of the security settings.
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
        )
    }
}

/// The app's end: accepts any number of clients, one pipe instance each.
pub struct PipeListener {
    name: String,
    next: NamedPipeServer,
}

impl PipeListener {
    /// Creates the first instance of the pipe; fails with
    /// [`PipeError::AlreadyRunning`] if another server owns it.
    pub fn bind(name: &str) -> Result<Self, PipeError> {
        match create_instance(name, true) {
            Ok(next) => Ok(Self {
                name: name.to_string(),
                next,
            }),
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                Err(PipeError::AlreadyRunning)
            }
            Err(e) => Err(PipeError::Io(e)),
        }
    }

    /// Waits for the next client.
    pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        self.next.connect().await?;
        let fresh = create_instance(&self.name, false)?;
        Ok(std::mem::replace(&mut self.next, fresh))
    }
}

/// How long to keep trying while every pipe instance is busy.
const BUSY_WAIT: Duration = Duration::from_secs(2);

/// Connects to the app. Unless `allowed` is empty, the server must be one of those
/// programs.
pub async fn connect(name: &str, allowed: &[PathBuf]) -> Result<NamedPipeClient, PipeError> {
    let deadline = Instant::now() + BUSY_WAIT;
    let client = loop {
        match ClientOptions::new().open(name) {
            Ok(client) => break client,
            Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) => {
                return Err(PipeError::NotRunning);
            }
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                    && Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => return Err(PipeError::Io(e)),
        }
    };
    check_server(client.as_raw_handle(), allowed)?;
    Ok(client)
}

fn check_server(pipe: RawHandle, allowed: &[PathBuf]) -> Result<(), PipeError> {
    if allowed.is_empty() {
        return Ok(());
    }
    let found = server_program(pipe)?;
    if allowed.iter().any(|program| same_file(&found, program)) {
        Ok(())
    } else {
        Err(PipeError::Impostor { found })
    }
}

/// The full path of the program serving `pipe`.
fn server_program(pipe: RawHandle) -> io::Result<PathBuf> {
    let mut pid = 0u32;
    // SAFETY: `pipe` is an open pipe handle; the process handle is closed below and the
    // name buffer is large enough for any Windows path.
    unsafe {
        if GetNamedPipeServerProcessId(pipe as HANDLE, &mut pid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u16; 32_768];
        let mut len = buffer.len() as u32;
        let ok =
            QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len);
        let error = io::Error::last_os_error();
        CloseHandle(process);
        if ok == 0 {
            return Err(error);
        }
        Ok(PathBuf::from(String::from_utf16_lossy(
            &buffer[..len as usize],
        )))
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
    }
}
