<script lang="ts">
  import { app } from '../stores/app.svelte';
  import { plugins } from '../plugins/manager';
  import type { SnowflakeInfo } from '../types';
  import { formatDuration } from '../time';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  let tab = $state<'connection' | 'plugins' | 'snowflake' | 'protocol' | 'about'>('connection');
  let snowflakeInput = $state('');
  let snowflakeResult = $state<SnowflakeInfo | null>(null);
  let snowflakeError = $state('');
  let rawFrame = $state('{\n  "op": 8,\n  "d": { "guild_id": "...", "query": "", "limit": 0 }\n}');
  let rawStatus = $state<{ kind: 'ok' | 'err'; text: string } | null>(null);

  /* ---------- plugins ---------- */
  let pluginList = $state<import('../types').PluginInfo[]>([]);
  let newPluginName = $state('');
  let newPluginCode = $state('');
  let pluginError = $state('');
  let viewCode = $state('');
  let viewCodeName = $state('');
  let pluginsBusy = $state(false);

  async function refreshPlugins() {
    try {
      pluginList = await plugins.loadedList();
    } catch (e) {
      pluginError = e instanceof Error ? e.message : String(e);
    }
  }

  // Подтягиваем список при открытии вкладки.
  $effect(() => {
    if (tab === 'plugins' && app.user) void refreshPlugins();
  });

  async function installPlugin() {
    pluginError = '';
    if (!newPluginName.trim() || !newPluginCode.trim()) {
      pluginError = 'Укажи имя файла и код плагина';
      return;
    }
    pluginsBusy = true;
    try {
      await plugins.install(newPluginName.trim(), newPluginCode);
      newPluginName = '';
      newPluginCode = '';
      await refreshPlugins();
      app.toast('Плагин установлен (выключен) — включи его в списке', 'info');
    } catch (e) {
      pluginError = e instanceof Error ? e.message : String(e);
    } finally {
      pluginsBusy = false;
    }
  }

  async function togglePlugin(file: string, enabled: boolean) {
    try {
      await plugins.setEnabled(file, enabled);
      await refreshPlugins();
    } catch (e) {
      app.toast(`Не удалось: ${e instanceof Error ? e.message : String(e)}`, 'error');
    }
  }

  async function removePlugin(file: string) {
    if (!confirm(`Удалить плагин ${file}?`)) return;
    await plugins.remove(file);
    await refreshPlugins();
  }

  async function showCode(file: string) {
    if (viewCodeName === file) {
      viewCodeName = '';
      viewCode = '';
      return;
    }
    viewCodeName = file;
    viewCode = await plugins.readCode(file);
  }

  const PLUGIN_TEMPLATE = `({
  name: 'Мой плагин',
  version: '1.0.0',
  author: 'ты',
  description: 'Что делает плагин',
  start(api) {
    // api.on('message:create', (m) => console.log(m.author.username, m.content));
    // api.addCss('.message { letter-spacing: 0.3px; }');
    // api.suppress('typing', true);
  },
  stop() {}
});`;

  async function inspect() {
    snowflakeError = '';
    snowflakeResult = null;
    const id = snowflakeInput.trim();
    if (!id) return;
    try {
      snowflakeResult = await app.parseSnowflake(id);
    } catch (e) {
      snowflakeError = e instanceof Error ? e.message : String(e);
    }
  }

  async function sendRaw() {
    rawStatus = null;
    try {
      await app.gatewaySendRaw(rawFrame);
      rawStatus = { kind: 'ok', text: 'Кадр отправлен в gateway — ответ придёт потоком dispatch-событий (RUST_LOG=debug покажет его в консоли).' };
    } catch (e) {
      rawStatus = { kind: 'err', text: e instanceof Error ? e.message : String(e) };
    }
  }

  const intents = [
    ['GUILDS', 1],
    ['GUILD_MEMBERS', 2],
    ['GUILD_MODERATION', 4],
    ['GUILD_EMOJIS_AND_STICKERS', 8],
    ['GUILD_INTEGRATIONS', 16],
    ['GUILD_WEBHOOKS', 32],
    ['GUILD_INVITES', 64],
    ['GUILD_VOICE_STATES', 128],
    ['GUILD_PRESENCES', 256],
    ['GUILD_MESSAGES', 512],
    ['GUILD_MESSAGE_REACTIONS', 1024],
    ['GUILD_MESSAGE_TYPING', 2048],
    ['DIRECT_MESSAGES', 4096],
    ['DIRECT_MESSAGE_REACTIONS', 8192],
    ['DIRECT_MESSAGE_TYPING', 16384],
    ['MESSAGE_CONTENT', 32768],
    ['GUILD_SCHEDULED_EVENTS', 65536],
  ] as const;
  const intentsSum = $derived(intents.reduce((acc, [, v]) => acc + v, 0));
  const intentsActive = $derived(intents.filter(([, v]) => (intentsSum & v) !== 0).length);
