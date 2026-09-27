!include "${__FILEDIR__}\start-menu.nsh"

; Check only this installation's files. Do not terminate a running scan or an
; independently deployed server. Opening for append tests write access without
; truncating or writing any bytes; Windows rejects it for a running executable.
LangString OrionFilesBusy 2052 "无法替换或移除 $R1。请从托盘选择“退出 Orion”；若后台是独立启动的，请先正常停止后台。然后点击重试。"
LangString OrionFilesBusy 1033 "Cannot replace or remove $R1. Choose Exit Orion from the system tray. If the server was started separately, stop it gracefully first. Then click Retry."

; Tauri's default check can kill a same-named desktop process, including in
; silent mode. Replace it with a retry/cancel check so a portable Orion instance
; is not terminated either. Keep the macro signature used by Tauri's template.
!ifmacrodef CheckIfAppIsRunning
  !macroundef CheckIfAppIsRunning
!endif
!macro CheckIfAppIsRunning executableName productName
  !define OrionCheckId ${__LINE__}
  orion_desktop_retry_${OrionCheckId}:
  !if "${INSTALLMODE}" == "both"
    ${If} $MultiUser.InstallMode == "AllUsers"
      nsis_tauri_utils::FindProcess "${executableName}"
    ${Else}
      nsis_tauri_utils::FindProcessCurrentUser "${executableName}"
    ${EndIf}
  !else if "${INSTALLMODE}" == "perMachine"
    nsis_tauri_utils::FindProcess "${executableName}"
  !else
    nsis_tauri_utils::FindProcessCurrentUser "${executableName}"
  !endif
  Pop $R0
  ${If} $R0 = 0
    StrCpy $R1 "${executableName}"
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(OrionFilesBusy)" /SD IDCANCEL IDRETRY orion_desktop_retry_${OrionCheckId}
    SetErrorLevel 2
    Abort
  ${EndIf}
  !undef OrionCheckId
!macroend

!macro OrionRequireStopped FILENAME LABEL
  ${If} ${FileExists} "$INSTDIR\${FILENAME}"
    orion_retry_${LABEL}:
    ClearErrors
    FileOpen $R0 "$INSTDIR\${FILENAME}" a
    ${If} ${Errors}
      StrCpy $R1 "${FILENAME}"
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(OrionFilesBusy)" /SD IDCANCEL IDRETRY orion_retry_${LABEL}
      SetErrorLevel 2
      Abort
    ${Else}
      FileClose $R0
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro OrionApplyStartMenuChoice
  !insertmacro OrionRequireStopped "orion-desktop.exe" desktop
  !insertmacro OrionRequireStopped "orion-server.exe" server
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro OrionRequireStopped "orion-desktop.exe" desktop
  !insertmacro OrionRequireStopped "orion-server.exe" server
!macroend
