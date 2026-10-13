<script lang="ts">
  import { app } from '../stores/app.svelte';
  import Icon from './Icon.svelte';

  let token = $state('');
  let showHelp = $state(false);

  async function submit() {
    if (!token.trim()) {
      app.loginError = 'Вставь Bot Token — получить можно на discord.com/developers/applications';
      return;
    }
    await app.login(token.trim());
  }

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

    <form onsubmit={(e) => { e.preventDefault(); void submit(); }}>
      <label class="field-label" for="token">BOT TOKEN</label>
      <input
        id="token"
        type="password"
        placeholder="MTIzNDU2Nzg5MDEyMzQ1Njc4.XXXXXX.XXXXXXXXXXXXXXXXXXXXXXXXXXX"
        bind:value={token}
        autocomplete="off"
        spellcheck="false"
      />
      <button class="btn-blurple submit" type="submit" disabled={app.loginBusy}>
        {app.loginBusy ? 'Подключаемся к Gateway…' : 'Войти'}
      </button>
    </form>

    <button class="btn-ghost demo" onclick={() => void demo()} disabled={app.loginBusy}>
      🦀 Открыть демо-режим (без токена)
    </button>

    <button class="help-toggle" onclick={() => (showHelp = !showHelp)}>
      {showHelp ? 'Скрыть' : 'Как получить токен?'}
    </button>

    {#if showHelp}
      <ol class="help">
        <li>Открой <a href="https://discord.com/developers/applications" target="_blank" rel="noreferrer">discord.com/developers/applications</a> → <b>New Application</b></li>
        <li>Вкладка <b>Bot</b> → <b>Reset Token</b> → скопируй токен</li>
        <li>Там же включи все три <b>Privileged Gateway Intents</b> (Presence, Server Members, Message Content)</li>
        <li>OAuth2 → URL Generator → scope <code>bot</code> → права <code>Administrator</code> → открой ссылку и пригласи бота на свой сервер</li>
        <li>Вставь токен выше — и увидишь свой сервер в клиенте</li>
      </ol>
      <p class="note">🔒 Токен хранится только локально (localStorage) и уходит только на discord.com</p>
      <p class="note">⚠️ Self-bot (user token) нарушает ToS Discord — поэтому только bot-токены.</p>
    {/if}
  </div>

  <div class="footer">
    <span>Gateway wss://gateway.discord.gg · REST api/v10 · Intents 46979</span>
    <span>RustyCord v0.1.0 · не аффилирован с Discord Inc.</span>
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
    margin-bottom: 20px;
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
  .demo {
    width: 100%;
    margin-top: 8px;
    color: var(--text-link);
  }
  .help-toggle {
    width: 100%;
    margin-top: 16px;
    color: var(--text-link);
    font-size: 13px;
  }
  .help-toggle:hover { text-decoration: underline; }
  .help {
    margin: 12px 0 4px 20px;
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.7;
  }
  .help code {
    background: #1e1f22;
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 12px;
  }
  .note {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 8px;
  }
  .footer {
    position: fixed;
    bottom: 12px;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
    padding: 0 16px;
    color: #ffffff66;
    font-size: 11px;
    font-family: var(--font-code);
  }
</style>
