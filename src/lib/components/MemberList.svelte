<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { Member, MemberListRowData, Role } from '../types';
  import Avatar from './Avatar.svelte';

  const guild = $derived(app.activeGuild);
  const guildId = $derived(guild?.id ?? '');

  /* ================= ленивый список (GUILD_MEMBER_LIST_UPDATE) =================
   * Сервер присылает сайдбар УЖЕ собранным: заголовки групп и участников
   * с пресенсами вперемешку, в готовом порядке (ответ на op 37 с channels,
   * диапазон [[0,99]]). Рендерим как есть — 1:1 с официальным клиентом. */

  type LazyMember = Extract<MemberListRowData, { kind: 'member' }>;

  const lazy = $derived(app.memberLists[guildId]);
  /** Снапшот соответствует текущему каналу → рендерим его, иначе фолбэк. */
  const lazyActive = $derived(
    !!lazy && lazy.rows.length > 0 && lazy.channel_id === (app.activeChannelId ?? '0'),
  );

  function groupTitle(id: string): string {
    if (id === 'online') return 'В СЕТИ';
    if (id === 'offline') return 'ОФЛАЙН';
    if (id === 'unknown') return 'УЧАСТНИКИ';
    const role = guild?.roles.find((r) => r.id === id);
    return (role?.name ?? '…').toUpperCase();
  }

  function rowName(r: LazyMember): string {
    return r.nick || r.global_name || r.username;
  }

  function rowActivity(r: LazyMember): string | null {
    const custom = r.activities?.find((a) => a.type === 4);
    if (custom?.state) return custom.state;
    const game = r.activities?.find((a) => a.type === 0);
    return game?.name ?? null;
  }

  /** Цвет ника — по высшей цветной роли участника (как в официале). */
  function rowColor(r: LazyMember): string {
    const top = (guild?.roles ?? [])
      .filter((role) => role.color && r.roles.includes(role.id))
      .sort((a, b) => b.position - a.position)[0];
    return top ? `#${top.color.toString(16).padStart(6, '0')}` : 'var(--header-primary)';
  }

  function rowUser(r: LazyMember) {
    return {
      id: r.user_id,
      username: r.username,
      global_name: r.global_name ?? null,
      avatar: r.avatar ?? null,
      bot: r.bot ?? false,
    };
  }

  /* ================= фолбэк: union карточек и пресенсов =================
   * Бот-аккаунты (op 8 → GUILD_MEMBERS_CHUNK) и мгновение до первого SYNC. */

  /** Строка списка: полноценный участник либо presence-only юзер. */
  interface Row {
    id: string;
    member: Member | null;
    presenceUser: { id: string; username?: string; global_name?: string | null; avatar?: string | null } | null;
    name: string;
    status: string;
    hasMember: boolean;
  }

  const OFFLINE_CAP = 100;

  const rows = $derived.by(() => {
    if (!guild) return [] as Row[];
    const out: Row[] = [];
    const seen = new Set<string>();
    for (const m of Object.values(app.membersByGuild[guildId] ?? {})) {
      if (!m.user || seen.has(m.user.id)) continue;
      seen.add(m.user.id);
      out.push({
        id: m.user.id,
        member: m,
        presenceUser: null,
        name: m.nick || m.user.global_name || m.user.username,
        status: app.statusOf(guildId, m.user.id),
        hasMember: true,
      });
    }
    // Presence-only: онлайн-участники без карточки (большие серверы).
    for (const p of Object.values(app.presencesByGuild[guildId] ?? {})) {
      if (!p.user?.id || seen.has(p.user.id) || p.status === 'offline') continue;
      seen.add(p.user.id);
      out.push({
        id: p.user.id,
        member: null,
        presenceUser: p.user,
        name: p.user.global_name || p.user.username || `id:${p.user.id.slice(-4)}`,
        status: p.status,
        hasMember: false,
      });
    }
    return out;
  });

  interface Group {
    role: Role | null; // null → offline
    members: Row[];
  }

  const groups = $derived.by(() => {
    if (!guild) return [] as Group[];
    const hoisted = [...guild.roles].filter((r) => r.hoist).sort((a, b) => b.position - a.position);
    const result: Group[] = [];
    for (const role of hoisted) {
      const list = rows.filter((r) => {
        if (r.status === 'offline') return false;
        return app.topRole(guildId, r.id)?.id === role.id;
      });
      if (list.length) result.push({ role, members: list });
    }
    // online members without hoisted role
    const noRole = rows.filter((r) => {
      return r.status !== 'offline' && !app.topRole(guildId, r.id);
    });
    if (noRole.length)
      result.push({ role: guild.roles.find((r) => r.id === guildId) ?? null, members: noRole });
    // offline (с потолком, как оригинал — остальные скрыты «ещё N»)
    const offline = rows.filter((r) => r.status === 'offline');
    if (offline.length) result.push({ role: null, members: offline });
    return result;
  });

  const onlineCount = $derived(rows.filter((r) => r.status !== 'offline').length);
  const offlineHidden = $derived(
    Math.max(
      0,
      rows.filter((r) => r.status === 'offline').length - OFFLINE_CAP,
    ),
  );

  function colorOf(r: Row): string {
    return r.hasMember ? app.memberColor(guildId, r.id) : 'var(--header-primary)';
  }

  function activity(r: Row): string | null {
    if (!app.presencesByGuild[guildId]?.[r.id]) return null;
    const p = app.presencesByGuild[guildId][r.id];
    const custom = p?.activities?.find((a) => a.type === 4);
    if (custom?.state) return custom.state;
    const game = p?.activities?.find((a) => a.type === 0);
    if (game) return game.name;
    return null;
  }

  /** Юзер-аккаунты ждут первый SYNC ленивого списка, боты — чанки op 8.
   *  Демо (mode='mock') синхронно присылает memberList — не мигаем. */
  const loading = $derived(
    !!guild &&
      app.mode === 'tauri' &&
      (app.isBotAccount ? !app.chunksDone[guildId] : !lazyActive),
  );

  const shownOnline = $derived(lazyActive && lazy ? lazy.online_count : onlineCount);
  const shownTotal = $derived(lazyActive && lazy ? lazy.member_count : guild?.member_count);

  /* ================= догрузка при скролле (математика официала) =================
   * onScroll → GUILD_SUBSCRIPTIONS_CHANNEL из бандла discord.com:
   *   l(x, extra) = max(0, ceil(x / rowHeight) + extra)  — видимые строки
   *   overscan = l(высота/2); start = l(scrollTop, -overscan);
   *   end = l(scrollTop + высота, overscan)
   *   диапазоны: всегда [0,99] (если start > 0), дальше сотнями
   *   [floor(start/100)*100 … end]. Сервер присылает SYNC только по
   *   НОВЫМ диапазонам — повторная подписка не плодит трафик. */
  const ROW_HEIGHT = 40;
  const CHUNK = 100;
  let lastRangesKey = '';
  let scrollTimer: ReturnType<typeof setTimeout> | null = null;

  // Сменился канал — стартовая подписка уже ушла из selectChannel ([[0,99]]),
  // кэш диапазонов скролла обнуляем.
  $effect(() => {
    void app.activeChannelId;
    lastRangesKey = '';
  });

  function onMembersScroll(e: Event) {
    if (!lazyActive) return; // список ещё не приехал — нечего расширять
    if (scrollTimer) clearTimeout(scrollTimer);
    scrollTimer = setTimeout(() => {
      scrollTimer = null;
      const el = e.currentTarget as HTMLElement;
      const rows = (x: number, extra = 0) => Math.max(0, Math.ceil(x / ROW_HEIGHT) + extra);
      const overscan = rows(el.clientHeight * 0.5);
      let start = rows(el.scrollTop, -overscan);
      const end = rows(el.scrollTop + el.clientHeight, overscan);
      const ranges: [number, number][] = [];
      const push = (s: number) => {
        ranges.push([s, s + CHUNK - 1]);
        return s + CHUNK;
      };
      if (start > 0) start = Math.max(push(0), start);
      start = Math.floor(start / CHUNK) * CHUNK;
      for (; start <= end; ) start = push(start);
      const key = ranges.map(([a, b]) => `${a}:${b}`).join(',');
      if (key === lastRangesKey) return; // ничего не изменилось
      lastRangesKey = key;
      app.subscribeMemberListRanges(ranges);
    }, 150);
  }

  /** Участников в загруженных строках (для футера «…и ещё N»). */
  const memberRows = $derived(
    lazy ? lazy.rows.reduce((n, r) => (r.kind === 'member' ? n + 1 : n), 0) : 0,
  );
  const hiddenCount = $derived(
    lazyActive && lazy && lazy.member_count > memberRows ? lazy.member_count - memberRows : 0,
  );
