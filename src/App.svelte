<script lang="ts">
  import { app } from './lib/stores/app.svelte';
  import LoginScreen from './lib/components/LoginScreen.svelte';
  import ServerRail from './lib/components/ServerRail.svelte';
  import ChannelSidebar from './lib/components/ChannelSidebar.svelte';
  import ChatArea from './lib/components/ChatArea.svelte';
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
    !!app.activeGuildId && app.showMembers && app.activeChannelId !== null && !isVoiceChannel,
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
      <div class="banner">
        {app.connStage === 'disconnected' ? 'Соединение потеряно' : 'Переподключаемся к Gateway'}…
        {app.connStage === 'disconnected' ? 'пробуем снова с backoff' : 'RESUME ' + (app.sessionId ? app.sessionId.slice(0, 8) + '…' : '')}
      </div>
    {/if}
    <div class="main">
      <ServerRail />
      <ChannelSidebar />
      {#if app.activeChannelId === null}
        <DmHome />
      {:else if isVoiceChannel}
        <VoiceView />
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
  .main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
</style>
