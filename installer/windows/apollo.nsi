Unicode true
RequestExecutionLevel user

!include "MUI2.nsh"
!include "Sections.nsh"

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
!define APP_PATHS_KEY "Software\Microsoft\Windows\CurrentVersion\App Paths\Apollo.exe"

Var ExistingInstall

Name "${APP_NAME}"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Apollo"
Icon "${APP_ICON}"
UninstallIcon "${APP_ICON}"

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PROJECT_ROOT}\LICENSE"
!define MUI_PAGE_CUSTOMFUNCTION_PRE DirectoryPagePre
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\Apollo.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Apollo"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Apollo" SEC_APOLLO
    SectionIn RO

    SetOutPath "$INSTDIR"
    SetOverwrite on

    File /oname=Apollo.exe "${APOLLO_EXE}"
    File /oname=LICENSE "${PROJECT_ROOT}\LICENSE"
    File /oname=THIRD_PARTY_NOTICES.md "${PROJECT_ROOT}\THIRD_PARTY_NOTICES.md"

    WriteUninstaller "$INSTDIR\Uninstall.exe"
    CreateShortcut "$SMPROGRAMS\Apollo.lnk" "$INSTDIR\Apollo.exe" "" "$INSTDIR\Apollo.exe" 0

    WriteRegStr HKCU "${APP_PATHS_KEY}" "" "$INSTDIR\Apollo.exe"

    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\Apollo.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section /o "Create desktop shortcut" SEC_DESKTOP
    CreateShortcut "$DESKTOP\Apollo.lnk" "$INSTDIR\Apollo.exe" "" "$INSTDIR\Apollo.exe" 0
SectionEnd

Section "-Finalize shortcut choice"
    SectionGetFlags ${SEC_DESKTOP} $0
    IntOp $0 $0 & ${SF_SELECTED}
    IntCmp $0 0 remove_desktop keep_desktop keep_desktop

remove_desktop:
    Delete "$DESKTOP\Apollo.lnk"
keep_desktop:
SectionEnd

Function .onInit
    SetShellVarContext current
    SetRegView 64
    StrCpy $ExistingInstall "0"

    ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "InstallLocation"
    StrCmp $0 "" install_location_done
    StrCpy $INSTDIR $0
    StrCpy $ExistingInstall "1"
install_location_done:

    StrCmp $ExistingInstall "1" 0 desktop_choice_done
    IfFileExists "$DESKTOP\Apollo.lnk" 0 desktop_choice_done
    SectionGetFlags ${SEC_DESKTOP} $0
    IntOp $0 $0 | ${SF_SELECTED}
    SectionSetFlags ${SEC_DESKTOP} $0
desktop_choice_done:
FunctionEnd

Function DirectoryPagePre
    StrCmp $ExistingInstall "1" 0 directory_page_done
    Abort
directory_page_done:
FunctionEnd

Function un.onInit
    SetShellVarContext current
    SetRegView 64
FunctionEnd

Section "Uninstall"
    SetShellVarContext current
    SetRegView 64

    Delete "$SMPROGRAMS\Apollo.lnk"
    Delete "$DESKTOP\Apollo.lnk"
    DeleteRegKey HKCU "${APP_PATHS_KEY}"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"

    Delete "$INSTDIR\Apollo.exe"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
SectionEnd
