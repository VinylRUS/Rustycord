<script lang="ts">
  import { app } from './lib/stores/app.svelte';
  import LoginScreen from './lib/components/LoginScreen.svelte';
  import ServerRail from './lib/components/ServerRail.svelte';
  import ChannelSidebar from './lib/components/ChannelSidebar.svelte';
  import ChatArea from './lib/components/ChatArea.svelte';
  import ForumView from './lib/components/ForumView.svelte';
  import MemberList from './lib/components/MemberList.svelte';
  import VoiceView from './lib/components/VoiceView.svelte';
  import DmHome from './lib/components/DmHome.svelte';
  import ContextMenu from './lib/components/ContextMenu.svelte';
  import ProfileCard from './lib/components/ProfileCard.svelte';
  import SettingsModal from './lib/components/SettingsModal.svelte';
  import GuildSettingsModal from './lib/components/GuildSettingsModal.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import QuickSwitcher from './lib/components/QuickSwitcher.svelte';

  // Быстрый переключатель — Ctrl+K / Cmd+K (как в оригинале).
  // В браузерной разработке без Tauri не перехватываем (DevTools и адресная
  // строка браузера важнее), в приложении — глобальный хоткей.
  const isTauri = '__TAURI_INTERNALS__' in globalThis;
  // DEV-хук: живой стор в консоли браузера (window.__app.guilds и т.д.)
  if (import.meta.env.DEV) (window as never as { __app: unknown }).__app = app;
  function onGlobalKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && (e.key === 'k' || e.key === 'K')) {
      e.preventDefault();
      app.showQuickSwitcher = !app.showQuickSwitcher;
    }
  }
  $effect(() => {
    if (!isTauri) return;
    window.addEventListener('keydown', onGlobalKey);
    return () => window.removeEventListener('keydown', onGlobalKey);
  });

  // Auto-login with a saved token (Tauri only).
  $effect(() => {
    void app.boot();
  });

  // Window title like the real client.
  $effect(() => {
    const ch = app.activeChannel;
    const guild = app.activeGuild;
    const chName = ch
      ? ch.type === 1 || ch.type === 3
        ? app.channelTitle
        : `#${ch.name}`
      : null;
    document.title = chName
      ? `${chName} | ${guild?.name ?? 'Личные сообщения'} | RustyCord`
      : 'RustyCord';
  });

  const isVoiceChannel = $derived(app.activeChannel?.type === 2 || app.activeChannel?.type === 13);
  const showMemberList = $derived(
    !!app.activeGuildId && app.showMembers && app.activeChannelId !== null && !isVoiceChannel && !app.showPins && !app.showSearch,
  );
  const showReconnectBanner = $derived(
    app.user !== null && (app.connStage === 'reconnecting' || app.connStage === 'connecting' || app.connStage === 'disconnected'),
  );
</script>

