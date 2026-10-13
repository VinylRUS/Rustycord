<script lang="ts">
  import { app } from '../stores/app.svelte';
  import { defaultAvatarColor, guildIconUrl } from '../avatars';
  import Icon from './Icon.svelte';

  const guilds = $derived(app.guilds);

  function unreadOfGuild(guildId: string): boolean {
    const g = app.guilds.find((x) => x.id === guildId);
    return (g?.channels ?? []).some((c) => app.unread[c.id]);
  }

  function select(id: string | null) {
    app.selectGuild(id);
  }
</script>

<nav class="rail">
  <div class="pill-container">
    <button
      class="server home"
      class:active={app.activeGuildId === null}
      onclick={() => select(null)}
      title="Личные сообщения"
    >
      <span class="tooltip">Личные сообщения</span>
      <Icon name="logo" size={26} color={app.activeGuildId === null ? '#fff' : '#dbdee1'} />
    </button>
    <span class="pill" class:visible={app.activeGuildId === null}></span>
  </div>

  <div class="separator"></div>

  {#each guilds as guild (guild.id)}
    {@const icon = guildIconUrl(guild)}
    {@const active = app.activeGuildId === guild.id}
    {@const unread = unreadOfGuild(guild.id)}
    <div class="pill-container">
      <button class="server" class:active onclick={() => select(guild.id)}>
        <span class="tooltip">{guild.name}</span>
        {#if icon}
          <img src={icon} alt={guild.name} />
        {:else}
          <span class="guild-initial" style="background: {defaultAvatarColor(guild.id)}">
            {guild.name.split(' ').map((w) => w[0]).join('').slice(0, 2).toUpperCase()}
          </span>
        {/if}
      </button>
      <span class="pill" class:visible={active}></span>
      {#if unread && !active}
        <span class="unread-dot"></span>
      {/if}
    </div>
  {/each}

  <div class="pill-container">
    <button class="server add" title="Пригласить бота на новый сервер" onclick={() => app.toast('Пригласи бота через OAuth2 URL Generator — см. README', 'info')}>
      <span class="tooltip">Добавить сервер</span>
      <Icon name="plus" size={22} color="#23a55a" />
    </button>
    <span class="pill"></span>
  </div>
</nav>

<style>
  .rail {
    width: 72px;
    flex-shrink: 0;
    background: var(--bg-base);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 12px 0;
    gap: 8px;
    overflow-y: auto;
    scrollbar-width: none;
  }
  .rail::-webkit-scrollbar { display: none; }

  .pill-container {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    justify-content: center;
  }

  /* left active pill like Discord */
  .pill {
    position: absolute;
    left: 0;
    width: 4px;
    height: 8px;
    border-radius: 0 4px 4px 0;
    background: #fff;
    opacity: 0;
    transition: height 0.15s ease, opacity 0.15s ease;
  }
  .pill.visible {
    opacity: 1;
    height: 40px;
  }

  .server {
    width: 48px;
    height: 48px;
    border-radius: 24px; /* hover → circle */
    transition: border-radius 0.15s ease;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    position: relative;
    cursor: pointer;
  }
  .server:hover,
  .server.active {
    border-radius: 16px;
  }
  .server.home {
    background: transparent;
  }
  .server.home.active,
  .server.home:hover {
    background: #5865f2;
  }
  .server:not(.home) {
    background: #313338;
  }
  .server:not(.home):hover {
    background: #5865f2;
  }
  .server.active:not(.home) {
    background: #5865f2;
  }
  .server img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .guild-initial {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    font-weight: 600;
    font-size: 16px;
  }
  .server.add {
    background: transparent;
  }
  .server.add:hover {
    background: #23a55a;
  }
  .server.add:hover :global(svg) {
    stroke: #fff;
  }

  .unread-dot {
    position: absolute;
    bottom: -2px;
    right: 6px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #fff;
  }

  .separator {
    width: 32px;
    height: 2px;
    border-radius: 1px;
    background: #35363c;
    flex-shrink: 0;
  }

  /* Discord-style tooltip */
  .tooltip {
    position: absolute;
    left: calc(100% + 12px);
    top: 50%;
    transform: translateY(-50%);
    background: #111214;
    color: #fff;
    font-size: 14px;
    font-weight: 600;
    padding: 8px 12px;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.1s ease 0.05s;
    z-index: 100;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }
  .server:hover .tooltip {
    opacity: 1;
  }
</style>
