# Changelog

Все заметные изменения RustyCord документируются в этом файле.

Формат основан на [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
версионирование — [SemVer](https://semver.org/lang/ru/).

## [Unreleased]

### Planned
- Адаптивная глубина jitter-буфера (сейчас фиксированный пре-ролл 40 мс).
- Вложенные вложения (attachment) и embeds в сообщениях.
- Отправка/редактирование/удаление сообщений через REST.

## [0.2.0] — 2026-10-13

Аудиодвижок: полный дуплекс — слышим собеседников и говорим сами.

### Added

**Приём звука (динамики)**
- Opus-декодирование входящих кадров: libopus 1.5 (через `audiopus`),
  моно-потоки апмиксятся в стерео — как у официального клиента.
- Jitter-буфер на SSRC-дорожках: переупорядочивание по seq, дедупликация,
  компенсация потерь (дыра ≤ 8 кадров — вставка тишины 20 мс, длиннее —
  ресинк без тишины), пре-ролл 40 мс на старт всплеска речи, потолок
  очереди 1 с.
- Микширование нескольких говорящих с жёстким клампом [-1; 1].
- Вывод через `cpal` (WASAPI / CoreAudio / ALSA), подбор конфигурации
  48 кГц F32 с фолбэком, линейный ресемплер для устройств с иной частотой,
  миксдаун/дабмикс каналов. Деафиен глушит вывод на уровне микшера.

**Передача звука (микрофон)**
- Захват микрофона через `cpal`: любой формат/частота → моно 48 кГц.
- Кодирование opus (режим Voip, 20 мс кадры), RTP-заголовок с
  seq/timestamp/marker, шифрование xsalsa20poly1305 (nonce = заголовок,
  tag-first), отправка через `try_send` без блокировки.
- Энергетический VAD с гистерезисом ~300 мс: op 5 SPEAKING отправляется
  только на переходах «заговорил/замолчал» — зелёные кольца у собеседников
  выглядят как при voice activity официального клиента.

**Управление**
- Мьют/деафиен гоняются прежним op 4; ядро теперь слушает эхо
  VOICE_STATE_UPDATE и на лету останавливает/возобновляет передачу и вывод
  без переподключения сессии.
- Сборка `--no-default-features` — headless-вариант без аудиоустройств:
  сигналинг, криптография и jitter работают, звук не выводится/не пишется.

**Инфраструктура**
- audiopus: на Linux линкуется системным libopus (libopus-dev в CI),
  на Windows/macOS собирает vendored-opus через cmake.
- CI: `CMAKE_POLICY_VERSION_MINIMUM=3.5` — vendored-opus несовместим с
  CMake >= 4 (совместимость с < 3.5 из CMake удалена); Linux не задет,
  там pkg-config.
- Тесты: 7 (jitter) + 3 (ресемплер) + opus roundtrip (steady state, 5 кадров)
  + RTP build/parse roundtrip; локальный logic-test — 16/16 с реальным
  libopus.

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

[Unreleased]: https://github.com/VinylRUS/Rustycord/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/VinylRUS/Rustycord/releases/tag/v0.2.0
[0.1.0]: https://github.com/VinylRUS/Rustycord/releases/tag/v0.1.0
