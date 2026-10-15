<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { Member, Role } from '../types';
  import Avatar from './Avatar.svelte';

  const guild = $derived(app.activeGuild);
  const guildId = $derived(guild?.id ?? '');

  /** Строка списка: полноценный участник либо presence-only юзер
   *  (merged_presences/op 14 дают статусы людей, чья карточка участника
   *  ещё не приехала — оригинал показывает их так же). */
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
</script>

<aside class="members">
  {#if !app.chunksDone[guildId] && guild}
    <div class="chunk-hint muted">
      Загружаем участников…
    </div>
  {/if}

  <div class="member-stats muted">
    В сети — {onlineCount}
    {#if guild?.member_count}
      · всего {guild.member_count}
    {/if}
  </div>

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
  .pad { height: 24px; }
</style>
