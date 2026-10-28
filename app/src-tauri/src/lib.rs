// SPDX-License-Identifier: GPL-3.0-or-later

//! OmniDm desktop app: connects the engine crates to the UI.

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running OmniDm");
}
