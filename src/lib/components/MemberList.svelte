<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { Member, Role } from '../types';
  import Avatar from './Avatar.svelte';

  const guild = $derived(app.activeGuild);
  const guildId = $derived(guild?.id ?? '');

  interface Group {
    role: Role | null; // null → offline
    members: Member[];
  }

  const groups = $derived.by(() => {
    if (!guild) return [] as Group[];
    const members = Object.values(app.membersByGuild[guildId] ?? {});
    const hoisted = [...guild.roles].filter((r) => r.hoist).sort((a, b) => b.position - a.position);
    const result: Group[] = [];
    for (const role of hoisted) {
      const list = members.filter((m) => {
        if (!m.user) return false;
        const status = app.statusOf(guildId, m.user.id);
        if (status === 'offline') return false;
        return app.topRole(guildId, m.user.id)?.id === role.id;
      });
      if (list.length) result.push({ role, members: list });
    }
    // online members without hoisted role
    const noRole = members.filter((m) => {
      if (!m.user) return false;
      return app.statusOf(guildId, m.user.id) !== 'offline' && !app.topRole(guildId, m.user.id);
    });
    if (noRole.length) result.push({ role: guild.roles.find((r) => r.id === guildId) ?? null, members: noRole });
    // offline
    const offline = members.filter((m) => m.user && app.statusOf(guildId, m.user.id) === 'offline');
    if (offline.length) result.push({ role: null, members: offline });
    return result;
  });

  const totalOnline = $derived(
    Object.values(app.membersByGuild[guildId] ?? {}).filter((m) => m.user && app.statusOf(guildId, m.user.id) !== 'offline').length,
  );

  function name(m: Member): string {
    return m.nick || m.user?.global_name || m.user?.username || '?';
  }

  function activity(m: Member): string | null {
    if (!m.user) return null;
    const p = app.presencesByGuild[guildId]?.[m.user.id];
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
      Загружаем участников (GUILD_MEMBERS_CHUNK)…
    </div>
  {/if}
  {#each groups as g (g.role?.id ?? 'offline')}
    <div class="group">
      <div class="group-title muted">
        {g.role ? g.role.name.toUpperCase() : 'ОФЛАЙН'} — {g.members.length}
      </div>
      {#each g.members as m (m.user!.id)}
        <button
          class="member"
          class:offline={g.role === null}
          onclick={() => (app.profileCard = { userId: m.user!.id, guildId })}
          oncontextmenu={(e) => {
            e.preventDefault();
            app.profileCard = { userId: m.user!.id, guildId };
          }}
        >
          <Avatar
            user={m.user}
            size={32}
            status={g.role === null ? 'offline' : app.statusOf(guildId, m.user!.id)}
            ringColor="var(--bg-secondary)"
          />
          <div class="meta">
            <span class="name truncate" style="color: {app.memberColor(guildId, m.user!.id)}">
              {name(m)}
              {#if m.user?.bot}<span class="bot">APP</span>{/if}
            </span>
            {#if activity(m)}
              <span class="activity truncate muted">{activity(m)}</span>
            {/if}
          </div>
        </button>
      {/each}
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
  .pad { height: 24px; }
</style>