</script>

{#if app.showSettings}
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => (app.showSettings = false)} oncontextmenu={(e) => e.preventDefault()}></div>
  <div class="modal">
    <header>
      <h1>Настройки RustyCord</h1>
      <button class="close" onclick={() => (app.showSettings = false)}>
        <Icon name="close" size={20} color="#b5bac1" />
      </button>
    </header>

    <nav class="tabs">
      <button class:active={tab === 'connection'} onclick={() => (tab = 'connection')}>Подключение</button>
      <button class:active={tab === 'plugins'} onclick={() => (tab = 'plugins')}>Плагины</button>
      <button class:active={tab === 'snowflake'} onclick={() => (tab = 'snowflake')}>Инспектор Snowflake</button>
      <button class:active={tab === 'protocol'} onclick={() => (tab = 'protocol')}>Протокол</button>
      <button class:active={tab === 'about'} onclick={() => (tab = 'about')}>О проекте</button>
    </nav>

    <div class="content scroller">
      {#if tab === 'connection'}
        <div class="user-line">
          <Avatar user={app.user} size={48} ringColor="#313338" />
          <div>
            <div class="big">{app.user?.global_name || app.user?.username}</div>
            <div class="muted">{app.user ? '@' + app.user.username : ''} · {app.user?.bot ? 'bot' : 'user'}</div>
          </div>
        </div>
        <table>
          <tbody>
          <tr><td>Режим</td><td class="mono">{app.mode === 'mock' ? 'Демо (симуляция Gateway)' : 'Tauri → Rust core'}</td></tr>
          <tr><td>Статус</td><td class="mono">{app.connStage}</td></tr>
          <tr><td>Session ID</td><td class="mono small">{app.sessionId || '—'}</td></tr>
          <tr><td>Gateway</td><td class="mono small">wss://gateway.discord.gg/?v=10&amp;encoding=json</td></tr>
          <tr><td>Intents</td><td class="mono">{intentsSum} ({intentsActive} intents, user)</td></tr>
          <tr><td>Гильдий в кэше</td><td class="mono">{app.guilds.length}</td></tr>
          </tbody>
        </table>
        <div class="row">
          <button class="btn-blurple" onclick={() => void app.logout()}>
            <span style="display:inline-flex;align-items:center;gap:6px">
              <Icon name="logout" size={16} color="#fff" /> Выйти из аккаунта
            </span>
          </button>
        </div>
      {:else if tab === 'snowflake'}
        <p class="hint">
          Snowflake — 64-битный ID Discord: <b>42 бита</b> времени (эпоха 2015-01-01) + <b>5</b> worker +
          <b>5</b> process + <b>12</b> инкремент. Вставь любой ID (сообщение, канал, пользователь) — парсер
          работает на Rust side, а в демо — на BigInt.
        </p>
        <div class="sf-form">
          <input placeholder="Например: 1301000000000000305" bind:value={snowflakeInput} />
          <button class="btn-blurple" onclick={() => void inspect()}>Разобрать</button>
        </div>
        {#if snowflakeError}
          <div class="err">{snowflakeError}</div>
        {/if}
        {#if snowflakeResult}
          <table>
            <tbody>
            <tr><td>ID</td><td class="mono small">{snowflakeResult.id}</td></tr>
            <tr><td>Создан</td><td class="mono">{new Date(snowflakeResult.timestamp_ms).toLocaleString('ru-RU')}</td></tr>
            <tr><td>ISO 8601</td><td class="mono small">{snowflakeResult.created_at}</td></tr>
            <tr><td>Возраст</td><td class="mono">{formatDuration(snowflakeResult.age_seconds)} назад</td></tr>
            <tr><td>Worker ID</td><td class="mono">{snowflakeResult.worker_id}</td></tr>
            <tr><td>Process ID</td><td class="mono">{snowflakeResult.process_id}</td></tr>
            <tr><td>Инкремент</td><td class="mono">{snowflakeResult.increment}</td></tr>
            </tbody>
          </table>
        {/if}
      {:else if tab === 'protocol'}
        <p class="hint">
          <b>Консоль протокола</b> — escape hatch в сам websocket. Любой JSON-кадр уйдёт как есть:
          экспериментируйте с опкодами 1/8/… как настоящий клиент. Ответ смотрите в dispatch-потоке
          или в логах (RUST_LOG=debug). В демо-режиме кадр только валидируется.
        </p>
        <div class="sf-form">
          <textarea
            rows="7"
            class="mono"
            spellcheck="false"
            bind:value={rawFrame}
            placeholder={'{ "op": 1, "d": null }'}
          ></textarea>
        </div>
        <div class="row">
          <button class="btn-blurple" onclick={() => void sendRaw()}>Отправить кадр</button>
        </div>
        {#if rawStatus}
          <div class={rawStatus.kind === 'ok' ? 'ok-msg' : 'err'}>{rawStatus.text}</div>
        {/if}
      {:else if tab === 'plugins'}
        <p class="hint">
          <b>Плагины</b> — как у Vencord: JS-файлы в папке приложения. Исполняются в том же WebView,
          что и интерфейс; сетевые запросы идут через Rust-слой с общими rate limits.
        </p>

        {#if pluginError}
          <div class="err">{pluginError}</div>
        {/if}

        {#if pluginList.length === 0}
          <p class="hint">Установленных плагинов нет. Вставь код ниже или положи .js в папку плагинов.</p>
        {:else}
          {#each pluginList as p (p.file)}
            <div class="plugin-card">
              <div class="plugin-head">
                <div class="plugin-title">
                  <b>{p.meta?.name ?? p.file}</b>
                  {#if p.meta?.version}<span class="muted">v{p.meta.version}</span>{/if}
                  {#if p.meta?.author}<span class="muted">· {p.meta.author}</span>{/if}
                  {#if plugins.isRunning(p.file)}<span class="ok-tag">запущен</span>{/if}
                </div>
                <label class="switch">
                  <input type="checkbox" checked={p.enabled} onchange={(e) => void togglePlugin(p.file, (e.target as HTMLInputElement).checked)} />
                  <span></span>
                </label>
              </div>
              {#if p.meta?.description}
                <p class="plugin-desc">{p.meta.description}</p>
              {/if}
              <div class="plugin-actions">
                <button class="mini" onclick={() => void showCode(p.file)}>{viewCodeName === p.file ? 'Скрыть код' : 'Код'}</button>
                <button class="mini danger" onclick={() => void removePlugin(p.file)}>Удалить</button>
                <span class="muted mono-s">{p.file} · {Math.max(1, Math.round(p.size / 1024))} КБ</span>
              </div>
              {#if viewCodeName === p.file}
                <pre class="plugin-code">{viewCode}</pre>
              {/if}
            </div>
          {/each}
        {/if}

        <h3 class="sub">Установить новый плагин</h3>
        <div class="sf-form">
          <input placeholder="имя-файла (латиница, напр. my-plugin)" bind:value={newPluginName} />
          <textarea
            rows="10"
            class="mono"
            spellcheck="false"
            bind:value={newPluginCode}
            placeholder={PLUGIN_TEMPLATE}
          ></textarea>
          <div class="row">
            <button class="btn-blurple" onclick={() => void installPlugin()} disabled={pluginsBusy}>
              {pluginsBusy ? 'Устанавливаем…' : 'Установить'}
            </button>
            <button class="btn-ghost" onclick={() => (newPluginCode = PLUGIN_TEMPLATE)}>Вставить шаблон</button>
          </div>
        </div>
        <p class="hint small">
          API: api.on('message:create'|'message:send'|'dispatch'|'ready', fn) · api.rest(method, path, body) ·
          api.addCss(css) · api.suppress('typing', true) · api.storage · api.toast(text).
          Хук 'message:send' может вернуть изменённый текст или false для отмены.
        </p>
      {:else}
        <p class="hint">
          <b>RustyCord</b> — экспериментальный клиент Discord, построенный на реверс-инжиниринге публичного API.
        </p>
        <table>
          <tbody>
          <tr><td>Ядро</td><td class="mono">Rust · tokio · tokio-tungstenite</td></tr>
          <tr><td>UI</td><td class="mono">Svelte 5 (runes) · Vite · TypeScript</td></tr>
          <tr><td>Оболочка</td><td class="mono">Tauri 2 (system WebView)</td></tr>
          <tr><td>Голос</td><td class="mono">voice WS v4 · UDP · xsalsa20poly1305</td></tr>
          <tr><td>Лицензия</td><td class="mono">MIT</td></tr>
          </tbody>
        </table>
        <p class="hint small">
          Не аффилирован с Discord Inc. Клиент работает на пользовательском токене (как Vencord/Vesktop) —
          это серая зона ToS: безавтоматизационное использование терпимо, спам и self-ботинг = бан аккаунта.
          Используй разумно и на свой риск.
        </p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    z-index: 100;
  }
  .modal {
    position: fixed;
    inset: 0;
    margin: auto;
    width: min(720px, calc(100vw - 48px));
    height: min(560px, calc(100vh - 48px));
    background: var(--bg-primary);
    border-radius: var(--radius-md);
    z-index: 101;
    display: flex;
    flex-direction: column;
    animation: popIn 0.15s ease;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px 8px;
  }
  h1 { font-size: 20px; color: var(--header-primary); }
  .close {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .close:hover { background: var(--bg-modifier-hover); }
  .tabs {
    display: flex;
    gap: 4px;
    padding: 0 20px;
    border-bottom: 1px solid #26282c;
  }
  .tabs button {
    padding: 8px 12px;
    font-size: 14px;
    font-weight: 600;
    color: var(--text-muted);
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tabs button.active { color: #fff; border-bottom-color: #fff; }
  .content { padding: 20px; }
  .user-line { display: flex; align-items: center; gap: 14px; margin-bottom: 18px; }
  .big { font-size: 18px; font-weight: 700; color: var(--header-primary); }
  table { width: 100%; border-collapse: collapse; margin-bottom: 18px; }
  td { padding: 7px 10px; border-bottom: 1px solid #26282c66; font-size: 14px; }
  td:first-child { color: var(--text-muted); width: 160px; }
  .mono { font-family: var(--font-code); }
  .small { font-size: 12px; word-break: break-all; }
  .row { display: flex; gap: 8px; margin-bottom: 14px; }
  .hint { color: var(--text-muted); font-size: 13px; line-height: 1.6; margin-bottom: 14px; }
  .sf-form { display: flex; gap: 8px; margin-bottom: 14px; }
  .sf-form input {
    flex: 1;
    background: var(--bg-base);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    font-family: var(--font-code);
    font-size: 13px;
  }
  .sf-form textarea {
    flex: 1;
    background: var(--bg-base);
    border: none;
    outline: none;
    resize: vertical;
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    font-family: var(--font-code);
    font-size: 13px;
    color: var(--text-normal);
    line-height: 1.5;
  }
  .err { color: var(--red); font-size: 13px; margin-bottom: 12px; }
  .ok-msg { color: var(--green, #23a55a); font-size: 13px; line-height: 1.5; margin-bottom: 12px; }

  .sub {
    font-size: 13px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.02em;
    margin: 18px 0 8px;
  }

  .plugin-card {
    background: #2b2d31;
    border-radius: 8px;
    padding: 12px 14px;
    margin-bottom: 10px;
  }
  .plugin-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .plugin-title {
    display: flex;
    align-items: baseline;
    gap: 6px;
    color: var(--header-primary);
    flex-wrap: wrap;
  }
  .ok-tag {
    color: #23a55a;
    font-size: 11px;
    border: 1px solid #23a55a66;
    border-radius: 4px;
    padding: 0 5px;
  }
  .plugin-desc {
    color: var(--text-muted);
    font-size: 13px;
    margin: 6px 0;
    line-height: 1.5;
  }
  .plugin-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .plugin-actions .muted { margin-left: auto; }
  .mono-s { font-family: var(--font-code); font-size: 11px; }
  .mini {
    background: #4e5058;
    color: #dbdee1;
    border: none;
    border-radius: 4px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .mini:hover { background: #6d6f78; }
  .mini.danger { background: transparent; color: #f23f43; }
  .mini.danger:hover { background: #f23f4322; }
  .plugin-code {
    background: #1e1f22;
    border-radius: 6px;
    padding: 10px;
    margin-top: 10px;
    font-family: var(--font-code);
    font-size: 12px;
    line-height: 1.55;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 260px;
    overflow-y: auto;
    color: #b5bac1;
  }

  .switch {
    position: relative;
    display: inline-block;
    width: 40px;
    height: 22px;
    flex-shrink: 0;
    cursor: pointer;
  }
  .switch input { opacity: 0; width: 0; height: 0; }
  .switch span {
    position: absolute;
    inset: 0;
    background: #80848e;
    border-radius: 11px;
    transition: 0.2s;
  }
  .switch span::before {
    content: '';
    position: absolute;
    width: 16px;
    height: 16px;
    left: 3px;
    top: 3px;
    background: #fff;
    border-radius: 50%;
    transition: 0.2s;
  }
  .switch input:checked + span { background: #23a55a; }
  .switch input:checked + span::before { transform: translateX(18px); }
</style>
