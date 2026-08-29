Unicode true
RequestExecutionLevel user

!ifndef VERSION
    !error "VERSION define is required"
!endif
!ifndef PROJECT_ROOT
    !error "PROJECT_ROOT define is required"
!endif
!ifndef APOLLO_EXE
    !error "APOLLO_EXE define is required"
!endif
!ifndef OUTPUT_FILE
    !error "OUTPUT_FILE define is required"
!endif
!ifndef APP_ICON
    !error "APP_ICON define is required"
!endif

!define APP_NAME "Apollo"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Apollo"

Name "${APP_NAME}"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Apollo"
LicenseData "${PROJECT_ROOT}\LICENSE"
Icon "${APP_ICON}"
UninstallIcon "${APP_ICON}"

Page license
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Function .onInit
    SetShellVarContext current
    SetRegView 64
FunctionEnd

Function un.onInit
    SetShellVarContext current
    SetRegView 64
FunctionEnd

Section "Apollo" SEC_APOLLO
    SetOutPath "$INSTDIR"
    SetOverwrite on

    File /oname=Apollo.exe "${APOLLO_EXE}"
    File /oname=LICENSE "${PROJECT_ROOT}\LICENSE"
    File /oname=THIRD_PARTY_NOTICES.md "${PROJECT_ROOT}\THIRD_PARTY_NOTICES.md"

    WriteUninstaller "$INSTDIR\Uninstall.exe"
    CreateShortcut "$SMPROGRAMS\Apollo.lnk" "$INSTDIR\Apollo.exe"

    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\Apollo.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
    SetShellVarContext current
    SetRegView 64

    Delete "$SMPROGRAMS\Apollo.lnk"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"

    Delete "$INSTDIR\Apollo.exe"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
SectionEnd