{#if !app.user}
  <LoginScreen />
{:else}
  <div class="layout">
    {#if showReconnectBanner}
      <div class="banner" title={app.connDetail || ''}>
        {app.connStage === 'disconnected' ? 'Соединение потеряно — переподключаемся…' : 'Переподключаемся…'}
      </div>
    {/if}
    {#if app.updateInfo}
      <div class="update-card" role="status">
        {#if app.updatePhase === 'idle'}
          <div class="update-head">
            <span class="update-title" title={app.updateInfo.notes || ''}>
              ✨ RustyCord {app.updateInfo.latest}
            </span>
            <button class="update-close" onclick={() => app.dismissUpdate()} title="Скрыть до следующей версии">✕</button>
          </div>
          {#if app.updateInfo.notes}
            <div class="update-notes">{app.updateInfo.notes}</div>
          {/if}
          <div class="update-actions">
            {#if app.updateInfo.can_auto}
              <button class="update-btn primary" onclick={() => { void app.installUpdate(); }}>
                Обновить автоматически
              </button>
            {/if}
            <button class="update-btn ghost" onclick={() => { void app.openReleaseUrl(); }}>
              {app.updateInfo.can_auto ? 'Скачать вручную' : 'Открыть страницу релиза'}
            </button>
          </div>
        {:else if app.updatePhase === 'downloading'}
          <div class="update-head">
            <span class="update-title">Загрузка обновления… {Math.round(app.updateProgress * 100)}%</span>
          </div>
          <div class="update-bar">
            <div class="update-bar-fill" style="width: {Math.max(4, app.updateProgress * 100)}%"></div>
          </div>
        {:else if app.updatePhase === 'ready'}
          <div class="update-head">
            <span class="update-title">✓ Установлено — версия {app.updateInfo.latest}</span>
          </div>
          <div class="update-actions">
            <button class="update-btn primary" onclick={() => { void app.relaunchApp(); }}>
              Перезапустить сейчас
            </button>
          </div>
        {:else if app.updatePhase === 'error'}
          <div class="update-head">
            <span class="update-title err">Не удалось обновиться</span>
          </div>
          {#if app.updateError}
            <div class="update-notes">{app.updateError}</div>
          {/if}
          <div class="update-actions">
            <button class="update-btn primary" onclick={() => { void app.openReleaseUrl(); }}>
              Скачать вручную
            </button>
            <button class="update-btn ghost" onclick={() => app.dismissUpdate()}>Позже</button>
          </div>
        {/if}
      </div>
    {/if}
    <div class="main">
      <ServerRail />
      <ChannelSidebar />
      {#if app.activeChannelId === null}
        <DmHome />
      {:else if isVoiceChannel}
        <VoiceView />
      {:else if app.isForumView}
        <ForumView />
      {:else}
        <ChatArea />
      {/if}
      {#if showMemberList}
        <MemberList />
      {/if}
    </div>
  </div>
{/if}

<ContextMenu />
<ProfileCard />
<SettingsModal />
<GuildSettingsModal />
<QuickSwitcher />
<Toasts />
<!-- Капча — на любом экране (вход и авторизованные действия) -->

<style>
  .layout {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .banner {
    background: var(--yellow);
    color: #1e1f22;
    font-weight: 600;
    font-size: 13px;
    text-align: center;
    padding: 6px;
    flex-shrink: 0;
  }
  .update-card {
    position: fixed;
    bottom: 16px;
    right: 16px;
    z-index: 110;
    width: 300px;
    background: var(--bg-tertiary, #2b2d31);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 12px;
    padding: 12px 14px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .update-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .update-title {
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .update-title.err { color: #f5776c; }
  .update-close {
    background: none;
    border: none;
    color: #b5bac1;
    font-size: 13px;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 4px;
    flex-shrink: 0;
  }
  .update-close:hover { background: rgba(255, 255, 255, 0.08); color: #fff; }
  .update-notes {
    font-size: 12px;
    line-height: 1.35;
    color: #b5bac1;
    max-height: 72px;
    overflow: hidden;
    white-space: pre-line;
  }
  .update-actions { display: flex; gap: 8px; }
  .update-btn {
    flex: 1;
    font-size: 12.5px;
    font-weight: 600;
    border-radius: 8px;
    padding: 7px 10px;
    cursor: pointer;
    border: none;
    transition: filter 0.15s;
  }
  .update-btn:hover { filter: brightness(1.12); }
  .update-btn.primary { background: var(--blurple, #5865f2); color: #fff; }
  .update-btn.ghost {
    background: rgba(255, 255, 255, 0.06);
    color: #dbdee1;
    border: 1px solid rgba(255, 255, 255, 0.08);
  }
  .update-bar {
    height: 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }
  .update-bar-fill {
    height: 100%;
    border-radius: 3px;
    background: var(--blurple, #5865f2);
    transition: width 0.2s ease;
  }
  .main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
</style>
