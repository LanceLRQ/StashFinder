!define STASHFINDER_SERVICE_SCRIPT "${__FILEDIR__}\service.ps1"
!include nsDialogs.nsh

Var StashFinderService
Var StashFinderServiceCheck

Page custom StashFinderBackendPage StashFinderBackendLeave

Function StashFinderBackendChoice
  ${If} $StashFinderService != ""
    Return
  ${EndIf}
  ReadRegDWORD $StashFinderService HKLM "Software\StashFinder" "InstallService"
  ${If} ${Errors}
    StrCpy $StashFinderService 0
    ReadRegStr $0 HKLM "SYSTEM\CurrentControlSet\Services\StashFinder" "ImagePath"
    ${If} $0 != ""
      StrCpy $StashFinderService 1
    ${EndIf}
  ${EndIf}
  ${GetParameters} $0
  ${GetOptions} $0 "/SERVICE" $1
  ${IfNot} ${Errors}
    StrCpy $StashFinderService 1
  ${EndIf}
  ${GetOptions} $0 "/NOSERVICE" $1
  ${IfNot} ${Errors}
    StrCpy $StashFinderService 0
  ${EndIf}
FunctionEnd

Function StashFinderBackendPage
  Call StashFinderBackendChoice
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 8u 100% 12u "Indexing backend"
  Pop $0
  ${NSD_CreateCheckbox} 0 27u 100% 13u "Install the StashFinder Windows service"
  Pop $StashFinderServiceCheck
  ${NSD_SetState} $StashFinderServiceCheck $StashFinderService
  ${NSD_CreateLabel} 0 49u 100% 60u "With the service, indexing continues after you close StashFinder.$\r$\n$\r$\nWithout the service, indexing runs while StashFinder is open and asks for administrator approval when it starts."
  Pop $0
  nsDialogs::Show
FunctionEnd

Function StashFinderBackendLeave
  ${NSD_GetState} $StashFinderServiceCheck $StashFinderService
FunctionEnd

!macro STASHFINDER_SERVICE_PHASE PHASE CHOICE
  InitPluginsDir
  File /oname=$PLUGINSDIR\stashfinder-install-service.ps1 "${STASHFINDER_SERVICE_SCRIPT}"
  StrCpy $2 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$2" +2
  StrCpy $2 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  nsExec::ExecToStack /TIMEOUT=90000 '"$2" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\stashfinder-install-service.ps1" -Phase ${PHASE} -InstallDirectory "$INSTDIR" -InstallService ${CHOICE}'
  Pop $0
  Pop $1
  ${If} $0 != 0
    DetailPrint "$1"
    IfSilent +2
    MessageBox MB_OK|MB_ICONSTOP "StashFinder background service (${PHASE}) failed.$\r$\n$1"
    SetErrorLevel 1
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  Call StashFinderBackendChoice
  !insertmacro STASHFINDER_SERVICE_PHASE Prepare $StashFinderService
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro STASHFINDER_SERVICE_PHASE Install $StashFinderService
  WriteRegDWORD HKLM "Software\StashFinder" "InstallService" $StashFinderService
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro STASHFINDER_SERVICE_PHASE Uninstall 0
!macroend
