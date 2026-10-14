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
        {app.connStage === 'disconnected' ? 'Соединение потеряно' : 'Переподключаемся к Gateway'}…
        {#if app.connDetail}
          <span class="banner-detail"> · {app.connDetail}</span>
        {:else if app.connStage === 'disconnected'}
          <span class="banner-detail"> · пробуем снова с backoff</span>
        {:else}
          <span class="banner-detail"> · RESUME {(app.sessionId || '').slice(0, 8)}…</span>
        {/if}
      </div>
    {/if}
    {#if app.updateInfo}
      <button class="update-chip" onclick={() => { void app.openReleaseUrl(); app.markUpdateSeen(); }}
        title={app.updateInfo.notes || 'Открыть страницу релиза'}>
        ✨ Доступна версия {app.updateInfo.latest} — скачать
      </button>
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
    font-family: var(--font-code);
  }
  .banner-detail { font-weight: 400; opacity: 0.85; }
  .update-chip {
    position: fixed;
    bottom: 16px;
    right: 16px;
    z-index: 110;
    background: var(--blurple);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    border-radius: 10px;
    padding: 10px 16px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.4);
  }
  .update-chip:hover { filter: brightness(1.1); }
  .main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
</style>
