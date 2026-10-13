# Changelog

Все заметные изменения RustyCord документируются в этом файле.

Формат основан на [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
версионирование — [SemVer](https://semver.org/lang/ru/).

## [Unreleased]

### Planned
- Проигрывание входящего Opus-аудио через `cpal` (сейчас голосовой канал
  расшифровывает RTP-пакеты, но не воспроизводит звук).
- Захват микрофона и отправка аудио.
- Вложенные вложения (attachment) и embeds в сообщениях.
- Отправка/редактирование/удаление сообщений через REST.

## [0.1.0] — 2026-10-13

Первый публичный выпуск.

### Added

**Ядро (Rust, src-tauri)**
- Gateway-клиент Discord (WebSocket): рукопожатие HELLO → IDENTIFY/RESUME,
  heartbeat ~41.25 с с детектом «зомби»-соединения по отсутствию HEARTBEAT_ACK,
  реконнект с экспоненциальным бэкоффом 1→30 с, обработка close-кодов
  (4004 — невалидный токен, 4014 — деградация интентов, 1000 — чистый реконнект).
- Кэш состояния: гильдии, каналы, участники, роли, сообщения, presence,
  voice states — за `RwLock` без блокировки UI-потока.
- REST-клиент с полными rate-limit-бакетами: парсинг `X-RateLimit-*` заголовков,
  маршрутные ключи с мажорными ID, 429-backoff, глобальный lock на `X-RateLimit-Global`.
- Голосовые каналы: полный сигналинг Voice v4 (WS HELLO → IDENTIFY → READY с SSRC →
  UDP IP discovery для NAT-проброса → SELECT_PROTOCOL → SESSION_DESCRIPTION),
  парсер RTP, шифрование xsalsa20poly1305 (nonce = RTP-заголовок, tag-first layout),
  маппинг SSRC → user через опкоды 12/13.
- Декодер Snowflake-ID (время создания, worker/process/increment) с тестами.
- 15 Tauri-команд: логин, гильдии/каналы/сообщения, голос, kick/ban, роли.

**Интерфейс (Svelte 5, runes)**
- Пиксельная тематика Discord: #313338 / #5865F2, шрифты gg sans-подобные,
  все 15 компонентов — от ServerRail до VoiceView.
- Живые обновления через 25+ Gateway-событий: сообщения, реакции, typing,
  presence, speaking-индикаторы.
- Демо-режим для браузера без Discord-токена (MockBackend эмулирует Gateway).
- Инспектор Snowflake, контекстные меню, профили с ролями и правами.
- Консоль протокола: отправка сырых JSON-кадров прямо в Gateway websocket
  (команда `gateway_send_raw` + вкладка «Протокол» в настройках) — инструмент
  протокольных экспериментов как у настоящего клиента.
- Полный набор иконок приложения: PNG (32…512) + многослойный ICO + ICNS,
  логотип «шестерёнка в облачке чата» на блурпле-градиенте Discord.

**Инфраструктура**
- Репозиторий проверен реальной компиляцией: `cargo check` + `cargo clippy --all-targets
  -D warnings` по всему дереву зависимостей Tauri 2, unit-тесты crypto (roundtrip/
  tamper/wrong-key) и snowflake проходят.
- CI: svelte-check + production-сборка фронтенда, cargo fmt/clippy/test
  на трёх платформах (`.github/workflows/ci.yml`).
- Dev-сборка установщиков на каждый пуш в main (`.github/workflows/dev-build.yml`).
- Релизы по тегу v*: проверка консистентности версий, сборка macOS (universal),
  Linux (deb/rpm/AppImage), Windows (NSIS/MSI), черновик GitHub Release
  (`.github/workflows/release.yml`).
- Dependabot (npm + cargo + actions), шаблоны issue/PR, .editorconfig, CHANGELOG.

[Unreleased]: https://github.com/VinylRUS/Rustycord/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/VinylRUS/Rustycord/releases/tag/v0.1.0
