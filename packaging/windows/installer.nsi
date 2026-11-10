; ─────────────────────────────────────────────────────────────────────────────
; RustyCord (GPUI) — NSIS-инсталлер для Windows.
;
; С 0.12.0 единственный клиент — нативный GPUI-бинарник. Инсталлер повторяет
; раскладку Tauri-версии (tauri-bundler, installMode=currentUser), чтобы
; обновление Tauri → GPUI через tauri-plugin-updater встало «на место»:
;
;   каталог установки ....... $LOCALAPPDATA\RustyCord
;   бинарник ............... $INSTDIR\rustycord.exe   (то же имя, что у Tauri)
;   деинсталлятор .......... $INSTDIR\uninstall.exe
;   реестр ................. HKCU\Software\Microsoft\Windows\CurrentVersion\
;                            Uninstall\RustyCord
;   ярлыки ................. $SMPROGRAMS\RustyCord.lnk, $DESKTOP\RustyCord.lnk
;
; Флаги командной строки (совместимо с tauri-plugin-updater 2.x, который
; запускает инсталлер как `setup.exe /P /UPDATE /R /ARGS …`):
;   /S       — полностью тихая установка (штатный silent-режим NSIS)
;   /P       — passive: тихая установка (как у Tauri)
;   /UPDATE  — режим обновления: гарантированно гасим запущенный клиент
;   /R       — перезапустить клиент после установки
;   /ARGS …  — аргументы для перезапуска (/R)
;   /NS      — не создавать ярлыки
;
; Сборка (из корня репозитория):
;   makensis -DVERSION=0.12.0 ^
;     -DSRC_EXE=target\release\rustycord-gpui.exe ^
;     -DICON=src-tauri\icons\icon.ico ^
;     -DOUTFILE=RustyCord_0.12.0_x64-setup.exe packaging\windows\installer.nsi
; ─────────────────────────────────────────────────────────────────────────────

!define PRODUCTNAME "RustyCord"
!define EXECUTABLE  "rustycord"
!define COMPANY     "VinylRUS"
!define UNINSTKEY   "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"

!ifndef VERSION
  !error "VERSION не задан: makensis -DVERSION=x.y.z …"
!endif
!ifndef SRC_EXE
  !error "SRC_EXE не задан: makensis -DSRC_EXE=путь\к\rustycord-gpui.exe …"
!endif
!ifndef OUTFILE
  !error "OUTFILE не задан: makensis -DOUTFILE=RustyCord_x.y.z_x64-setup.exe …"
!endif
!ifndef ICON
  !define ICON "src-tauri\icons\icon.ico"
!endif

Name "${PRODUCTNAME}"
OutFile "${OUTFILE}"
Unicode true
InstallDir "$LOCALAPPDATA\${PRODUCTNAME}"
RequestExecutionLevel user
SetCompressor /SOLID lzma
; Браузерные страницы не нужны: один бинарник без ресурсов.

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "RustyCord installer (GPUI)"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT License, VinylRUS"
VIAddVersionKey "CompanyName" "${COMPANY}"

!include "MUI2.nsh"
!include "FileFunc.nsh"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Var PassiveMode
Var UpdateMode
Var NoShortcutMode
Var RestartApp
Var RestartArgs

Function .onInit
  ; /P — passive (тихая установка, прогресс не показываем: инсталлер крошечный)
  ${GetOptions} $CMDLINE "/P" $R0
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
    SetSilent silent
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $R0
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
    SetSilent silent
  ${EndIf}

  ${GetOptions} $CMDLINE "/NS" $R0
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/R" $R0
  ${IfNot} ${Errors}
    StrCpy $RestartApp 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/ARGS" $RestartArgs

  ; Ставим поверх прежней установки: Tauri-инсталлер писал InstallLocation
  ; в кавычках ("C:\Users\...\RustyCord") — срезаем их.
  ReadRegStr $R0 HKCU "${UNINSTKEY}" "InstallLocation"
  ${If} $R0 != ""
    StrCpy $R1 $R0 1
    ${If} $R1 == '"'
      StrCpy $R0 $R0 "" 1
      StrCpy $R0 $R0 -1
    ${EndIf}
    ${If} $R0 != ""
      StrCpy $INSTDIR $R0
    ${EndIf}
  ${EndIf}
FunctionEnd

Section "RustyCord" SEC_MAIN
  SectionIn RO

  ; В режиме обновления tauri-plugin-updater сам завершает приложение
  ; (std::process::exit(0)), но на всякий случай гасим зависший процесс:
  ; иначе перезапись rustycord.exe упрётся в блокировку файла.
  ${If} $UpdateMode = 1
  ${OrIf} $PassiveMode = 1
  ${OrIf} ${Silent}
    nsExec::Exec 'taskkill /F /IM "${EXECUTABLE}.exe"'
    Sleep 700
  ${EndIf}

  SetOutPath "$INSTDIR"
  File "/oname=${EXECUTABLE}.exe" "${SRC_EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ${If} $NoShortcutMode != 1
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${EXECUTABLE}.exe"
    CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${EXECUTABLE}.exe"
  ${EndIf}

  ; Тот же набор значений, что пишет tauri-bundler (MainBinaryName — тоже),
  ; чтобы «Программы и компоненты» бесшовно показывал новую версию.
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${COMPANY}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\${EXECUTABLE}.exe$\""
  WriteRegStr HKCU "${UNINSTKEY}" "MainBinaryName" "${EXECUTABLE}.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "URLInfoAbout" "https://github.com/VinylRUS/Rustycord"
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1

  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" $0
SectionEnd

Function .onInstSuccess
  ; /R — перезапуск после авто-обновления (запускаем от текущего пользователя:
  ; инсталлер и так работает без повышения прав).
  ${If} $RestartApp = 1
    ${If} $RestartArgs != ""
      Exec '"$INSTDIR\${EXECUTABLE}.exe" $RestartArgs'
    ${Else}
      Exec '"$INSTDIR\${EXECUTABLE}.exe"'
    ${EndIf}
  ${EndIf}
FunctionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /F /IM "${EXECUTABLE}.exe"'
  Sleep 500

  Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  Delete "$INSTDIR\${EXECUTABLE}.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTKEY}"
SectionEnd
