# Безопасность / Security

🇷🇺 [Русский](#русский) · 🇬🇧 [English](#english)

## Русский

RustyCord работает с пользовательским токеном, поэтому безопасность здесь важнее, чем в среднем проекте. Если вы нашли уязвимость (например, способ для плагина прочитать токен, утечку данных или обход ограничений Tauri), **не создавайте публичный issue**.

**Как сообщить:** на странице репозитория откройте **Security → Report a vulnerability** (приватные security advisories GitHub). Адрес публиковать не нужно, сообщение увидит только владелец.

Пожалуйста, опишите: что именно можно сделать, как это воспроизвести, версию RustyCord и ОС. **Не прикладывайте реальные токены.**

Что ожидать: время у владельца ограничено, поэтому первый ответ обычно приходит в течение одной-двух недель. Просим не раскрывать детали публично, пока уязвимость не исправлена. Имя нашедшего мы с радостью укажем в CHANGELOG, если вы не против.

Что считается уязвимостью: утечка или кража токена и данных пользователя, выполнение чужого кода вне модели плагинов, обход ограничений Tauri (capabilities), подмена обновлений. Баги интерфейса и функциональные ошибки отправляйте в обычные issues.

## English

RustyCord works with a user token, so security matters more here than in the average project. If you find a vulnerability (for example a way for a plugin to read the token, a data leak or a Tauri restriction bypass), **do not open a public issue**.

**How to report:** on the repository page open **Security → Report a vulnerability** (GitHub's private security advisories). You don't need to publish an address; only the owner will see the report.

Please describe what can be done, how to reproduce it, and your RustyCord version and OS. **Do not attach real tokens.**

What to expect: the owner's time is limited, so a first reply usually arrives within one to two weeks. Please don't disclose details publicly until the vulnerability is fixed. We'll gladly credit you in the CHANGELOG if you're fine with that.

What counts as a vulnerability: leaking or stealing the token or user data, running someone else's code outside the plugin model, bypassing Tauri restrictions (capabilities), spoofed updates. UI bugs and functional errors go to regular issues.
