!include nsDialogs.nsh

Var OrionStartMenuCheckbox
Var OrionStartMenuSelected

LangString OrionShortcutsTitle 2052 "开始菜单快捷方式"
LangString OrionShortcutsTitle 1033 "Start Menu shortcut"
LangString OrionShortcutsDescription 2052 "选择是否在开始菜单中添加 Orion。"
LangString OrionShortcutsDescription 1033 "Choose whether to add Orion to the Start Menu."
LangString OrionCreateStartMenu 2052 "创建开始菜单快捷方式"
LangString OrionCreateStartMenu 1033 "Create a Start Menu shortcut"

; Keep MUI's folder metadata and shortcut/uninstall macros, but replace its
; negative "Do not create shortcuts" option with a positive, opt-in checkbox.
!macroundef MUI_PAGE_STARTMENU
!macro MUI_PAGE_STARTMENU ID FOLDER
  !undef /ifexist MUI_PAGE_CUSTOMFUNCTION_PRE
  !define MUI_PAGE_CUSTOMFUNCTION_PRE Skip
  !insertmacro MUI_PAGE_INIT
  !insertmacro MUI_PAGEDECLARATION_STARTMENU "${ID}" "${FOLDER}"
  Page custom OrionStartMenuPage OrionStartMenuLeave

  Function OrionStartMenuPage
    ${If} $PassiveMode = 1
      Abort
    ${EndIf}
    !insertmacro MUI_HEADER_TEXT "$(OrionShortcutsTitle)" "$(OrionShortcutsDescription)"
    nsDialogs::Create 1018
    Pop $0
    ${If} $0 == "error"
      Abort
    ${EndIf}
    ${NSD_CreateCheckbox} 0 10u 100% 16u "$(OrionCreateStartMenu)"
    Pop $OrionStartMenuCheckbox
    ${If} $OrionStartMenuSelected == ${BST_CHECKED}
      ${NSD_Check} $OrionStartMenuCheckbox
    ${Else}
      ${NSD_Uncheck} $OrionStartMenuCheckbox
    ${EndIf}
    ${NSD_OnClick} $OrionStartMenuCheckbox OrionStartMenuChanged
    nsDialogs::Show
  FunctionEnd

  Function OrionStartMenuChanged
    Pop $0
    ${NSD_GetState} $OrionStartMenuCheckbox $OrionStartMenuSelected
  FunctionEnd

  Function OrionStartMenuLeave
    ${NSD_GetState} $OrionStartMenuCheckbox $OrionStartMenuSelected
  FunctionEnd
!macroend

!macro OrionApplyStartMenuChoice
  ; MUI treats a leading '>' as an instruction to skip shortcut creation.
  ; Silent/passive installs also default to no new Start Menu shortcut.
  ${If} $OrionStartMenuSelected == ${BST_CHECKED}
    StrCpy $AppStartMenuFolder ""
  ${Else}
    StrCpy $AppStartMenuFolder ">"
  ${EndIf}
!macroend
