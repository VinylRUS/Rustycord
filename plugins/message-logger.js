/*
 * Message Logger — логирует все входящие/редактируемые/удаляемые сообщения
 * в консоль (F12) и тостом показывает удалённые.
 */
({
  name: 'Message Logger',
  version: '1.1.0',
  author: 'RustyCord',
  description: 'Показывает удалённые сообщения и пишет лог всех событий в консоль',
  start(api) {
    const seen = new Map();

    api.on('message:create', (m) => {
      seen.set(m.id, { author: m.author?.username ?? '?', content: m.content ?? '' });
    });

    api.on('message:delete', ({ id, channel_id }) => {
      const orig = seen.get(id);
      if (orig) {
        api.toast(`🗑 ${orig.author} удалил: ${orig.content.slice(0, 80)}`);
        console.log(`[logger] удалено в ${channel_id}:`, orig);
      }
    });

    api.on('message:update', (m) => {
      const orig = seen.get(m.id);
      if (orig && orig.content !== m.content) {
        console.log(`[logger] ${orig.author} отредактировал:\n  было: ${orig.content}\n  стало: ${m.content}`);
        seen.set(m.id, { ...orig, content: m.content });
      }
    });

    console.log('[logger] запущен, кэш сообщений активен');
  },
  stop() {
    console.log('[logger] остановлен');
  }
});
