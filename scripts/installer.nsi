Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!ifndef APP_REG_NAME
  !define APP_REG_NAME "vault"
!endif
!ifndef APP_DEFAULT_DIR
  !define APP_DEFAULT_DIR "D:\vault"
!endif
Name "${APP_REG_NAME}"
OutFile "${ARTIFACT_DIR}\vault-${APP_VERSION}-setup.exe"
InstallDir "${APP_DEFAULT_DIR}"
InstallDirRegKey HKCU "Software\${APP_REG_NAME}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
!define MUI_ABORTWARNING
!define MUI_ICON "..\src-tauri\icons\icon.ico"
!define MUI_UNICON "..\src-tauri\icons\icon.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PACKAGE_DIR}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\vault.exe"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchInstalled
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "${APP_REG_NAME}"
VIAddVersionKey /LANG=2052 "FileDescription" "${APP_REG_NAME} 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "panda-student · Apache-2.0"

Function .onInit
  SetShellVarContext current
  ${If} $INSTDIR == "D:\vault"
    StrCpy $INSTDIR "D:\vault"
    ${IfNot} ${FileExists} "D:\*.*"
      StrCpy $INSTDIR "$LOCALAPPDATA\Programs\vault"
    ${EndIf}
  ${EndIf}
FunctionEnd

!macro DefineEnsureClosed Prefix
Function ${Prefix}EnsureClosed
  IfFileExists "$INSTDIR\vault.exe" 0 done
  ClearErrors
  FileOpen $0 "$INSTDIR\vault.exe" a
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONSTOP "无法修改程序文件。请先退出 vault 并断开 MCP，确认目录可写后重试。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  FileClose $0
  done:
FunctionEnd
!macroend
!insertmacro DefineEnsureClosed "un."

Function LaunchInstalled
  ; Replacement already restarted any previously running desktop windows.
  IfFileExists "$PLUGINSDIR\install.restarted" done
  Exec '"$INSTDIR\vault.exe"'
  done:
FunctionEnd

Function un.onInit
  Call un.EnsureClosed
FunctionEnd

Section "vault" Main
  SetShellVarContext current
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=vault-helper.exe "${PACKAGE_DIR}\vault.exe"
  SetOutPath "$INSTDIR"
  File "${PACKAGE_DIR}\README.txt"
  File "${PACKAGE_DIR}\LICENSE.txt"
  File "${PACKAGE_DIR}\NOTICE.txt"
  File "${PACKAGE_DIR}\THIRD_PARTY_NOTICES.txt"
  SetOutPath "$INSTDIR\third-party-sources"
  File "${PACKAGE_DIR}\third-party-sources\*"
  SetOutPath "$INSTDIR"
  DetailPrint "等待操作完成，自动退出旧版并恢复 MCP..."
  ; The replacement coordinator is a GUI-subsystem program, not a console filter.
  ; Status is written to JSON; only the actual exit code belongs in this adapter.
  ClearErrors
  ExecWait '"$PLUGINSDIR\vault-helper.exe" --installer-replace "$INSTDIR\vault.exe" "$PLUGINSDIR\install.json"' $0
  ${If} ${Errors}
    StrCpy $0 1
  ${EndIf}
  ${If} $0 != 0
    DetailPrint "程序替换未完成，协调器退出码：$0"
    MessageBox MB_OK|MB_ICONSTOP "安装未完成，旧程序和 MCP 将自动恢复。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\${APP_REG_NAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "DisplayName" "vault"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "Publisher" "panda-student"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "DisplayIcon" "$INSTDIR\vault.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}" "NoRepair" 1
  CreateDirectory "$SMPROGRAMS\${APP_REG_NAME}"
  CreateShortcut "$SMPROGRAMS\${APP_REG_NAME}\vault.lnk" "$INSTDIR\vault.exe"
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  ; Delete only installed files. Never remove the user's data directory.
  Delete "$INSTDIR\vault.exe"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\NOTICE.txt"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  Delete "$INSTDIR\third-party-sources\cssparser-0.37.0.crate"
  Delete "$INSTDIR\third-party-sources\cssparser-macros-0.7.1.crate"
  Delete "$INSTDIR\third-party-sources\dtoa-short-0.3.5.crate"
  Delete "$INSTDIR\third-party-sources\option-ext-0.2.0.crate"
  Delete "$INSTDIR\third-party-sources\selectors-0.38.0.crate"
  Delete "$INSTDIR\third-party-sources\README.md"
  RMDir "$INSTDIR\third-party-sources"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\${APP_REG_NAME}\vault.lnk"
  RMDir "$SMPROGRAMS\${APP_REG_NAME}"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_REG_NAME}"
  DeleteRegKey HKCU "Software\${APP_REG_NAME}"
  RMDir "$INSTDIR"
SectionEnd
