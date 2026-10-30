// SPDX-License-Identifier: GPL-3.0-or-later

//! IPC protocol types, message framing and the named-pipe server and client.

pub mod frame;
#[cfg(windows)]
pub mod pipe;
pub mod protocol;

use std::path::{Path, PathBuf};

/// The OmniDM apps next to `program` (the host or the CLI), newest first: `OmniDm.exe`
/// when installed, `omnidm-app.exe` in a development build. Only these may serve the
/// pipe; the newest is the one to start.
pub fn app_paths(program: &Path) -> Vec<PathBuf> {
    let Some(dir) = program.parent() else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, std::time::SystemTime)> = ["OmniDm.exe", "omnidm-app.exe"]
        .iter()
        .map(|name| dir.join(name))
        .filter_map(|path| {
            let modified = path.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();
    found.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    found.into_iter().map(|(path, _)| path).collect()
}
