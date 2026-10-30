; SPDX-License-Identifier: GPL-3.0-or-later

; Extra steps for Tauri's NSIS installer. OmniDM-Setup.exe runs the installer silently and
; passes the choices from its Options screen as flags:
;   /NODESKTOP, /NOSTARTMENU      remove the shortcut that silent installs always create
;   /AUTOSTART, /NOAUTOSTART      start OmniDM with Windows, or not (no flag: left as it is)
; and OmniDm.exe --uninstall runs the uninstaller silently with:
;   /DELETEDATA                   also delete the download list and settings
; The browser extension's link (the native messaging host) is always registered, and the
; install folder is put on the user's PATH so `omnidm-cli` works in any terminal.

; Adds (ADD = 1) or removes (ADD = 0) the install folder in the user's PATH. The registry
; value is read unexpanded and written back as REG_EXPAND_SZ, so entries like
; %USERPROFILE%\bin survive. The folder goes through an environment variable, so no
; character in it needs quoting.
!macro OMNIDM_USER_PATH ADD
  System::Call 'Kernel32::SetEnvironmentVariable(t "OMNIDM_DIR", t "$INSTDIR")i'
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); $$p = $$k.GetValue('Path', '', 'DoNotExpandEnvironmentNames'); $$d = $$env:OMNIDM_DIR.TrimEnd('\'); $$l = @($$p -split ';' | Where-Object { $$_ -and $$_.TrimEnd('\') -ne $$d }); if (${ADD}) { $$l += $$d }; $$k.SetValue('Path', ($$l -join ';'), 'ExpandString')"`
  Pop $R0
  ; Tells open programs (Explorer, new terminals) that PATH changed.
  SendMessage 0xFFFF 0x1A 0 "STR:Environment" /TIMEOUT=5000
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; The installer closes OmniDm.exe itself, but a browser keeps the host running and a
  ; terminal may be running the CLI: both would stay locked and silently not be replaced.
  ; The extension starts the host again when it next needs the app.
  nsExec::Exec 'taskkill /F /IM omnidm-host.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /IM omnidm-cli.exe'
  Pop $R0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro OMNIDM_USER_PATH 1

  ; Lets Chrome, Edge, Chromium and Firefox start the host, which connects the
  ; extension to OmniDM.
  ExecWait '"$INSTDIR\omnidm-host.exe" --register'

  ${GetOptions} $CMDLINE "/NODESKTOP" $R0
  ${IfNot} ${Errors}
    Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}

  ${GetOptions} $CMDLINE "/NOSTARTMENU" $R0
  ${IfNot} ${Errors}
    Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  ${EndIf}

  ; The same value the app's own "Start with Windows" setting writes
  ; (tauri-plugin-autostart), so the setting shows it as on.
  ${GetOptions} $CMDLINE "/AUTOSTART" $R0
  ${IfNot} ${Errors}
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe --background"
  ${EndIf}
  ${GetOptions} $CMDLINE "/NOAUTOSTART" $R0
  ${IfNot} ${Errors}
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
  ${EndIf}

  ; Windows' Apps list opens OmniDM's own uninstall window; uninstall.exe stays for
  ; silent uninstalls.
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --uninstall"
  WriteRegStr SHCTX "${UNINSTKEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ; First the browsers lose the way to start the host, then a running host is
    ; stopped, so nothing starts OmniDM again while it is being removed.
    ExecWait '"$INSTDIR\omnidm-host.exe" --unregister'
    nsExec::Exec 'taskkill /F /IM omnidm-host.exe'
    Pop $R0
    !insertmacro OMNIDM_USER_PATH 0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${GetOptions} $CMDLINE "/DELETEDATA" $R0
  ${IfNot} ${Errors}
  ${AndIf} $UpdateMode <> 1
    ; The data folder keeps its internal name; see app/src-tauri/src/lib.rs (data_dir).
    RmDir /r "$APPDATA\OmniDm"
    RmDir /r "$LOCALAPPDATA\${BUNDLEID}"
  ${EndIf}
!macroend
