<script lang="ts">
  import { app } from '../stores/app.svelte';
  import Icon from './Icon.svelte';

  let tab = $state<'qr' | 'email' | 'token'>('qr');
  let email = $state('');
  let password = $state('');
  let mfaCode = $state('');
  let token = $state('');
  let showHelp = $state(false);

  async function submitEmail() {
    if (app.mfaTicket) {
      if (!mfaCode.trim()) {
        app.loginError = 'Введи код из приложения-аутентификатора';
        return;
      }
      await app.submitMfa(mfaCode.trim());
    } else {
      if (!email.trim() || !password) {
        app.loginError = 'Заполни email и пароль';
        return;
      }
      await app.loginEmail(email.trim(), password);
    }
  }

  async function submitToken() {
    if (!token.trim()) {
      app.loginError = 'Вставь токен сессии (или войди по QR — так надёжнее)';
      return;
    }
    await app.login(token.trim());
  }

  function switchTab(next: 'qr' | 'email' | 'token') {
    tab = next;
    app.loginError = '';
    if (next !== 'qr') void app.cancelQr();
  }

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  // QR-таб в приложении: сразу запускаем сессию. В браузере — по кнопке
  // (мок-QR всё равно ведёт в демо-режим, автостарт был бы неожиданным).
  $effect(() => {
    if (isTauri && tab === 'qr' && app.qrStage === 'idle' && !app.loginBusy && !app.user) {
      void app.startQr();
    }
  });

  async function demo() {
    await app.startDemo();
  }
</script>

