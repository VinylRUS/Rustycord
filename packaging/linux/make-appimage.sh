#!/usr/bin/env bash
# Сборка AppImage из релизного бинарника rustycord-gpui (Linux x86_64).
#
# Использование:
#   make-appimage.sh <бинарник> <иконка.png> <файл.desktop> <AppRun> <выходной.AppImage>
#
# Формат повторяет артефакт авто-обновления Tauri-версии (RustyCord_x.y.z_amd64.
# AppImage): tauri-plugin-updater в Linux-режиме заменяет файл AppImage на
# скачанный, поэтому после авто-обновления Tauri → GPUI ярлык продолжит
# запускать уже новый клиент.
set -euo pipefail

BIN="$1"
ICON="$2"
DESKTOP="$3"
APPRUN="$4"
OUT="$5"

APPDIR="appimage-build/AppDir"
rm -rf appimage-build
mkdir -p "$APPDIR/usr/bin"

install -m 0755 "$BIN"      "$APPDIR/usr/bin/rustycord"
install -m 0644 "$DESKTOP"  "$APPDIR/rustycord.desktop"
install -m 0644 "$ICON"     "$APPDIR/rustycord.png"
install -m 0755 "$APPRUN"   "$APPDIR/AppRun"

# appimagetool пакует AppDir в AppImage. На CI-раннерах нет /dev/fuse,
# поэтому распаковываем сами AppImage-инструменты (APPIMAGE_EXTRACT_AND_RUN).
curl -fL -o appimagetool.AppImage \
  "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x appimagetool.AppImage

ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 ./appimagetool.AppImage "$APPDIR" "$OUT"
echo "Готово: $OUT"
