; Per-user installer for uConsole for DSPi on Windows (no administrator rights):
; installs to %LOCALAPPDATA%\Programs\uConsole for DSPi, adds a Start menu
; shortcut and an entry in Settings > Apps. Build on Linux or Windows with
;   makensis -DVERSION=0.13.0 -DBUNDLE=<deployed folder> -DOUTFILE=<setup.exe> dspi-console.nsi
; (packaging/windows/build-windows.sh does this).
;
; -- WP QP20 -- The name people see is "uConsole for DSPi". What an earlier
; install wrote keeps its name, so this installer updates it in place: the
; uninstall key (DSPiuConsole), the key holding the install folder, and the
; folder itself when one is recorded there. The earlier Start menu shortcut
; ("DSPi uConsole.lnk") is replaced by the new one. The app's own entries
; (the Run value "DSPi uConsole" for open at login, the settings under
; %APPDATA%\DSPi) are the app's and stay as they are.

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !error "pass -DVERSION=..."
!endif
!ifndef BUNDLE
  !error "pass -DBUNDLE=<folder made by deploy-mingw.sh>"
!endif
!ifndef OUTFILE
  !define OUTFILE "DSPi-uConsole-${VERSION}-windows-x86_64-setup.exe"
!endif

!define APP "uConsole for DSPi"
!define EXE "DSPiConsole.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\DSPiuConsole"
!define DIR_KEY "Software\DSPi\DSPi uConsole"
!define OLD_LINK "DSPi uConsole.lnk"

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

!define MUI_ICON "${__FILEDIR__}\dspi-console.ico"
!define MUI_UNICON "${__FILEDIR__}\dspi-console.ico"
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
  Delete "$SMPROGRAMS\${OLD_LINK}"
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

Section "Uninstall"
  ; The settings (%APPDATA%\DSPi) stay, as Windows apps usually leave them
  Call un.CheckNotRunning
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$SMPROGRAMS\${OLD_LINK}"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${DIR_KEY}"
SectionEnd
