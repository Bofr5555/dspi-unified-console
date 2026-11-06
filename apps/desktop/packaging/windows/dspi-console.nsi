; Per-user installer for DSPi Console on Windows (no administrator rights):
; installs to %LOCALAPPDATA%\Programs\DSPi Console, adds a Start menu
; shortcut and an entry in Settings > Apps. Build on Linux or Windows with
;   makensis -DVERSION=1.1.6-beta4 -DBUNDLE=<deployed folder> -DOUTFILE=<setup.exe> dspi-console.nsi
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

Section "Install"
  SetOutPath "$INSTDIR"
  ; A previous version's files go first, so no stale Qt modules stay behind
  RMDir /r "$INSTDIR\qml"
  File /r "${BUNDLE}\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\DSPiConsole.exe"
  WriteRegStr HKCU "Software\DSPi\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "WeebLabs"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\DSPiConsole.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  ; The settings (%APPDATA%\DSPi) stay, as Windows apps usually leave them
  Delete "$SMPROGRAMS\${APP}.lnk"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\DSPi\${APP}"
SectionEnd
