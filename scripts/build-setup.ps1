# SPDX-License-Identifier: GPL-3.0-or-later

# Builds OmniDM-Setup-<version>.exe into target\release:
#   1. the browser host (omnidm-host.exe) and the command line (omnidm.exe, installed as
#      omnidm-cli.exe: Windows treats omnidm.exe and OmniDm.exe as one name), copied
#      where Tauri's bundler looks for them;
#   2. the app and its NSIS installer (tauri build), with the host, the CLI and the hooks;
#   3. the setup window (installer/), with that NSIS installer inside it.
# Run from anywhere: .\scripts\build-setup.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$triple = ((rustc -vV) | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
$version = ((Get-Content app\src-tauri\tauri.conf.json -Raw) | ConvertFrom-Json).version

Write-Host "1/3  Browser host and command line"
cargo build --release -p omnidm-host -p omnidm-cli
if ($LASTEXITCODE) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force app\src-tauri\binaries | Out-Null
Copy-Item target\release\omnidm-host.exe "app\src-tauri\binaries\omnidm-host-$triple.exe" -Force
Copy-Item target\release\omnidm.exe "app\src-tauri\binaries\omnidm-cli-$triple.exe" -Force

Write-Host "2/3  App and its NSIS installer"
Push-Location app
npm run tauri -- build --config src-tauri/tauri.release.conf.json
$code = $LASTEXITCODE
Pop-Location
if ($code) { exit $code }
$nsis = Get-ChildItem target\release\bundle\nsis\*_x64-setup.exe |
    Where-Object Name -like "*_$version*" | Select-Object -First 1
if (-not $nsis) { throw "Tauri made no NSIS installer for version $version" }

Write-Host "3/3  Setup window"
# What the app takes once installed: the app, the host, the CLI and the uninstaller.
$size = (Get-Item target\release\OmniDm.exe).Length + (Get-Item target\release\omnidm-host.exe).Length +
    (Get-Item target\release\omnidm.exe).Length + 200KB
$env:OMNIDM_SETUP_PAYLOAD = $nsis.FullName
$env:OMNIDM_INSTALLED_SIZE = "$size"
try {
    cargo build --release -p omnidm-setup --features custom-protocol
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
} finally {
    Remove-Item Env:OMNIDM_SETUP_PAYLOAD, Env:OMNIDM_INSTALLED_SIZE
}
$out = "target\release\OmniDM-Setup-$version.exe"
Copy-Item target\release\omnidm-setup.exe $out -Force
Write-Host "Done: $out"
