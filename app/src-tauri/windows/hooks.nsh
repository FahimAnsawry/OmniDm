; SPDX-License-Identifier: GPL-3.0-or-later

; Extra steps for Tauri's NSIS installer. OmniDM-Setup.exe runs the installer silently and
; passes the choices from its Options screen as flags:
;   /NODESKTOP, /NOSTARTMENU      remove the shortcut that silent installs always create
;   /AUTOSTART, /NOAUTOSTART      start OmniDM with Windows, or not (no flag: left as it is)
; and OmniDm.exe --uninstall runs the uninstaller silently with:
;   /DELETEDATA                   also delete the download list and settings
; The browser extension's link (the native messaging host) is always registered.

!macro NSIS_HOOK_POSTINSTALL
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