</script>

<aside class="members" onscroll={onMembersScroll}>
  {#if loading}
    <div class="chunk-hint muted">
      Загружаем участников…
    </div>
  {/if}

  <div class="member-stats muted">
    В сети — {shownOnline}
    {#if shownTotal}
      · всего {shownTotal}
    {/if}
  </div>

  {#if lazyActive && lazy}
    {#each lazy.rows as row (row.kind === 'member' ? `m:${row.user_id}` : `g:${row.id}`)}
      {#if row.kind === 'group'}
        <div class="group-title lazy muted">
          {groupTitle(row.id)} — {row.count}
        </div>
      {:else}
        <button
          class="member"
          class:offline={row.status === 'offline'}
          onclick={() => (app.profileCard = { userId: row.user_id, guildId })}
          oncontextmenu={(e) => {
            e.preventDefault();
            app.profileCard = { userId: row.user_id, guildId };
          }}
        >
          <Avatar
            user={rowUser(row) as never}
            size={32}
            status={row.status}
            ringColor="var(--bg-secondary)"
          />
          <div class="meta">
            <span class="name truncate" style="color: {rowColor(row)}">
              {rowName(row)}
              {#if row.bot}<span class="bot">APP</span>{/if}
            </span>
            {#if rowActivity(row)}
              <span class="activity truncate muted">{rowActivity(row)}</span>
            {/if}
          </div>
        </button>
      {/if}
    {/each}
    {#if hiddenCount > 0}
      <div class="offline-more lazy-more">…и ещё {hiddenCount} — прокрутите ниже, подгрузим</div>
    {/if}
  {:else}
    {#each groups as g (g.role?.id ?? 'offline')}
      <div class="group">
        <div class="group-title muted">
          {g.role ? g.role.name.toUpperCase() : 'ОФЛАЙН'} — {g.members.length}
        </div>
        {#each g.role === null ? g.members.slice(0, OFFLINE_CAP) : g.members as m (m.id)}
          <button
            class="member"
            class:offline={g.role === null}
            onclick={() => (app.profileCard = { userId: m.id, guildId })}
            oncontextmenu={(e) => {
              e.preventDefault();
              app.profileCard = { userId: m.id, guildId };
            }}
          >
            <Avatar
              user={(m.member?.user ?? m.presenceUser) as never}
              size={32}
              status={g.role === null ? 'offline' : m.status}
              ringColor="var(--bg-secondary)"
            />
            <div class="meta">
              <span class="name truncate" style="color: {colorOf(m)}">
                {m.name}
                {#if m.member?.user?.bot}<span class="bot">APP</span>{/if}
              </span>
              {#if activity(m)}
                <span class="activity truncate muted">{activity(m)}</span>
              {/if}
            </div>
          </button>
        {/each}
        {#if g.role === null && offlineHidden > 0}
          <div class="offline-more muted">…и ещё {offlineHidden} не в сети</div>
        {/if}
      </div>
    {/each}
  {/if}
  <div class="pad"></div>
</aside>

<style>
  .members {
    width: 240px;
    flex-shrink: 0;
    background: var(--bg-secondary);
    overflow-y: auto;
    padding-top: 8px;
  }
  .chunk-hint {
    font-size: 12px;
    padding: 8px 16px;
  }
  .member-stats {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    padding: 8px 16px 4px;
  }
  .group { margin-bottom: 16px; }
  .group-title {
    font-size: 11px;
    font-weight: 700;
    padding: 8px 16px 4px;
    letter-spacing: 0.02em;
  }
  /* Ленивый режим: группы и участники — плоский список в серверном порядке. */
  .group-title.lazy { margin-top: 16px; }
  .member {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 5px 8px 5px 16px;
  }
  .member:hover { background: var(--bg-modifier-hover); border-radius: var(--radius-sm); }
  .member.offline { opacity: 0.4; }
  .meta { display: flex; flex-direction: column; min-width: 0; text-align: left; }
  .name { font-size: 15px; font-weight: 600; }
  .bot {
    background: var(--blurple);
    color: #fff;
    font-size: 9px;
    font-weight: 800;
    padding: 1px 4px;
    border-radius: 3px;
    margin-left: 5px;
    vertical-align: 2px;
  }
  .activity { font-size: 12px; }
  .offline-more {
    font-size: 11px;
    padding: 4px 16px;
  }
  /* Ленивый режим: футер под последней группой. */
  .lazy-more {
    margin-top: 8px;
    margin-bottom: 8px;
  }
  .pad { height: 24px; }
</style>
