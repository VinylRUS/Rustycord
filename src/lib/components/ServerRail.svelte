<script lang="ts">
  import { app } from '../stores/app.svelte';
  import { defaultAvatarColor, guildIconUrl } from '../avatars';
  import Icon from './Icon.svelte';

  /** Иконки, чей CDN-запрос упал (устаревший хэш/сеть) — показываем инициалы. */
  let iconFailed = $state<Record<string, boolean>>({});
  /** Раскрытая папка (index) — flyout с серверами папки. */
  let openFolder = $state<number | null>(null);

  let showJoin = $state(false);
  let invite = $state('');
  let joining = $state(false);
  let newServerName = $state('');
  let creatingServer = $state(false);

  function select(id: string | null) {
    openFolder = null;
    app.selectGuild(id);
  }

  // Инициалы для кружка без иконки. name бывает undefined (GUILD_CREATE
  // юзер-сессий без верхнеуровневого name) — без защиты TypeError из
  // name.split ронял весь {#each} и рейл серверов исчезал целиком.
  function initials(name?: string | null): string {
    const s = (name ?? '').trim();
    if (!s) return '?';
    return s
      .split(/\s+/)
      .map((w) => w[0])
      .join('')
      .slice(0, 2)
      .toUpperCase();
  }

  function colorOf(i: number | null | undefined): string {
    if (i == null) return '#5865f2';
    return `#${(i & 0xffffff).toString(16).padStart(6, '0')}`;
  }

  function folderTitle(index: number): string {
    const f = app.guildFolders[index];
    if (!f) return 'Папка';
    if (f.name) return f.name;
    // Безымянная папка — официальные клиенты показывают имя первого сервера.
    const first = f.guild_ids
      .map((gid) => app.guilds.find((g) => g.id === gid)?.name)
      .find(Boolean);
    return first ?? 'Папка';
  }

  async function submitJoin() {
    joining = true;
    await app.joinServer(invite);
    joining = false;
    showJoin = false;
    invite = '';
  }

  /** Свой сервер (v0.8.0): POST /guilds {name} — как кнопка в оригинале. */
  async function submitCreateServer() {
    creatingServer = true;
    await app.createServer(newServerName.trim());
    creatingServer = false;
    showJoin = false;
    newServerName = '';
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
    <span
      class="badge"
      class:show={Object.entries(app.mentions).some(
        ([cid, n]) => n > 0 && app.dms.some((dm) => dm.id === cid),
      )}
    >
      {Object.entries(app.mentions).reduce(
        (acc, [cid, n]) => acc + (app.dms.some((dm) => dm.id === cid) ? n : 0),
        0,
      ) || ''}
    </span>
  </div>

  <div class="separator"></div>

  {#each app.railItems as item, i (item.type === 'folder' ? `f${item.index}` : item.id)}
    {#if item.type === 'guild'}
      {@const guild = app.guilds.find((g) => g.id === item.id)}
      {#if guild}
        {@const icon = iconFailed[guild.id] ? null : guildIconUrl(guild)}
        {@const active = app.activeGuildId === guild.id}
        {@const mentions = app.mentionCountOfGuild(guild.id)}
        {@const unread = app.unreadOfGuild(guild.id)}
        <div class="pill-container">
          <button class="server" class:active onclick={() => select(guild.id)}>
            <span class="tooltip">{guild.name ?? 'Без названия'}</span>
            {#if icon}
              <img
                src={icon}
                alt={guild.name ?? ''}
                loading="lazy"
                onerror={() => (iconFailed[guild.id] = true)}
              />
            {:else}
              <span class="guild-initial" style="background: {defaultAvatarColor(guild.id)}">
                {initials(guild.name)}
              </span>
            {/if}
          </button>
          <span class="pill" class:visible={active}></span>
          {#if mentions > 0 && !active}
            <span class="badge show">{mentions > 99 ? '99+' : mentions}</span>
          {:else if unread && !active}
            <span class="unread-dot"></span>
          {/if}
        </div>
      {/if}
    {:else}
      {@const f = app.guildFolders[item.index]}
      {#if f && f.guild_ids.length}
        {@const mentions = app.mentionCountOfFolder(item.index)}
        {@const unread = app.unreadOfFolder(item.index)}
        {@const active = f.guild_ids.includes(app.activeGuildId ?? '')}
        {@const expanded = openFolder === item.index}
        <div class="pill-container">
          <button
            class="server folder"
            class:active
            class:expanded
            style="--folder-color: {colorOf(f.color)}"
            onclick={() => (openFolder = expanded ? null : item.index)}
          >
            <span class="tooltip">{folderTitle(item.index)}</span>
            <span class="folder-body">
              {#each f.guild_ids.slice(0, 4) as gid, j}
                {@const g = app.guilds.find((x) => x.id === gid)}
                {#if g}
                  <span class="mini" data-pos={j}>
                    {#if iconFailed[gid]}
                      <span class="mini-initial" style="background: {defaultAvatarColor(gid)}">
                        {initials(g.name)}
                      </span>
                    {:else if guildIconUrl(g)}
                      <img
                        src={guildIconUrl(g) ?? ''}
                        alt={g.name ?? ''}
                        loading="lazy"
                        onerror={() => (iconFailed[gid] = true)}
                      />
                    {:else}
                      <span class="mini-initial" style="background: {defaultAvatarColor(gid)}">
                        {initials(g.name)}
                      </span>
                    {/if}
                  </span>
                {/if}
              {/each}
              {#if f.guild_ids.length > 4}<span class="mini-more">+{f.guild_ids.length - 4}</span>{/if}
            </span>
          </button>
          <span class="pill" class:visible={active || expanded}></span>
          {#if mentions > 0 && !active}
            <span class="badge show">{mentions > 99 ? '99+' : mentions}</span>
          {:else if unread && !active}
            <span class="unread-dot"></span>
          {/if}
        </div>
      {/if}
    {/if}
  {/each}

  {#if openFolder !== null && app.guildFolders[openFolder]}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="folder-backdrop" onclick={() => (openFolder = null)}></div>
    <div class="folder-pop">
      {#if app.guildFolders[openFolder!].name}
        <div class="folder-pop-title">{app.guildFolders[openFolder!].name}</div>
      {/if}
      {#each app.guildFolders[openFolder!].guild_ids as gid (gid)}
        {@const g = app.guilds.find((x) => x.id === gid)}
        {#if g}
          {@const icon = iconFailed[gid] ? null : guildIconUrl(g)}
          {@const gActive = app.activeGuildId === gid}
          {@const gMentions = app.mentionCountOfGuild(gid)}
          <button
            class="folder-server"
            class:active={gActive}
            onclick={() => select(gid)}
          >
            {#if icon}
              <img src={icon} alt={g.name ?? ''} loading="lazy" onerror={() => (iconFailed[gid] = true)} />
            {:else}
              <span class="guild-initial" style="background: {defaultAvatarColor(gid)}">
                {initials(g.name)}
              </span>
            {/if}
            <span class="folder-server-name truncate">{g.name}</span>
            {#if gMentions > 0 && !gActive}
              <span class="badge show">{gMentions > 99 ? '99+' : gMentions}</span>
            {/if}
          </button>
        {/if}
      {/each}
    </div>
  {/if}

  <div class="pill-container">
    <button
      class="server add"
      title={app.isBotAccount ? 'Пригласить бота на сервер' : 'Присоединиться к серверу'}
      onclick={() => (showJoin = !showJoin)}
    >
      <span class="tooltip">Добавить сервер</span>
      <Icon name="plus" size={22} color="#23a55a" />
    </button>
    <span class="pill"></span>
  </div>
</nav>

{#if showJoin}
  <div class="join-pop">
    <h4>{app.isBotAccount ? 'Пригласить бота' : 'Присоединиться к серверу'}</h4>
    {#if app.isBotAccount}
      <p class="join-hint">Бот-аккаунты присоединяются по OAuth2-ссылке (URL Generator). Войди пользователем, чтобы заходить по приглашению.</p>
    {:else}
      <p class="join-hint">Вставь ссылку-приглашение (discord.gg/…) или код.</p>
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void submitJoin();
        }}
      >
        <input placeholder="discord.gg/rustycord" bind:value={invite} spellcheck="false" />
        <button type="submit" disabled={!invite.trim() || joining}>
          {joining ? 'Входим…' : 'Войти'}
        </button>
      </form>
      <!-- v0.8.0: создание своего сервера -->
      <div class="create-sep"></div>
      <h4>Создать свой сервер</h4>
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void submitCreateServer();
        }}
      >
        <input placeholder="Название сервера" bind:value={newServerName} maxlength={100} />
        <button type="submit" disabled={!newServerName.trim() || creatingServer}>
          {creatingServer ? 'Создаём…' : 'Создать'}
        </button>
      </form>
    {/if}
    <button class="join-close" onclick={() => (showJoin = false)}>Закрыть</button>
  </div>
{/if}

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

  /* ---- попап «присоединиться к серверу» ---- */
  .join-pop {
    position: fixed;
    left: 84px;
    bottom: 24px;
    width: 320px;
    background: var(--bg-primary);
    border-radius: var(--radius-md);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    padding: 18px;
    z-index: 60;
    animation: joinIn 0.15s ease;
  }
  @keyframes joinIn {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }
  .join-pop h4 {
    color: var(--header-primary);
    font-size: 15px;
    margin-bottom: 8px;
  }
  .join-hint {
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.5;
    margin-bottom: 10px;
  }
  .join-pop form {
    display: flex;
    gap: 8px;
    margin-bottom: 10px;
  }
  .join-pop input {
    flex: 1;
    background: var(--bg-base);
    border-radius: var(--radius-sm);
    padding: 9px 10px;
    font-size: 14px;
    color: var(--text-normal);
  }
  .join-pop form button {
    background: #5865f2;
    color: #fff;
    border-radius: var(--radius-sm);
    padding: 9px 14px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .join-pop form button:disabled { opacity: 0.5; cursor: not-allowed; }
  .join-close {
    width: 100%;
    color: var(--text-muted);
    font-size: 13px;
  }
  .join-close:hover { color: var(--text-normal); text-decoration: underline; }

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
  .server:not(.home):not(.folder) {
    background: #313338;
  }
  .server:not(.home):not(.folder):hover {
    background: #5865f2;
  }
  .server.active:not(.home):not(.folder) {
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

  /* ---- папка серверов (как в оригинале) ---- */
  .server.folder {
    background: transparent;
    padding: 0;
  }
  .folder-body {
    width: 100%;
    height: 100%;
    background: #313338;
    border-radius: inherit;
    position: relative;
  }
  .server.folder:hover .folder-body,
  .server.folder.active .folder-body,
  .server.folder.expanded .folder-body {
    background: var(--folder-color, #5865f2);
  }
  .server.folder.active,
  .server.folder.expanded,
  .server.folder:hover {
    border-radius: 16px;
  }
  .mini {
    position: absolute;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    overflow: hidden;
    background: #1e1f22;
  }
  .mini[data-pos='0'] { top: 6px; left: 6px; }
  .mini[data-pos='1'] { top: 6px; right: 6px; }
  .mini[data-pos='2'] { bottom: 6px; left: 6px; }
  .mini[data-pos='3'] { bottom: 6px; right: 6px; }
  .mini img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .mini-initial {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    font-size: 9px;
    font-weight: 700;
  }
  .mini-more {
    position: absolute;
    bottom: 4px;
    right: 4px;
    color: #fff;
    font-size: 8px;
    font-weight: 700;
  }

  /* раскрывающийся список папки */
  .folder-backdrop {
    position: fixed;
    inset: 0;
    z-index: 69;
  }
  .folder-pop {
    position: fixed;
    left: 84px;
    top: 50%;
    transform: translateY(-50%);
    width: 260px;
    max-height: 70vh;
    overflow-y: auto;
    background: var(--bg-primary);
    border-radius: var(--radius-md);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    padding: 10px;
    z-index: 70;
    animation: joinIn 0.15s ease;
  }
  .folder-pop-title {
    color: var(--header-primary);
    font-size: 13px;
    font-weight: 700;
    padding: 4px 10px 10px;
  }
  .folder-server {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px 10px;
    border-radius: var(--radius-sm);
  }
  .folder-server:hover { background: var(--bg-modifier-hover); }
  .folder-server.active { background: var(--bg-modifier-active); }
  .folder-server img {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }
  .folder-server .guild-initial {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    font-size: 12px;
    flex-shrink: 0;
  }
  .folder-server-name {
    color: var(--text-normal);
    font-size: 14px;
    font-weight: 600;
    flex: 1;
    text-align: left;
  }

  /* красный бейдж упоминаний */
  .badge {
    position: absolute;
    bottom: -2px;
    right: 0;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: #f23f43;
    color: #fff;
    font-size: 11px;
    font-weight: 700;
    line-height: 16px;
    text-align: center;
    display: none;
    box-shadow: 0 0 0 3px var(--bg-base);
  }
  .badge.show { display: block; }

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
  .create-sep {
    height: 1px;
    background: #3f4147;
    margin: 14px 0 10px;
  }
</style>
