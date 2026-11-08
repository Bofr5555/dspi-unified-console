; Per-user installer for DSPi Console on Windows (no administrator rights):
; installs to %LOCALAPPDATA%\Programs\DSPi Console, adds a Start menu
; shortcut and an entry in Settings > Apps. Build on Linux or Windows with
;   makensis -DVERSION=1.1.6 -DBUNDLE=<deployed folder> -DOUTFILE=<setup.exe> dspi-console.nsi
; (packaging/windows/build-windows.sh does this).

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !error "pass -DVERSION=..."
!endif
!ifndef BUNDLE
  !error "pass -DBUNDLE=<folder made by deploy-mingw.sh>"
!endif
!ifndef OUTFILE
  !define OUTFILE "DSPi-Console-${VERSION}-windows-x86_64-setup.exe"
!endif

!define APP "DSPi Console"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\DSPiConsole"

Name "${APP}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "Software\DSPi\${APP}" "InstallDir"
SetCompressor /SOLID lzma

!define MUI_ICON "${__FILEDIR__}\dspi-console.ico"
!define MUI_UNICON "${__FILEDIR__}\dspi-console.ico"
; "Run DSPi Console" on the last page, started through the shell like the
; Start menu does. The first start after an install takes several seconds
; (QML compiled into the cache, files scanned by Defender).
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Run DSPi Console"
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
  ExecShell "open" "$INSTDIR\DSPiConsole.exe" "" SW_SHOWNORMAL
FunctionEnd

; -- WP QD21a -- DSPi Console must not be running while its files are replaced or
; removed. A running exe can't be opened for writing (Windows maps it), which is
; the case that matters: this copy, also when it only shows in the tray. Retry
; or cancel; a silent run (/S) cancels, with exit code 2.
!macro DSPI_NOT_RUNNING
  retry:
  IfFileExists "$INSTDIR\DSPiConsole.exe" 0 done
  ClearErrors
  FileOpen $0 "$INSTDIR\DSPiConsole.exe" a
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
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\DSPiConsole.exe"
  WriteRegStr HKCU "Software\DSPi\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "DSPi unified console maintainers"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\DSPiConsole.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  ; The settings (%APPDATA%\DSPi) stay, as Windows apps usually leave them
  Call un.CheckNotRunning
  Delete "$SMPROGRAMS\${APP}.lnk"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\DSPi\${APP}"
SectionEnd
