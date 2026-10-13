/*
 * Silent Typing — не отправлять индикатор «печатает…».
 * Установка: Настройки → Плагины → вставить код (имя: silent-typing).
 */
({
  name: 'Silent Typing',
  version: '1.0.0',
  author: 'RustyCord',
  description: 'Другие не видят, что вы печатаете (индикатор не отправляется на сервер)',
  start(api) {
    api.suppress('typing', true);
  },
  stop(api) {
    api.suppress('typing', false);
  }
});
