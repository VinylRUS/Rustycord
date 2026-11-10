#!/usr/bin/env bash
# Сборка .deb без cargo-deb: обычный dpkg-deb поверх собранного бинарника.
#
# Использование:
#   make-deb.sh <бинарник> <версия> <иконка.png> <файл.desktop> <выходной.deb>
#
# Пакет называется так же, как deb Tauri-версии (rustycord) и кладёт бинарник
# в тот же /usr/bin/rustycord — `sudo dpkg -i` обновляет Tauri → GPUI «на место».
set -euo pipefail

BIN="$1"
VERSION="$2"
ICON="$3"
DESKTOP="$4"
OUT="$5"

PKGDIR="deb-build/rustycord"
rm -rf deb-build
install -d "$PKGDIR/DEBIAN" \
          "$PKGDIR/usr/bin" \
          "$PKGDIR/usr/share/applications" \
          "$PKGDIR/usr/share/icons/hicolor/128x128/apps"

install -m 0755 "$BIN"     "$PKGDIR/usr/bin/rustycord"
install -m 0644 "$DESKTOP" "$PKGDIR/usr/share/applications/rustycord.desktop"
install -m 0644 "$ICON"    "$PKGDIR/usr/share/icons/hicolor/128x128/apps/rustycord.png"

# Зависимости рантайма GPUI (blade-graphics → Vulkan, X11-окна → xcb/xkbcommon,
# шрифты → fontconfig).
INSTALLED_SIZE="$(du -sk "$PKGDIR" | cut -f1)"
cat > "$PKGDIR/DEBIAN/control" <<EOF
Package: rustycord
Version: $VERSION
Section: net
Priority: optional
Architecture: amd64
Depends: libvulkan1, libfontconfig1, libxcb1, libxkbcommon0, libxkbcommon-x11-0
Maintainer: VinylRUS <pepega.ads@pepe.ga>
Homepage: https://github.com/VinylRUS/Rustycord
Installed-Size: $INSTALLED_SIZE
Description: Unofficial Discord client (native GPUI frontend)
 RustyCord — unofficial Discord client: shared protocol in rustycord-core,
 native UI on GPUI. Successor of the legacy Tauri build.
EOF

dpkg-deb --build --root-owner-group "$PKGDIR" "$OUT"
echo "Готово: $OUT"
