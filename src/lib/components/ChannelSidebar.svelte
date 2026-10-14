<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { Channel } from '../types';
  import { displayName } from '../avatars';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import { menu } from '../stores/menu.svelte';

  let collapsed = $state<Record<string, boolean>>({});
  let dmFilter = $state('');

  const STATUSES: { id: 'online' | 'idle' | 'dnd' | 'invisible'; label: string }[] = [
    { id: 'online', label: 'В сети' },
    { id: 'idle', label: 'Неактивен' },
    { id: 'dnd', label: 'Не беспокоить' },
    { id: 'invisible', label: 'Невидимка' },
  ];

  /** Клик по своему аватару: выбор статуса (юзер-аккаунт). */
  function statusMenu(e: MouseEvent) {
    if (app.isBotAccount) {
      app.showSettings = true;
      return;
    }
    menu.show(
      e.clientX,
      e.clientY,
      STATUSES.map((s) => ({
        label: `${s.label}${app.myStatus === s.id ? ' ✓' : ''}`,
        action: () => void app.setStatus(s.id),
      })),
    );
  }

  const isGuild = $derived(app.activeGuildId !== null);
  const guild = $derived(app.activeGuild);
  const channels = $derived(app.activeGuildChannels);

  const uncategorized = $derived(channels.filter((c) => c.type !== 4 && !c.parent_id));
  const categories = $derived(channels.filter((c) => c.type === 4));

  function childrenOf(catId: string): Channel[] {
    return channels.filter((c) => c.parent_id === catId && c.type !== 4);
  }

  function isText(c: Channel): boolean {
    return c.type === 0 || c.type === 5;
  }

  function isVoice(c: Channel): boolean {
    return c.type === 2 || c.type === 13;
  }

  function isActive(id: string): boolean {
    return app.activeChannelId === id;
  }

  function channelClick(c: Channel) {
    if (isVoice(c) && guild) {
      void app.toggleVoice(guild.id, c.id);
      app.selectChannel(c.id); // show voice view
    } else {
      app.selectChannel(c.id);
    }
  }

  const filteredDms = $derived(
    dmFilter
      ? app.dms.filter((c) => (c.recipients ?? []).some((r) => displayName(r).toLowerCase().includes(dmFilter.toLowerCase())))
      : app.dms,
  );

  /** Друзья, подходящие под поиск, но без открытой беседы — можно начать переписку. */
  const friendHits = $derived.by(() => {
    if (!dmFilter.trim() || app.isBotAccount) return [];
    const q = dmFilter.trim().toLowerCase();
    return app.friendsList.filter((r) => {
      const hay = `${displayName(r.user)} ${r.user.username}`.toLowerCase();
      return hay.includes(q) && !app.dmWith(r.user.id);
    });
  });
</script>

