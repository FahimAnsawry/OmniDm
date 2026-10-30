// SPDX-License-Identifier: GPL-3.0-or-later

//! Puts the NSIS installer inside the setup program. scripts/build-setup.ps1 sets
//! OMNIDM_SETUP_PAYLOAD (the installer Tauri made) and OMNIDM_INSTALLED_SIZE (what the
//! app takes on disk, in bytes). Without them the setup window only pretends to install,
//! for working on its pages.

use std::path::PathBuf;

/// Asks Windows to run it as the user, like any app. Without this, Windows guesses from
/// "Setup" in the name that it needs admin rights and asks for them.
const MANIFEST: &str = r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-env-changed=OMNIDM_SETUP_PAYLOAD");
    println!("cargo:rerun-if-env-changed=OMNIDM_INSTALLED_SIZE");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let payload = out.join("payload.exe");
    match std::env::var_os("OMNIDM_SETUP_PAYLOAD") {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            std::fs::copy(&path, &payload).expect("OMNIDM_SETUP_PAYLOAD can be read");
        }
        None => std::fs::write(&payload, []).expect("OUT_DIR is writable"),
    }
    let size = std::env::var("OMNIDM_INSTALLED_SIZE").unwrap_or_else(|_| "20000000".into());
    println!("cargo:rustc-env=OMNIDM_INSTALLED_SIZE={size}");

    let windows = tauri_build::WindowsAttributes::new().app_manifest(MANIFEST);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
