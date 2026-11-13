; Per-user installer for uConsole for DSPi on Windows (no administrator rights):
; installs to %LOCALAPPDATA%\Programs\uConsole for DSPi, adds a Start menu
; shortcut and an entry in Settings > Apps. Build on Linux or Windows with
;   makensis -DVERSION=0.13.0 -DBUNDLE=<deployed folder> -DOUTFILE=<setup.exe> uconsole-for-dspi.nsi
; (packaging/windows/build-windows.sh does this).
;
; -- WP QP20 -- Every name here is "uConsole for DSPi": the uninstall key, the
; key holding the install folder, the folder and the Start menu shortcut
; (-- WP RN1 --: no earlier install to carry over). The app's own entries (the
; Run value "uConsole for DSPi" for open at login, the settings file
; %APPDATA%\uconsole-for-dspi\uconsole-for-dspi.ini) are the app's and stay as
; they are on an install or an update; the uninstaller removes the Run value
; (-- WP QPI4 --), whose name is the one the app writes (SystemIntegration.cpp's
; kRunName), the settings file and the app's firewall rule (-- WP SWAPFIX --).

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !error "pass -DVERSION=..."
!endif
!ifndef BUNDLE
  !error "pass -DBUNDLE=<folder made by deploy-mingw.sh>"
!endif
!ifndef OUTFILE
  !define OUTFILE "uConsole-for-DSPi-${VERSION}-windows-x86_64-setup.exe"
!endif

!define APP "uConsole for DSPi"
!define EXE "uconsole-for-dspi.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define DIR_KEY "Software\${APP}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define RUN_NAME "${APP}"
; -- WP SWAPFIX -- the app's settings (QSettings, INI: behaviour.md §21)
!define SETTINGS_DIR "uconsole-for-dspi"
!define SETTINGS_FILE "uconsole-for-dspi.ini"

Name "${APP}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${DIR_KEY}" "InstallDir"
SetCompressor /SOLID lzma
VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP} setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"

!define MUI_ICON "${__FILEDIR__}\uconsole-for-dspi.ico"
!define MUI_UNICON "${__FILEDIR__}\uconsole-for-dspi.ico"
; "Run uConsole for DSPi" on the last page, started through the shell like the
; Start menu does. The first start after an install takes several seconds
; (QML compiled into the cache, files scanned by Defender).
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Run ${APP}"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchApp
; The welcome / finish page artwork (164 x 314) instead of NSIS's default
!define MUI_WELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\installer-side.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\installer-side.bmp"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function LaunchApp
  ExecShell "open" "$INSTDIR\${EXE}" "" SW_SHOWNORMAL
FunctionEnd

; -- WP QD21a -- The app must not be running while its files are replaced or
; removed. A running exe can't be opened for writing (Windows maps it), which is
; the case that matters: this copy, also when it only shows in the tray. Retry
; or cancel; a silent run (/S) cancels, with exit code 2.
!macro DSPI_NOT_RUNNING
  retry:
  IfFileExists "$INSTDIR\${EXE}" 0 done
  ClearErrors
  FileOpen $0 "$INSTDIR\${EXE}" a
  IfErrors 0 closed
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION \
    "${APP} is running. Quit it (also from its icon in the notification area), then click Retry." \
    /SD IDCANCEL IDRETRY retry
  SetErrorLevel 2
  Quit
  closed:
  FileClose $0
  done:
!macroend
Function CheckNotRunning
  !insertmacro DSPI_NOT_RUNNING
FunctionEnd
Function un.CheckNotRunning
  !insertmacro DSPI_NOT_RUNNING
FunctionEnd

Section "Install"
  Call CheckNotRunning
  SetOutPath "$INSTDIR"
  ; A previous version's files go first, so no stale Qt modules stay behind
  RMDir /r "$INSTDIR\qml"
  RMDir /r "$INSTDIR\LICENSES"   ; -- WP QD42 -- and no stale licence texts
  File /r "${BUNDLE}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${DIR_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "DSPi unified console maintainers"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

; -- WP SWAPFIX -- The inbound firewall rules named after the app: the one
; the app adds (FirewallController.cpp, named by its display name) and the ones
; Windows adds from its own prompt (named by the exe's FileDescription, the
; same name). Reading them needs no rights; deleting them does, so netsh runs
; elevated (one UAC prompt), and only when such a rule exists. A declined
; prompt leaves the rule; the uninstall goes on.
Function un.RemoveFirewallRule
  nsExec::ExecToStack '"$SYSDIR\netsh.exe" advfirewall firewall show rule name="${APP}"'
  Pop $0
  Pop $1
  StrCmp $0 "0" 0 done
  ExecShellWait "runas" "$SYSDIR\netsh.exe" 'advfirewall firewall delete rule name="${APP}"' SW_HIDE
  done:
FunctionEnd

Section "Uninstall"
  Call un.CheckNotRunning
  Delete "$SMPROGRAMS\${APP}.lnk"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${DIR_KEY}"
  ; -- WP QPI4 -- open at login would start a deleted exe
  DeleteRegValue HKCU "${RUN_KEY}" "${RUN_NAME}"
  ; -- WP SWAPFIX -- the settings file (owner, 2026-11-13); its folder only
  ; when nothing else is in it (the hub store, the AutoEQ cache stay)
  Delete "$APPDATA\${SETTINGS_DIR}\${SETTINGS_FILE}"
  RMDir "$APPDATA\${SETTINGS_DIR}"
  Call un.RemoveFirewallRule
SectionEnd
