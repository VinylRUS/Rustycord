<script lang="ts">
  import { app } from '../stores/app.svelte';
  import { computePermissions, summarizePermissions } from '../permissions';
  import { memberAvatarUrl, userTag } from '../avatars';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import { menu } from '../stores/menu.svelte';

  const card = $derived(app.profileCard);
  const guild = $derived(card ? app.guilds.find((g) => g.id === card.guildId) : null);
  const member = $derived(card && guild ? app.membersByGuild[guild.id]?.[card.userId] : null);
  const user = $derived(member?.user ?? app.userById(card?.userId ?? ''));

  const roles = $derived.by(() => {
    if (!guild || !member) return [];
    return guild.roles
      .filter((r) => member.roles.includes(r.id))
      .sort((a, b) => b.position - a.position);
  });

  const topColor = $derived(roles.find((r) => r.color !== 0)?.color ?? 0x5865f2);
  const permSummary = $derived.by(() => {
    if (!guild || !member) return [];
    return summarizePermissions(computePermissions(member.roles, guild.roles));
  });

  const isSelf = $derived(card?.userId === app.user?.id);

  function modMenu(e: MouseEvent) {
    if (!card || isSelf) return;
    e.preventDefault();
    menu.show(e.clientX, e.clientY, [
      { label: 'Кикнуть', danger: true, action: () => void app.kickMember(card.guildId, card.userId) },
      { label: 'Забанить', danger: true, action: () => void app.banMember(card.guildId, card.userId) },
    ]);
  }

  const avatarSrc = $derived(
    guild && member?.avatar && user ? memberAvatarUrl(guild.id, user.id, member.avatar) : null,
  );
</script>

{#if card && user}
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => (app.profileCard = null)} oncontextmenu={(e) => e.preventDefault()}></div>
<!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="profile-card" oncontextmenu={modMenu}>
    <div class="banner" style="background: linear-gradient(135deg, #{topColor.toString(16).padStart(6, '0')}, #1e1f22 140%)"></div>
    <div class="avatar-row">
      <Avatar user={user} size={80} ringColor="#232428" />
    </div>
    <div class="body">
      <div class="name">{member?.nick || user.global_name || user.username}</div>
      <button class="tag" onclick={() => void navigator.clipboard?.writeText(userTag(user))} title="Скопировать">
        {userTag(user)} <Icon name="copy" size={12} color="var(--text-muted)" />
      </button>

      {#if member?.communication_disabled_until && new Date(member.communication_disabled_until) > new Date()}
        <div class="timeout">⏳ Тайм-аут до {new Date(member.communication_disabled_until).toLocaleString('ru-RU')}</div>
      {/if}

      {#if roles.length}
        <div class="section-title">Роли</div>
        <div class="chips">
          {#each roles.slice(0, 8) as r (r.id)}
            <span class="chip">
              {#if r.color}<span class="chip-dot" style="background: #{r.color.toString(16).padStart(6, '0')}"></span>{/if}
              {r.name}
            </span>
          {/each}
        </div>
      {/if}

      {#if permSummary.length}
        <div class="section-title">Ключевые права</div>
        <ul class="perms">
          {#each permSummary as p (p)}
            <li>{p}</li>
          {/each}
        </ul>
      {/if}

      <div class="section-title">ID</div>
      <div class="snowflake mono">{user.id}</div>

      {#if !isSelf}
        <div class="mod-actions">
          <button class="mod-btn" onclick={() => void app.kickMember(card.guildId, card.userId)}>
            <Icon name="kick" size={16} color="#f23f43" /> Кикнуть
          </button>
          <button class="mod-btn" onclick={() => void app.banMember(card.guildId, card.userId)}>
            <Icon name="ban" size={16} color="#f23f43" /> Забанить
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 80;
  }
  .profile-card {
    position: fixed;
    top: 56px;
    right: 256px;
    width: 320px;
    background: #232428;
    border-radius: var(--radius-md);
    z-index: 81;
    box-shadow: 0 8px 16px rgba(0, 0, 0, 0.4);
    overflow: hidden;
    animation: popIn 0.15s ease;
  }
  .banner { height: 60px; }
  .avatar-row {
    margin-top: -40px;
    padding-left: 16px;
  }
  .body { padding: 8px 16px 16px; }
  .name { font-size: 20px; font-weight: 700; color: var(--header-primary); }
  .tag {
    color: var(--text-muted);
    font-size: 13px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 2px;
  }
  .tag:hover { color: var(--text-normal); }
  .timeout {
    margin-top: 8px;
    background: #f0b23222;
    border: 1px solid #f0b23255;
    color: #f0b232;
    border-radius: var(--radius-sm);
    padding: 6px 10px;
    font-size: 12px;
  }
  .section-title {
    margin-top: 14px;
    margin-bottom: 6px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: #2b2d31;
    border: 1px solid #1e1f22;
    border-radius: var(--radius-sm);
    padding: 3px 6px;
    font-size: 12px;
    color: var(--text-normal);
  }
  .chip-dot { width: 10px; height: 10px; border-radius: 50%; }
  .perms {
    list-style: none;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 3px 10px;
    font-size: 12px;
    color: var(--text-normal);
  }
  .perms li::before { content: '✓ '; color: var(--green); }
  .mono { font-family: var(--font-code); font-size: 12px; color: var(--text-muted); word-break: break-all; }
  .mod-actions { display: flex; gap: 8px; margin-top: 16px; }
  .mod-btn {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    background: #2b2d31;
    border: 1px solid #1e1f22;
    border-radius: var(--radius-sm);
    padding: 7px 0;
    font-size: 13px;
    color: var(--red);
    font-weight: 600;
  }
  .mod-btn:hover { background: #f23f4322; }
</style>
