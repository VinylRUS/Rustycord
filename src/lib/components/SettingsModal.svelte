<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { SnowflakeInfo } from '../types';
  import { formatDuration } from '../time';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  let tab = $state<'connection' | 'snowflake' | 'protocol' | 'about'>('connection');
  let snowflakeInput = $state('');
  let snowflakeResult = $state<SnowflakeInfo | null>(null);
  let snowflakeError = $state('');
  let rawFrame = $state('{\n  "op": 8,\n  "d": { "guild_id": "...", "query": "", "limit": 0 }\n}');
  let rawStatus = $state<{ kind: 'ok' | 'err'; text: string } | null>(null);

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
    ['GUILD_VOICE_STATES', 128],
    ['GUILD_PRESENCES', 256],
    ['GUILD_MESSAGES', 512],
    ['GUILD_MESSAGE_TYPING', 1024],
    ['DIRECT_MESSAGES', 4096],
    ['DIRECT_MESSAGE_TYPING', 8192],
    ['MESSAGE_CONTENT', 32768],
  ] as const;
  const intentsSum = $derived(intents.reduce((acc, [, v]) => acc + v, 0));
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
          <tr><td>Intents</td><td class="mono">{intentsSum} (9 intents)</td></tr>
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
          Не аффилирован с Discord Inc. Использует только официально документированные API
          (discord.com/developers). Self-ботинг не поддерживается принципиально.
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
</style>
