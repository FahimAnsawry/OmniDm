// SPDX-License-Identifier: GPL-3.0-or-later

//! Keeping the PC awake while downloading, and waking it from sleep for a scheduled
//! queue (plan §11.2).

use std::io;

use jiff::Timestamp;

/// What the scheduler asks of the PC's power management.
pub trait Power: Send + Sync {
    /// Wakes the PC from sleep at `at`, replacing any earlier request; `None` cancels it.
    fn wake_at(&self, at: Option<Timestamp>) -> io::Result<()>;
    /// While on, the PC doesn't go to sleep by itself (it still can by hand).
    fn keep_awake(&self, on: bool) -> io::Result<()>;
    /// Puts the PC to sleep, or into hibernation. Wake timers stay allowed.
    fn suspend(&self, hibernate: bool) -> io::Result<()>;
    /// Shuts the PC down and turns it off.
    fn shut_down(&self) -> io::Result<()>;
}

/// Does nothing: for tests, and where the Windows APIs aren't available. Sleeping and
/// shutting down fail.
pub struct NoPower;

impl Power for NoPower {
    fn wake_at(&self, _at: Option<Timestamp>) -> io::Result<()> {
        Ok(())
    }

    fn keep_awake(&self, _on: bool) -> io::Result<()> {
        Ok(())
    }

    fn suspend(&self, _hibernate: bool) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn shut_down(&self) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// `at` as a Windows `FILETIME` count: 100 ns steps since 1601-01-01 UTC.
pub fn filetime(at: Timestamp) -> i64 {
    const UNIX_EPOCH_IN_FILETIME_SECONDS: i64 = 11_644_473_600;
    (at.as_second() + UNIX_EPOCH_IN_FILETIME_SECONDS) * 10_000_000
        + i64::from(at.subsec_nanosecond()) / 100
}

#[cfg(windows)]
pub use system::{SystemPower, enable_shutdown_privilege};

#[cfg(windows)]
mod system {
    use std::io;

    use jiff::Timestamp;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LUID,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Power::{
        PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
        SetSuspendState,
    };
    use windows_sys::Win32::System::Shutdown::{
        EWX_FORCEIFHUNG, EWX_POWEROFF, EWX_SHUTDOWN, ExitWindowsEx, SHTDN_REASON_FLAG_PLANNED,
        SHTDN_REASON_MAJOR_APPLICATION, SHTDN_REASON_MINOR_OTHER,
    };
    use windows_sys::Win32::System::Threading::{
        CancelWaitableTimer, CreateWaitableTimerExW, GetCurrentProcess, OpenProcessToken,
        POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0, SetWaitableTimer,
        TIMER_ALL_ACCESS,
    };

    use super::{Power, filetime};

    /// `REASON_CONTEXT`'s only version.
    const POWER_REQUEST_CONTEXT_VERSION: u32 = 0;

    /// A kernel handle, closed when dropped.
    struct Handle(HANDLE);

    // SAFETY: timer and power request handles may be used from any thread.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle is open and owned by `self`.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// The real thing: a wake timer and a "system required" power request.
    pub struct SystemPower {
        timer: Handle,
        request: Handle,
    }

    impl SystemPower {
        pub fn new() -> io::Result<Self> {
            // SAFETY: null attributes and name are allowed; the result is checked.
            let timer = unsafe {
                CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), 0, TIMER_ALL_ACCESS)
            };
            if timer.is_null() {
                return Err(io::Error::last_os_error());
            }
            let timer = Handle(timer);
            // Shown by `powercfg /requests` while downloads keep the PC awake.
            let mut reason: Vec<u16> = "OmniDM is downloading"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let context = REASON_CONTEXT {
                Version: POWER_REQUEST_CONTEXT_VERSION,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: reason.as_mut_ptr(),
                },
            };
            // SAFETY: `context` and its string are valid for the call, which copies them.
            let request = unsafe { PowerCreateRequest(&context) };
            if request.is_null() || request == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                timer,
                request: Handle(request),
            })
        }
    }

    impl Power for SystemPower {
        fn wake_at(&self, at: Option<Timestamp>) -> io::Result<()> {
            // SAFETY: the timer handle is open; `due` outlives the call; no callback.
            let ok = unsafe {
                match at {
                    Some(at) => {
                        // Positive: an absolute UTC time.
                        let due = filetime(at);
                        SetWaitableTimer(self.timer.0, &due, 0, None, std::ptr::null(), 1)
                    }
                    None => CancelWaitableTimer(self.timer.0),
                }
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        fn keep_awake(&self, on: bool) -> io::Result<()> {
            // SAFETY: the request handle is open.
            let ok = unsafe {
                if on {
                    PowerSetRequest(self.request.0, PowerRequestSystemRequired)
                } else {
                    PowerClearRequest(self.request.0, PowerRequestSystemRequired)
                }
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        fn suspend(&self, hibernate: bool) -> io::Result<()> {
            enable_shutdown_privilege()?;
            // Not forced; wake events (our timer included) stay enabled.
            // SAFETY: plain values.
            if unsafe { SetSuspendState(hibernate, false, false) } {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        fn shut_down(&self) -> io::Result<()> {
            enable_shutdown_privilege()?;
            // Apps get to save their work; only hung ones are closed by force.
            let flags = EWX_SHUTDOWN | EWX_POWEROFF | EWX_FORCEIFHUNG;
            let reason = SHTDN_REASON_MAJOR_APPLICATION
                | SHTDN_REASON_MINOR_OTHER
                | SHTDN_REASON_FLAG_PLANNED;
            // SAFETY: plain values.
            if unsafe { ExitWindowsEx(flags, reason) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    /// Sleeping and shutting down need the shutdown privilege, which every user has but
    /// which is off until a program turns it on. Turning it on changes nothing else, so
    /// this also tells whether OmniDM will be allowed to.
    pub fn enable_shutdown_privilege() -> io::Result<()> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the current-process pseudo handle needs no closing; `token` receives a
        // handle that `Handle` closes.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let mut luid = LUID::default();
        // SAFETY: the name is a static NUL-terminated string; `luid` is written.
        if unsafe { LookupPrivilegeValueW(std::ptr::null(), SE_SHUTDOWN_NAME, &mut luid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let wanted = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        // SAFETY: `wanted` is valid; the previous state isn't asked for.
        let ok = unsafe {
            AdjustTokenPrivileges(
                token.0,
                0,
                &wanted,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        // It "succeeds" without the privilege too, reporting that through the last error.
        // SAFETY: no arguments.
        if ok == 0 || unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
            return Err(io::Error::other(
                "this account may not shut down or sleep the PC",
            ));
        }
        Ok(())
    }
}