<div class="login-bg">
  <div class="login-card">
    <div class="logo">
      <Icon name="logo" size={56} color="#5865f2" />
    </div>
    <h1>С возвращением!</h1>
    <p class="sub">RustyCord — неофициальный клиент Discord на <b>Rust + Tauri</b></p>

    {#if app.loginError}
      <div class="error">{app.loginError}</div>
    {/if}

    <div class="tabs" role="tablist">
      <button role="tab" class:active={tab === 'qr'} onclick={() => switchTab('qr')}>QR-код</button>
      <button role="tab" class:active={tab === 'email'} onclick={() => switchTab('email')}>Email и пароль</button>
      <button role="tab" class:active={tab === 'token'} onclick={() => switchTab('token')}>Токен</button>
    </div>

    {#if tab === 'qr'}
      <div class="qr-pane">
        {#if app.qrStage === 'idle' && !app.qrSvg}
          <div class="qr-box placeholder">
            <p>QR-логин работает в приложении.<br />В браузере можно посмотреть демо.</p>
          </div>
          <button class="btn-blurple submit" onclick={() => void app.startQr()}>
            {isTauri ? 'Сгенерировать QR-код' : 'Показать демо-QR'}
          </button>
        {:else if app.qrStage === 'error' && !app.qrSvg}
          <div class="qr-box placeholder">
            <p>Не удалось подключиться к Discord.<br />Если ошибка повторяется — проверь сеть или VPN.</p>
          </div>
        {:else if !app.qrSvg}
          <div class="qr-box placeholder">
            <div class="spinner"></div>
            <p>{app.qrStage === 'connecting' ? 'Подключаемся к Discord…' : 'Генерируем QR-код…'}</p>
          </div>
        {:else}
          <div class="qr-box" class:scanned={app.qrStage === 'scanned' || app.qrStage === 'done'}>
            <!-- eslint-disable-next-line svelte/no-at-html-tags — SVG генерирует Rust (qrcode crate), не пользовательский ввод -->
            {@html app.qrSvg}
          </div>
        {/if}
        {#if app.qrStage === 'fingerprint'}
          <p class="hint">Открой приложение Discord на телефоне: <b>Настройки → Устройства → Сканировать QR-код</b></p>
        {:else if app.qrStage === 'scanned'}
          <p class="hint ok">Отсканировал{app.qrScannedBy ? ` @${app.qrScannedBy}` : 'и'} — подтвердите вход на телефоне ✅</p>
        {:else if app.qrStage === 'timeout' || app.qrStage === 'cancelled' || app.qrStage === 'error'}
          <button class="btn-blurple submit" onclick={() => void app.startQr()}>Попробовать снова</button>
        {/if}
        <p class="note">Как в официальном клиенте: токен зашифрован RSA и приходит только на это устройство.</p>
      </div>
    {:else if tab === 'email'}
      <form onsubmit={(e) => { e.preventDefault(); void submitEmail(); }}>
        {#if app.mfaTicket}
          <label class="field-label" for="mfa">КОД ДВУХФАКТОРНОЙ АУТЕНТИФИКАЦИИ</label>
          <input
            id="mfa"
            type="text"
            inputmode="numeric"
            placeholder="123 456"
            bind:value={mfaCode}
            autocomplete="one-time-code"
            maxlength="8"
          />
          <button class="btn-blurple submit" type="submit" disabled={app.loginBusy}>
            {app.loginBusy ? 'Проверяем код…' : 'Подтвердить'}
          </button>
          <button type="button" class="btn-ghost" onclick={() => { app.mfaTicket = null; app.loginError = ''; }}>
            ← Назад к входу
          </button>
        {:else}
          <label class="field-label" for="email">EMAIL ИЛИ ТЕЛЕФОН</label>
          <input id="email" type="text" placeholder="you@example.com" bind:value={email} autocomplete="username" />
          <label class="field-label" for="password">ПАРОЛЬ</label>
          <input id="password" type="password" placeholder="••••••••" bind:value={password} autocomplete="current-password" />
          <button class="btn-blurple submit" type="submit" disabled={app.loginBusy}>
            {app.loginBusy ? 'Входим…' : 'Войти'}
          </button>
        {/if}
      </form>
      <p class="note">Иногда Discord требует капчу при входе с нового IP — тогда используй QR-код, он капчу не требует.</p>
    {:else}
      <form onsubmit={(e) => { e.preventDefault(); void submitToken(); }}>
        <label class="field-label" for="token">ТОКЕН СЕССИИ</label>
        <input
          id="token"
          type="password"
          placeholder="MTE5NzQ… (из localStorage официального клиента)"
          bind:value={token}
          autocomplete="off"
          spellcheck="false"
        />
        <button class="btn-blurple submit" type="submit" disabled={app.loginBusy}>
          {app.loginBusy ? 'Подключаемся к Gateway…' : 'Войти'}
        </button>
      </form>
      <p class="note">Токен живёт в localStorage браузера (ключ <code>token</code>) или в DevTools → Application.</p>
    {/if}

    <button class="btn-ghost demo" onclick={() => void demo()} disabled={app.loginBusy}>
      🦀 Открыть демо-режим (без входа)
    </button>

    <button class="help-toggle" onclick={() => (showHelp = !showHelp)}>
      {showHelp ? 'Скрыть' : 'Насколько это безопасно?'}
    </button>

    {#if showHelp}
      <div class="help">
        <p><b>Токен</b> — это вся сессия: он хранится только локально (localStorage) и уходит только на discord.com.</p>
        <p><b>Риск:</b> клиент на пользовательском токене — серая зона ToS Discord (как Vencord/Vesktop). Аккаунт за такое банят редко, но автоматизация/спам от своего имени = быстрый бан без разбана. Используй на свой страх и риск.</p>
        <p><b>Совет:</b> не вставляй токен в чужие программы; выходи из сессии (Настройки → Устройства) на чужих компьютерах.</p>
      </div>
    {/if}
  </div>

  <div class="footer">
    <span>Gateway wss://gateway.discord.gg · REST api/v10 · remote-auth v2 · Intents 131071</span>
    <span>RustyCord v0.4.0 · не аффилирован с Discord Inc.</span>
  </div>
</div>

<style>
  .login-bg {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: radial-gradient(ellipse at top, #404eed 0%, #2b2d31 65%);
    padding: 24px;
    overflow-y: auto;
  }
  .login-card {
    width: min(480px, 100%);
    background: var(--bg-primary);
    border-radius: var(--radius-md);
    padding: 32px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    animation: popIn 0.25s ease;
  }
  .logo {
    display: flex;
    justify-content: center;
    margin-bottom: 8px;
  }
  h1 {
    text-align: center;
    color: var(--header-primary);
    font-size: 24px;
    line-height: 1.3;
    margin-bottom: 8px;
  }
  .sub {
    text-align: center;
    color: var(--text-muted);
    font-size: 15px;
    margin-bottom: 16px;
  }
  .error {
    background: #f23f4322;
    border: 1px solid #f23f4366;
    color: #ff8a8d;
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    font-size: 14px;
    margin-bottom: 16px;
  }
  .tabs {
    display: flex;
    gap: 4px;
    background: #1e1f22;
    border-radius: var(--radius-sm);
    padding: 4px;
    margin-bottom: 20px;
  }
  .tabs button {
    flex: 1;
    background: transparent;
    color: var(--text-muted);
    font-size: 13px;
    font-weight: 600;
    padding: 8px 6px;
    border-radius: 4px;
    transition: 0.15s;
  }
  .tabs button:hover {
    color: var(--text-normal);
  }
  .tabs button.active {
    background: var(--bg-primary);
    color: var(--text-normal);
  }

  .qr-pane {
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .qr-box {
    width: 208px;
    height: 208px;
    background: #fff;
    border-radius: var(--radius-sm);
    padding: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: opacity 0.3s;
  }
  :global(.qr-box svg) {
    width: 100%;
    height: 100%;
    image-rendering: pixelated;
  }
  .qr-box.scanned {
    opacity: 0.45;
  }
  .qr-box.placeholder {
    background: #1e1f22;
    color: var(--text-muted);
    flex-direction: column;
    gap: 10px;
    font-size: 13px;
  }
  .spinner {
    width: 28px;
    height: 28px;
    border: 3px solid #5865f2;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
  .hint {
    color: var(--text-normal);
    font-size: 14px;
    text-align: center;
    margin-top: 14px;
    line-height: 1.5;
  }
  .hint.ok {
    color: #57f287;
    font-weight: 600;
  }

  .field-label {
    display: block;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.02em;
    margin-bottom: 8px;
  }
  input {
    width: 100%;
    background: #1e1f22;
    border-radius: var(--radius-sm);
    padding: 10px;
    font-size: 14px;
    margin-bottom: 16px;
    color: var(--text-normal);
  }
  .submit {
    width: 100%;
    font-size: 15px;
  }
  .note {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 12px;
    line-height: 1.5;
    text-align: center;
  }
  .note code {
    background: #1e1f22;
    padding: 1px 5px;
    border-radius: 3px;
  }
  .demo {
    width: 100%;
    margin-top: 8px;
    color: var(--text-link);
  }
  .help-toggle {
    width: 100%;
    margin-top: 12px;
    color: var(--text-link);
    font-size: 13px;
  }
  .help-toggle:hover { text-decoration: underline; }
  .help {
    margin: 12px 0 4px 0;
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.6;
    background: #1e1f22;
    border-radius: var(--radius-sm);
    padding: 12px 14px;
  }
  .help p + p { margin-top: 8px; }
  .footer {
    margin-top: 16px;
    display: flex;
    justify-content: space-between;
    gap: 16px;
    color: #ffffff66;
    font-size: 11px;
    font-family: var(--font-code);
  }
</style>
