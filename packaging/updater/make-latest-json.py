#!/usr/bin/env python3
"""Генерация latest.json — манифеста авто-обновления tauri-plugin-updater.

Начиная с 0.12.0 манифест ведёт на артефакты GPUI-клиента: легаси-клиент на
Tauri при проверке обновлений скачивает именно их и мигрирует «на место»
(NSIS-инсталлер на Windows, замена .app на macOS, замена AppImage на Linux).

Формат — Static (как у tauri-action): ключи `{os}-{arch}` и приоритетные для
плагина `{os}-{arch}-{bundle_type}` (плагин ищет сначала суффиксированный
ключ, затем generic — см. UpdaterBuilder::target в tauri-plugin-updater).

Использование:
  make-latest-json.py --version 0.12.0 --tag v0.12.0 \
    --repo VinylRUS/Rustycord \
    --notes-file notes.txt \
    --win-exe-sig sigs/setup.exe.sig \
    --linux-appimage-sig sigs/appimage.sig \
    --darwin-app-sig sigs/app.tar.gz.sig \
    --out latest.json
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys


def read_sig(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read().strip()


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--version", required=True, help="версия релиза (0.12.0)")
    p.add_argument("--tag", required=True, help="тег релиза (v0.12.0)")
    p.add_argument("--repo", default="VinylRUS/Rustycord")
    p.add_argument("--notes-file", required=True, help="файл с заметками")
    p.add_argument("--win-exe-sig", required=True, help="base64-подпись NSIS setup.exe")
    p.add_argument("--linux-appimage-sig", required=True, help="base64-подпись AppImage")
    p.add_argument("--darwin-app-sig", required=True, help="base64-подпись app.tar.gz")
    p.add_argument("--out", default="latest.json")
    args = p.parse_args()

    with open(args.notes_file, encoding="utf-8") as f:
        notes = f.read().strip()

    base = f"https://github.com/{args.repo}/releases/download/{args.tag}/"
    win = {
        "signature": read_sig(args.win_exe_sig),
        "url": base + f"RustyCord_{args.version}_x64-setup.exe",
    }
    linux = {
        "signature": read_sig(args.linux_appimage_sig),
        "url": base + f"RustyCord_{args.version}_amd64.AppImage",
    }
    darwin = {
        "signature": read_sig(args.darwin_app_sig),
        "url": base + "RustyCord_universal.app.tar.gz",
    }

    manifest = {
        "version": args.version,
        "notes": notes,
        "pub_date": dt.datetime.now(dt.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
        "platforms": {
            # Приоритетные (суффиксированные) ключи — их плагин ищет первыми.
            "windows-x86_64-nsis": win,
            "linux-x86_64-appimage": linux,
            "darwin-aarch64-app": darwin,
            "darwin-x86_64-app": darwin,
            # Generic-ключи — фолбэк и человекочитаемость.
            "windows-x86_64": win,
            "linux-x86_64": linux,
            "darwin-aarch64": darwin,
            "darwin-x86_64": darwin,
        },
    }

    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
        f.write("\n")

    print(f"Готово: {args.out} (версия {args.version}, 8 платформенных ключей)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