<aside class="sidebar">
  {#if isGuild && guild}
    <!-- Guild header: клик открывает настройки сервера (как в оригинале) -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <header class="guild-header clickable" onclick={() => (app.showGuildSettings = guild.id)} title="Настройки сервера">
      <span class="guild-name truncate">{guild.name}</span>
      <Icon name="settings" size={16} color="#949ba4" />
    </header>

    <div class="scroller">
      <!-- Uncategorized channels -->
      {#each uncategorized as c (c.id)}
        {#if isText(c) || isVoice(c)}
          {@render channelRow(c)}
        {/if}
      {/each}

      <!-- Categories -->
      {#each categories as cat (cat.id)}
        <button
          class="category"
          onclick={() => (collapsed[cat.id] = !collapsed[cat.id])}
        >
          <span class="cat-chevron" class:closed={collapsed[cat.id]}>
            <Icon name="chevron" size={12} color="#949ba4" />
          </span>
          <span class="cat-name truncate">{cat.name?.toUpperCase()}</span>
        </button>
        {#if !collapsed[cat.id]}
          {#each childrenOf(cat.id) as c (c.id)}
            {@render channelRow(c)}
          {/each}
        {/if}
      {/each}
      <div class="scroll-bottom-pad"></div>
    </div>
  {:else}
    <!-- DM home sidebar -->
    <div class="dm-find">
      <button class="friends-btn" class:active={app.activeChannelId === null} onclick={() => app.selectChannel(null)}>
        <Icon name="members" size={22} color={app.activeChannelId === null ? '#fff' : '#949ba4'} />
        <span>Друзья</span>
      </button>
    </div>
    <div class="scroller">
      <div class="dm-search">
        <input placeholder="Найти или начать беседу" bind:value={dmFilter} />
      </div>
      {#each filteredDms as c (c.id)}
        {@const u = c.recipients?.[0]}
        <button
          class="dm-item"
          class:active={app.activeChannelId === c.id}
          onclick={() => app.selectChannel(c.id)}
        >
          <Avatar user={u} size={32} status={app.statusOf(app.activeGuild?.id ?? '', u?.id ?? '')} ringColor="var(--bg-secondary)" />
          <span class="dm-name truncate">{c.recipients?.map((r) => displayName(r)).join(', ')}</span>
          {#if app.unread[c.id]}
            <span class="unread-badge"></span>
          {/if}
        </button>
      {/each}
      {#if friendHits.length}
        <div class="dm-section">Начать беседу</div>
        {#each friendHits as r (r.user.id)}
          <button class="dm-item new-dm" onclick={() => void app.openDmWith(r.user.id)}>
            <Avatar user={r.user} size={32} status={app.friendStatus(r.user.id)} ringColor="var(--bg-secondary)" />
            <span class="dm-name truncate">{displayName(r.user)}</span>
            <Icon name="plus" size={16} color="var(--green)" />
          </button>
        {/each}
      {/if}
      {#if app.isDemo}
        <div class="dm-note">
          Демо-режим: сервер, участники и друзья симулируются локально — в приложении здесь будут настоящие данные твоего аккаунта.
        </div>
      {/if}
    </div>
  {/if}

  {#if app.myVoice && guild}
    <!-- Voice connection panel -->
    <div class="voice-panel">
      <div class="voice-info">
        <div class="voice-status">
          <Icon name="signal" size={16} color="#23a55a" />
          <span>Голос подключён</span>
        </div>
        <div class="voice-target truncate">
          {app.channelById(app.myVoice.channelId)?.name ?? 'Канал'} / {app.myVoice.mode ?? 'connecting…'}
        </div>
        {#if app.myVoice.endpoint}
          <div class="voice-endpoint truncate">{app.myVoice.endpoint}{#if app.myVoice.ssrc} · ssrc {app.myVoice.ssrc}{/if}</div>
        {/if}
      </div>
      <!-- Трансляции: экран и камера (эксперимент) -->
      <button
        class="voice-btn"
        class:active={app.myStreams.some((s) => s.type === 'screen')}
        title="Показать экран"
        onclick={() => void app.toggleStream('screen')}
      >
        <Icon name="screen" size={18} color={app.myStreams.some((s) => s.type === 'screen') ? '#23a55a' : '#b5bac1'} />
      </button>
      <button
        class="voice-btn"
        class:active={app.myStreams.some((s) => s.type === 'camera')}
        title="Веб-камера"
        onclick={() => void app.toggleStream('camera')}
      >
        <Icon name="camera" size={18} color={app.myStreams.some((s) => s.type === 'camera') ? '#23a55a' : '#b5bac1'} />
      </button>
      <button class="voice-leave" title="Отключиться" onclick={() => void app.leaveVoice()}>
        <Icon name="phone-off" size={20} color="#f23f43" />
      </button>
    </div>
  {/if}

  <!-- User panel -->
  <footer class="user-panel">
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div
      class="user-id clickable"
      title={app.isBotAccount ? 'Профиль' : 'Сменить статус'}
      onclick={(e) => statusMenu(e)}
    >
      <Avatar user={app.user} size={32} status={app.isBotAccount ? 'online' : app.myStatus} ringColor="var(--bg-secondary)" />
      <div class="user-meta">
        <span class="username truncate">{app.user ? (app.user.global_name || app.user.username) : '—'}</span>
        <span class="usertag truncate">{app.user ? `@${app.user.username}` : ''}</span>
      </div>
    </div>
    <div class="user-actions">
      <button
        class="icon-btn"
        class:off={app.myVoice?.selfMute}
        title={app.myVoice?.selfMute ? 'Включить микрофон' : 'Выключить микрофон'}
        onclick={() => void app.setSelfMute(!app.myVoice?.selfMute)}
      >
        <Icon name={app.myVoice?.selfMute ? 'mic-off' : 'mic'} size={18} color={app.myVoice?.selfMute ? '#f23f43' : '#b5bac1'} />
      </button>
      <button
        class="icon-btn"
        class:off={app.myVoice?.selfDeaf}
        title={app.myVoice?.selfDeaf ? 'Включить звук' : 'Выключить звук'}
        onclick={() => void app.setSelfDeaf(!app.myVoice?.selfDeaf)}
      >
        <Icon name={app.myVoice?.selfDeaf ? 'headphones-off' : 'headphones'} size={18} color={app.myVoice?.selfDeaf ? '#f23f43' : '#b5bac1'} />
      </button>
      <button class="icon-btn" title="Настройки" onclick={() => (app.showSettings = true)}>
        <Icon name="settings" size={18} color="#b5bac1" />
      </button>
    </div>
  </footer>
</aside>

<!-- one channel row: text or voice -->
{#snippet channelRow(c: Channel)}
  {#if isVoice(c)}
    <div class="voice-block">
      <button class="channel" class:active={isActive(c.id)} class:joined={app.myVoice?.channelId === c.id} onclick={() => channelClick(c)}>
        <Icon name="speaker" size={20} color={isActive(c.id) || app.myVoice?.channelId === c.id ? '#f2f3f5' : '#949ba4'} />
        <span class="channel-name truncate">{c.name}</span>
        {#if guild}
          <span class="voice-count">{app.voiceUsers(guild.id, c.id).length || ''}</span>
        {/if}
      </button>
      {#if guild}
        {#each app.voiceUsers(guild.id, c.id) as vs (vs.user_id)}
          {@const member = app.memberById(guild.id, vs.user_id)}
          {@const speaking = !!app.speaking[vs.user_id]}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="voice-user clickable" onclick={() => (app.profileCard = { userId: vs.user_id, guildId: guild.id })}>
            <Avatar
              user={member?.user ?? null}
              size={20}
              speaking={speaking}
              ringColor="var(--bg-secondary)"
            />
            <span class="voice-username truncate" class:muted-user={vs.self_mute}>{member?.nick || member?.user?.global_name || member?.user?.username || vs.user_id}</span>
            {#if vs.self_mute}
              <Icon name="mic-off" size={12} color="#f23f43" />
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  {:else}
    <button class="channel" class:active={isActive(c.id)} onclick={() => channelClick(c)}>
      <Icon name="hash" size={20} color={isActive(c.id) ? '#f2f3f5' : '#949ba4'} />
      <span class="channel-name truncate">{c.name}</span>
      {#if app.unread[c.id] && !isActive(c.id)}
        <span class="unread-badge"></span>
      {/if}
    </button>
  {/if}
{/snippet}

<style>
  .sidebar {
    width: 240px;
    flex-shrink: 0;
    background: var(--bg-secondary);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  /* ---- guild mode ---- */
  .guild-header {
    height: 48px;
    padding: 0 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
    box-shadow: 0 1px 0 rgba(0, 0, 0, 0.25);
    font-weight: 600;
    color: var(--header-primary);
  }
  .guild-header:hover { background: #35373c; }
  .guild-name { font-size: 15px; }

  .category {
    display: flex;
    align-items: center;
    gap: 2px;
    width: 100%;
    padding: 16px 8px 4px 8px;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }
  .category:hover { color: var(--header-primary); }
  .cat-chevron {
    display: flex;
    transition: transform 0.15s ease;
  }
  .cat-chevron.closed { transform: rotate(-90deg); }
  .cat-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .channel {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 8px;
    margin: 1px 8px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-size: 15px;
    font-weight: 500;
    position: relative;
  }
  .channel:hover { background: var(--bg-modifier-hover); color: var(--text-normal); }
  .channel.active {
    background: var(--bg-modifier-active);
    color: #fff;
  }
  .channel.joined :global(svg) { color: var(--green); }
  .channel-name { flex: 1; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .voice-count { font-size: 12px; color: var(--text-muted); }

  .voice-block { display: flex; flex-direction: column; }
  .voice-user {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 8px 2px 26px;
    margin-left: 8px;
    color: var(--text-muted);
  }
  .voice-username { font-size: 14px; font-weight: 500; }
  .muted-user { opacity: 0.6; }

  .unread-badge {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #fff;
    flex-shrink: 0;
  }

  .scroll-bottom-pad { height: 16px; }

  /* ---- DM mode ---- */
  .dm-find { padding: 12px 8px 0 8px; }
  .friends-btn {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 8px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-size: 15px;
    font-weight: 500;
  }
  .friends-btn:hover, .friends-btn.active { background: var(--bg-modifier-active); color: #fff; }
  .dm-search { padding: 0 8px 8px 8px; }
  .dm-search input {
    width: 100%;
    background: var(--bg-base);
    border-radius: var(--radius-sm);
    padding: 6px 8px;
    font-size: 13px;
    color: var(--text-normal);
  }
  .dm-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px 8px;
    margin: 1px 0;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-size: 15px;
    font-weight: 500;
  }
  .dm-item:hover { background: var(--bg-modifier-hover); color: var(--text-normal); }
  .dm-item.active { background: var(--bg-modifier-active); color: #fff; }
  .dm-item.new-dm .dm-name { color: var(--green); font-weight: 500; }
  .dm-section {
    padding: 14px 8px 4px 8px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .dm-name { flex: 1; text-align: left; }
  .dm-note {
    margin: 12px 8px;
    padding: 10px;
    background: #11121488;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.5;
  }

  /* ---- voice panel ---- */
  .voice-panel {
    background: var(--channeldeep);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px;
    flex-shrink: 0;
    animation: fadeIn 0.2s ease;
  }
  .voice-info { min-width: 0; flex: 1; }
  .voice-status {
    display: flex;
    align-items: center;
    gap: 6px;
    color: #23a55a;
    font-size: 13px;
    font-weight: 600;
  }
  .voice-target { color: var(--text-muted); font-size: 12px; margin-top: 1px; }
  .voice-endpoint { color: var(--text-muted); font-size: 10px; font-family: var(--font-code); opacity: 0.7; }
  .voice-btn {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .voice-btn:hover { background: #ffffff14; }
  .voice-btn.active { background: #23a55a22; }
  .voice-leave {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .voice-leave:hover { background: #f23f4322; }

  /* ---- user panel ---- */
  .user-panel {
    height: 52px;
    background: var(--channeldeep);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 8px;
    flex-shrink: 0;
  }
  .user-id { display: flex; align-items: center; gap: 8px; min-width: 0; border-radius: var(--radius-sm); padding: 2px; }
  .user-id.clickable:hover { background: var(--bg-modifier-hover); }
  .user-meta { display: flex; flex-direction: column; min-width: 0; }
  .username { color: #fff; font-size: 13px; font-weight: 600; line-height: 1.2; }
  .usertag { color: var(--text-muted); font-size: 11px; line-height: 1.2; }
  .user-actions { display: flex; gap: 2px; }
  .icon-btn {
    width: 30px;
    height: 30px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .icon-btn:hover { background: var(--bg-modifier-hover); }
</style>
