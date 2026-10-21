Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
Name "Vault"
OutFile "${ARTIFACT_DIR}\Vault-${APP_VERSION}-Setup.exe"
InstallDir "D:\Vault"
InstallDirRegKey HKCU "Software\Vault" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
!define MUI_ABORTWARNING
!define MUI_ICON "..\src-tauri\icons\icon.ico"
!define MUI_UNICON "..\src-tauri\icons\icon.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PACKAGE_DIR}\LICENSE.txt"
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE EnsureClosed
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\Vault.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "Vault"
VIAddVersionKey /LANG=2052 "FileDescription" "Vault 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "panda-student · Apache-2.0"

Function .onInit
  SetShellVarContext current
  ${If} $INSTDIR == "D:\Vault"
    ${IfNot} ${FileExists} "D:\*.*"
      StrCpy $INSTDIR "$LOCALAPPDATA\Programs\Vault"
    ${EndIf}
  ${EndIf}
FunctionEnd

!macro DefineEnsureClosed Prefix
Function ${Prefix}EnsureClosed
  IfFileExists "$INSTDIR\Vault.exe" 0 done
  ClearErrors
  FileOpen $0 "$INSTDIR\Vault.exe" a
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONSTOP "无法修改程序文件。请先退出 Vault 并断开 MCP，确认目录可写后重试。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  FileClose $0
  done:
FunctionEnd
!macroend
!insertmacro DefineEnsureClosed ""
!insertmacro DefineEnsureClosed "un."

Function un.onInit
  Call un.EnsureClosed
FunctionEnd

Section "Vault" Main
  Call EnsureClosed
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${PACKAGE_DIR}\Vault.exe"
  File "${PACKAGE_DIR}\README.txt"
  File "${PACKAGE_DIR}\LICENSE.txt"
  File "${PACKAGE_DIR}\NOTICE.txt"
  File "${PACKAGE_DIR}\THIRD_PARTY_NOTICES.txt"
  SetOutPath "$INSTDIR\third-party-sources"
  File "${PACKAGE_DIR}\third-party-sources\*"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Vault" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "DisplayName" "Vault"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "Publisher" "panda-student"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "DisplayIcon" "$INSTDIR\Vault.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault" "NoRepair" 1
  CreateDirectory "$SMPROGRAMS\Vault"
  CreateShortcut "$SMPROGRAMS\Vault\Vault.lnk" "$INSTDIR\Vault.exe"
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  ; Delete only installed files. Never remove the user's data directory.
  Delete "$INSTDIR\Vault.exe"
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
  Delete "$SMPROGRAMS\Vault\Vault.lnk"
  RMDir "$SMPROGRAMS\Vault"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Vault"
  DeleteRegKey HKCU "Software\Vault"
  RMDir "$INSTDIR"
SectionEnd
