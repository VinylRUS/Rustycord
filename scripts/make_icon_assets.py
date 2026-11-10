#!/usr/bin/env python3
"""Генерация иконки RustyCord (краб, кусающий Вампуса) из загруженного PNG.

Выходные ассеты:
  app-gpui/assets/icon.png        — 512x512, для include_bytes (логин, домашняя
                                    кнопка, трей-иконка, распаковка через image)
  app-gpui/assets/icon.ico        — для build.rs (winresource вшивает в .exe)
  packaging/icons/icon.ico        — Windows-инсталлер (NSIS)
  packaging/icons/icon.icns       — macOS
  packaging/icons/128x128.png     — Linux deb/AppImage + .desktop
  packaging/icons/256x256.png     — README-логотип
"""
from PIL import Image
import os

SRC = "/home/z/my-project/upload/Краб Rustycord обнимает Вампуса.png"
ROOT = "/home/z/my-project"

im = Image.open(SRC).convert("RGBA")


def scaled(size: int) -> Image.Image:
    return im.resize((size, size), Image.LANCZOS)


# 1. app-gpui/assets/icon.png (512)
os.makedirs(f"{ROOT}/app-gpui/assets", exist_ok=True)
scaled(512).save(f"{ROOT}/app-gpui/assets/icon.png", optimize=True)

# 2. Windows .ico (multi-size) — один файл и для .exe, и для NSIS
os.makedirs(f"{ROOT}/packaging/icons", exist_ok=True)
for ico_path in (f"{ROOT}/app-gpui/assets", f"{ROOT}/packaging/icons"):
    scaled(256).save(
        f"{ico_path}/icon.ico",
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )

# 3. macOS .icns — Pillow собирает все размеры из одного большого изображения
scaled(512).save(f"{ROOT}/packaging/icons/icon.icns", format="ICNS")

# 4. Linux PNG + README
scaled(128).save(f"{ROOT}/packaging/icons/128x128.png", optimize=True)
scaled(256).save(f"{ROOT}/packaging/icons/256x256.png", optimize=True)

for p in [
    "app-gpui/assets/icon.png",
    "packaging/icons/icon.ico",
    "packaging/icons/icon.icns",
    "packaging/icons/128x128.png",
    "packaging/icons/256x256.png",
]:
    full = f"{ROOT}/{p}"
    assert os.path.getsize(full) > 0, p
    print(f"{p}: {os.path.getsize(full)} bytes")
print("OK")
