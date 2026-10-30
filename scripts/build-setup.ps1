# SPDX-License-Identifier: GPL-3.0-or-later

# Builds an OmniDM release into target\release\publish\v<version>:
#   1. the browser host (omnidm-host.exe) and the command line (omnidm.exe, installed as
#      omnidm-cli.exe: Windows treats omnidm.exe and OmniDm.exe as one name), copied
#      where Tauri's bundler looks for them;
#   2. the app and its NSIS installer (tauri build), with the host, the CLI and the hooks,
#      plus the installer's update signature;
#   3. the setup window (installer/), with that NSIS installer inside it;
#   4. the browser extension for Chromium browsers and for Firefox;
#   5. the release files: OmniDM-<version>.zip for people (the setup, "How to install.html"
#      and an Extension folder holding OmniDM_chrome and OmniDM_firefox), and for the app's
#      updater the NSIS installer and latest.json. Upload all three to the GitHub release v<version>.
#
# Signing the update needs the private key made with `tauri signer generate`, kept out of
# the repository: %USERPROFILE%\.tauri\omnidm.key and its password in omnidm.key.password
# (or TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD). Without them installed
# copies can't update. Release notes, if any, come from docs\releases\<version>.md.
# Run from anywhere: .\scripts\build-setup.ps1

#Requires -Version 7
# (PowerShell 7 writes UTF-8 without a byte order mark, which latest.json needs.)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$triple = ((rustc -vV) | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
$version = ((Get-Content app\src-tauri\tauri.conf.json -Raw) | ConvertFrom-Json).version
$repo = 'https://github.com/FahimAnsawry/OmniDm'

if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    $key = Join-Path $env:USERPROFILE '.tauri\omnidm.key'
    if (-not (Test-Path $key)) { throw "No update signing key at $key (see the top of this script)" }
    $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $key -Raw
    if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -and (Test-Path "$key.password")) {
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Get-Content "$key.password" -Raw
    }
}

Write-Host "1/5  Browser host and command line"
cargo build --release -p omnidm-host -p omnidm-cli
if ($LASTEXITCODE) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force app\src-tauri\binaries | Out-Null
Copy-Item target\release\omnidm-host.exe "app\src-tauri\binaries\omnidm-host-$triple.exe" -Force
Copy-Item target\release\omnidm.exe "app\src-tauri\binaries\omnidm-cli-$triple.exe" -Force

Write-Host "2/5  App, its NSIS installer and the update signature"
Push-Location app
npm run tauri -- build --config src-tauri/tauri.release.conf.json
$code = $LASTEXITCODE
Pop-Location
if ($code) { exit $code }
$nsis = Get-ChildItem target\release\bundle\nsis\*_x64-setup.exe |
    Where-Object Name -like "*_$version*" | Select-Object -First 1
if (-not $nsis) { throw "Tauri made no NSIS installer for version $version" }
$signature = "$($nsis.FullName).sig"
if (-not (Test-Path $signature)) { throw "Tauri didn't sign $($nsis.Name)" }

Write-Host "3/5  Setup window"
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
$setup = "target\release\OmniDM-Setup-$version.exe"
Copy-Item target\release\omnidm-setup.exe $setup -Force

Write-Host "4/5  Browser extension"
Push-Location extension
npm run build
if (-not $LASTEXITCODE) { npm run build:firefox }
$code = $LASTEXITCODE
Pop-Location
if ($code) { exit $code }

Write-Host "5/5  Release files"
$out = "target\release\publish\v$version"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
$stage = "$out\OmniDM-$version"
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $setup $stage
New-Item -ItemType Directory -Force "$stage\Extension" | Out-Null
Copy-Item -Recurse extension\.output\chrome-mv3 "$stage\Extension\OmniDM_chrome"
Copy-Item -Recurse extension\.output\firefox-mv3 "$stage\Extension\OmniDM_firefox"
(Get-Content docs\install-guide.html -Raw).Replace('{{VERSION}}', $version) |
    Set-Content -NoNewline -Encoding utf8 "$stage\How to install.html"
Compress-Archive -Path "$stage\*" -DestinationPath "$out\OmniDM-$version.zip"
Remove-Item -Recurse -Force $stage

# The updater reads latest.json from the newest release and downloads the NSIS installer
# (not the setup window, which can't run quietly) from beside it.
Copy-Item $nsis.FullName $out
$notes = "docs\releases\$version.md"
$latest = [ordered]@{
    version   = $version
    notes     = if (Test-Path $notes) { (Get-Content $notes -Raw).Trim() } else { '' }
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = @{
        'windows-x86_64' = @{
            signature = (Get-Content $signature -Raw).Trim()
            url       = "$repo/releases/download/v$version/$($nsis.Name)"
        }
    }
}
$latest | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 "$out\latest.json"

Write-Host "Done: $out"
Get-ChildItem $out | ForEach-Object { Write-Host ("  {0}  ({1:N1} MB)" -f $_.Name, ($_.Length / 1MB)) }
