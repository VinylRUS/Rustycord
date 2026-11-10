#!/usr/bin/env bash
# Упаковка универсального RustyCord.app для macOS: .dmg + артефакт
# авто-обновления RustyCord_universal.app.tar.gz + портативный архив.
#
# Использование:
#   make-dist.sh <бинарник-arm64> <бинарник-x86_64> <версия> <icon.icns> <каталог-для-результатов>
#
# Идентификатор бандла (dev.rustycord.app) совпадает с Tauri-версией:
# tauri-plugin-updater на macOS подменяет содержимое текущего .app на
# распакованный из app.tar.gz — миграция Tauri → GPUI происходит «на место»,
# и ярлык в /Applications продолжает работать.
set -euo pipefail

BIN_ARM64="$1"
BIN_X64="$2"
VERSION="$3"
ICNS="$4"
OUTDIR="$5"

# Абсолютизируем пути ДО cd в staging-каталог: аргументы приходят
# относительными от корня репозитория. Выходной каталог создаём заранее —
# иначе abs() не сможет в него «зайти».
mkdir -p "$OUTDIR"
abs() {
  ( cd "$(dirname "$1")" && printf '%s/%s' "$PWD" "$(basename "$1")" )
}
BIN_ARM64="$(abs "$BIN_ARM64")"
BIN_X64="$(abs "$BIN_X64")"
ICNS="$(abs "$ICNS")"
OUTDIR="$(abs "$OUTDIR")"

APP="RustyCord.app"
STAGE="macos-build"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cd "$STAGE"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Универсальный бинарник (как .dmg Tauri-версии: Apple Silicon + Intel).
lipo -create "$BIN_ARM64" "$BIN_X64" -output "$APP/Contents/MacOS/rustycord"
chmod +x "$APP/Contents/MacOS/rustycord"

cp "$ICNS" "$APP/Contents/Resources/icon.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>RustyCord</string>
  <key>CFBundleExecutable</key>
  <string>rustycord</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundleIdentifier</key>
  <string>dev.rustycord.app</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>RustyCord</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
</dict>
</plist>
EOF

# Ad-hoc подпись: структура бандла валидна для Gatekeeper/launchservices.
# (Распространяем без Apple Developer ID — как и прежние релизы.)
codesign --force --sign - "$APP"

# 1) Установочный образ (пользователи тащат .app в Applications).
hdiutil create -volname "RustyCord" -srcfolder "$APP" -ov -format UDZO \
  "$OUTDIR/RustyCord_${VERSION}_universal.dmg"

# 2) Артефакт авто-обновления: тот же формат и имя, что генерировал
#    tauri-action для Tauri-версии (RustyCord_universal.app.tar.gz).
COPYFILE_DISABLE=1 tar czf "$OUTDIR/RustyCord_universal.app.tar.gz" "$APP"

# 3) Портативный архив с одним бинарником (как в релизах 0.11.x).
mkdir -p portable
cp "$APP/Contents/MacOS/rustycord" portable/rustycord-gpui
COPYFILE_DISABLE=1 tar czf "$OUTDIR/rustycord-gpui-macos-universal.tar.gz" -C portable rustycord-gpui

echo "Готово: $OUTDIR"
