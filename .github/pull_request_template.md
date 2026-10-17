## Что делает этот PR / What this PR does

<!-- Одна проблема на один PR. / One problem per PR. -->

## Связанный issue / Related issue

<!-- Closes #... -->

## Что проверено руками / What I checked by hand

<!-- ОС и что именно вы нажимали. / OS and what exactly you clicked. -->

## Чек-лист / Checklist

- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings` и `cargo test` проходят
- [ ] `npx svelte-check` и `npm run build` проходят
- [ ] Добавлен тест (если баг воспроизводится тестом)
- [ ] В логах нет токенов, cookie и authorization-заголовков
- [ ] Если менялся формат событий или IPC, проверены потребители на стороне Rust и Svelte
- [ ] Я не добавляю ничего, что автоматизирует действия от имени аккаунта или отправляет данные за пределы Discord

## Использовался ли ИИ? / Was AI used?

- [ ] Нет / No
- [ ] Да / Yes. Что именно я проверил сам: / What I verified myself:
