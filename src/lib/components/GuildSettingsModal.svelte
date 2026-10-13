<script lang="ts">
  /**
   * GuildSettingsModal (v0.3.0) — управление сервером как в оригинале:
   * обзор, каналы (CRUD), роли (права/цвет), участники (роли/ник/кик/бан),
   * бан-лист и инвайты. Все вызовы идут через универсальный REST-прокси
   * Rust-слоя (api_request) с общими rate limits и заголовками сессии.
   */
  import { app } from '../stores/app.svelte';
  import type { BanEntry, Channel, Invite, Member, Role } from '../types';
  import { PERMISSIONS, PERMISSION_LABELS_RU, type PermissionKey } from '../permissions';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  type Tab = 'overview' | 'channels' | 'roles' | 'members' | 'bans' | 'invites';
  let tab = $state<Tab>('overview');

  const guild = $derived(app.guilds.find((g) => g.id === app.showGuildSettings) ?? null);
  const gid = $derived(guild?.id ?? '');

  /* ---------- overview ---------- */
  let nameDraft = $state('');
  let savingName = $state(false);
  $effect(() => {
    nameDraft = guild?.name ?? '';
  });

  async function saveName() {
    if (!guild || !nameDraft.trim() || nameDraft === guild.name) return;
    savingName = true;
    try {
      await app.apiCall('PATCH', `/guilds/${guild.id}`, { name: nameDraft.trim() });
      app.toast('Название сервера обновлено', 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    } finally {
      savingName = false;
    }
  }

  /* ---------- channels ---------- */
  const categories = $derived(
    (guild?.channels ?? []).filter((c) => c.type === 4).sort((a, b) => (a.position ?? 0) - (b.position ?? 0)),
  );
  const plainChannels = $derived(
    (guild?.channels ?? []).filter((c) => c.type !== 4).sort((a, b) => (a.position ?? 0) - (b.position ?? 0)),
  );
  let renameId = $state('');
  let renameValue = $state('');
  let newName = $state('');
  let newType = $state<0 | 2 | 4>(0);
  let newParent = $state('');
  let creatingChannel = $state(false);

  function startRename(c: Channel) {
    renameId = c.id;
    renameValue = c.name ?? '';
  }

  async function commitRename(c: Channel) {
    const name = renameValue.trim().toLowerCase().replace(/\s+/g, '-');
    if (!name || name === c.name) {
      renameId = '';
      return;
    }
    try {
      await app.apiCall('PATCH', `/channels/${c.id}`, { name });
      app.toast(`Канал переименован в #${name}`, 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    } finally {
      renameId = '';
    }
  }

  async function deleteChannel(c: Channel) {
    if (!confirm(`Удалить канал «${c.name}»? Отменить нельзя.`)) return;
    try {
      await app.apiCall('DELETE', `/channels/${c.id}`);
      app.toast('Канал удалён', 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    }
  }

  async function createChannel() {
    if (!guild || !newName.trim()) return;
    creatingChannel = true;
    const name = newName.trim().toLowerCase().replace(/\s+/g, '-');
    try {
      await app.apiCall('POST', `/guilds/${guild.id}/channels`, {
        name,
        type: newType,
        parent_id: newType !== 4 && newParent ? newParent : undefined,
      });
      newName = '';
      app.toast(`Канал #${name} создан`, 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    } finally {
      creatingChannel = false;
    }
  }

  /* ---------- roles ---------- */
  const roles = $derived(
    (guild?.roles ?? []).slice().sort((a, b) => b.position - a.position),
  );
  let selectedRoleId = $state('');
  const selectedRole = $derived(roles.find((r) => r.id === selectedRoleId) ?? null);
  let roleName = $state('');
  let roleColor = $state('#99aab5');
  let roleHoist = $state(false);
  let rolePerms = $state(0n);
  let creatingRole = $state(false);
  let savingRole = $state(false);

  const KEY_PERMS: PermissionKey[] = [
    'ADMINISTRATOR',
    'MANAGE_GUILD',
    'MANAGE_ROLES',
    'MANAGE_CHANNELS',
    'KICK_MEMBERS',
    'BAN_MEMBERS',
    'MANAGE_MESSAGES',
    'MANAGE_NICKNAMES',
    'MANAGE_WEBHOOKS',
    'VIEW_AUDIT_LOG',
  ];
  const ALL_PERMS = Object.keys(PERMISSIONS) as PermissionKey[];
  let showAllPerms = $state(false);

  $effect(() => {
    if (selectedRole) {
      roleName = selectedRole.name;
      roleColor = selectedRole.color ? `#${selectedRole.color.toString(16).padStart(6, '0')}` : '#99aab5';
      roleHoist = selectedRole.hoist;
      rolePerms = BigInt(selectedRole.permissions || '0');
    }
  });

  function togglePerm(key: PermissionKey) {
    if (key === 'ADMINISTRATOR') {
      rolePerms = rolePerms & PERMISSIONS.ADMINISTRATOR ? rolePerms & ~PERMISSIONS.ADMINISTRATOR : rolePerms | PERMISSIONS.ADMINISTRATOR;
    } else {
      rolePerms = rolePerms & PERMISSIONS[key] ? rolePerms & ~PERMISSIONS[key] : rolePerms | PERMISSIONS[key];
    }
  }

  function hasPerm(key: PermissionKey): boolean {
    return (rolePerms & PERMISSIONS[key]) !== 0n;
  }

  async function createRole() {
    if (!guild || !roleName.trim()) return;
    creatingRole = true;
    try {
      const created: Role = await app.apiCall('POST', `/guilds/${guild.id}/roles`, {
        name: roleName.trim(),
        color: parseInt(roleColor.slice(1), 16),
        permissions: rolePerms.toString(),
        hoist: roleHoist,
      });
      selectedRoleId = created.id;
      app.toast(`Роль «${created.name}» создана`, 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    } finally {
      creatingRole = false;
    }
  }

  async function saveRole() {
    if (!guild || !selectedRole || !roleName.trim()) return;
    savingRole = true;
    try {
      await app.apiCall('PATCH', `/guilds/${guild.id}/roles/${selectedRole.id}`, {
        name: roleName.trim(),
        color: parseInt(roleColor.slice(1), 16),
        permissions: rolePerms.toString(),
        hoist: roleHoist,
      });
      app.toast('Роль сохранена', 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    } finally {
      savingRole = false;
    }
  }

  async function deleteRole() {
    if (!guild || !selectedRole) return;
    if (!confirm(`Удалить роль «${selectedRole.name}»?`)) return;
    try {
      await app.apiCall('DELETE', `/guilds/${guild.id}/roles/${selectedRole.id}`);
      selectedRoleId = '';
      app.toast('Роль удалена', 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    }
  }

  /* ---------- members ---------- */
  const members = $derived(Object.values(app.membersByGuild[gid] ?? {}));
  let memberQuery = $state('');
  let editRolesOf = $state('');
  let nickEditId = $state('');
  let nickValue = $state('');

  const filteredMembers = $derived(
    memberQuery
      ? members.filter((m) => {
          const q = memberQuery.toLowerCase();
          const u = m.user;
          return (
            u?.username?.toLowerCase().includes(q) ||
            u?.global_name?.toLowerCase().includes(q) ||
            (m.nick ?? '').toLowerCase().includes(q)
          );
        })
      : members.slice(0, 50),
  );

  async function saveNick(member: Member) {
    if (!guild || !member.user) return;
    try {
      await app.apiCall('PATCH', `/guilds/${guild.id}/members/${member.user.id}`, { nick: nickValue.trim() });
      app.toast('Никнейм обновлён', 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    } finally {
      nickEditId = '';
    }
  }

  async function toggleMemberRole(member: Member, roleId: string) {
    if (!guild || !member.user) return;
    const current = member.roles.includes(roleId)
      ? member.roles.filter((r) => r !== roleId)
      : [...member.roles, roleId];
    try {
      await app.apiCall('PATCH', `/guilds/${guild.id}/members/${member.user.id}`, { roles: current });
      app.toast('Роли участника обновлены', 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    }
  }

  /* ---------- bans ---------- */
  let bans: BanEntry[] = $state([]);
  let bansLoaded = $state(false);
  let bansBusy = $state(false);

  async function loadBans() {
    if (!guild) return;
    bansBusy = true;
    try {
      bans = await app.listBans(guild.id);
      bansLoaded = true;
    } catch (e) {
      app.toast(`Не удалось загрузить баны: ${msg(e)}`, 'error');
    } finally {
      bansBusy = false;
    }
  }

  async function unban(userId: string) {
    if (!guild) return;
    try {
      await app.apiCall('DELETE', `/guilds/${guild.id}/bans/${userId}`);
      bans = bans.filter((b) => b.user.id !== userId);
      app.toast('Пользователь разбанен', 'info');
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    }
  }

  /* ---------- invites ---------- */
  let invites: Invite[] = $state([]);
  let invitesBusy = $state(false);

  async function loadInvites() {
    if (!guild) return;
    invitesBusy = true;
    try {
      invites = await app.listInvites(guild.id);
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    } finally {
      invitesBusy = false;
    }
  }

  async function createInvite() {
    if (!guild) return;
    const channel = (guild.channels ?? []).find((c) => c.type === 0);
    if (!channel) {
      app.toast('На сервере нет текстовых каналов', 'error');
      return;
    }
    try {
      await app.apiCall('POST', `/channels/${channel.id}/invites`, {
        max_age: 86400,
        max_uses: 0,
        temporary: false,
      });
      app.toast(`Инвайт создан (24ч) для #${channel.name}`, 'info');
      await loadInvites();
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    }
  }

  async function deleteInvite(code: string) {
    try {
      await app.apiCall('DELETE', `/invites/${code}`);
      invites = invites.filter((i) => i.code !== code);
    } catch (e) {
      app.toast(`Не удалось: ${msg(e)}`, 'error');
    }
  }

  /* ---------- lazy loaders per tab ---------- */
  $effect(() => {
    if (!guild) return;
    if (tab === 'bans' && !bansLoaded && !bansBusy) void loadBans();
    if (tab === 'invites' && !invites.length && !invitesBusy) void loadInvites();
  });

  function close() {
    app.showGuildSettings = null;
  }

  function msg(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  function channelTypeLabel(t: number): string {
    if (t === 2) return 'войс';
    if (t === 4) return 'категория';
    if (t === 5) return 'анонс';
    if (t === 13) return 'stage';
    return 'текст';
  }
</script>

{#if guild}
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={close} oncontextmenu={(e) => e.preventDefault()}></div>
  <div class="modal">
    <header>
      <h1>{guild.name}</h1>
      <button class="close" onclick={close}>
        <Icon name="close" size={20} color="#b5bac1" />
      </button>
    </header>

    <nav class="tabs">
      {#each [['overview', 'Обзор'], ['channels', 'Каналы'], ['roles', 'Роли'], ['members', 'Участники'], ['bans', 'Баны'], ['invites', 'Инвайты']] as [id, label]}
        <button class:active={tab === id} onclick={() => (tab = id as Tab)}>{label}</button>
      {/each}
    </nav>

    <div class="content scroller">
      <!-- ============ ОБЗОР ============ -->
      {#if tab === 'overview'}
        <section>
          <h2>Название сервера</h2>
          <div class="row">
            <input class="grow" bind:value={nameDraft} maxlength={100} />
            <button class="btn-blurple" onclick={() => void saveName()} disabled={savingName || nameDraft === guild.name}>
              {savingName ? 'Сохраняем…' : 'Сохранить'}
            </button>
          </div>
          <p class="hint">Требуется право MANAGE_GUILD. ID сервера: <code>{guild.id}</code></p>
        </section>
        <section>
          <h2>Статистика</h2>
          <div class="stats">
            <div class="stat"><b>{guild.member_count ?? Object.keys(app.membersByGuild[gid] ?? {}).length}</b><span>участников</span></div>
            <div class="stat"><b>{(guild.channels ?? []).length}</b><span>каналов</span></div>
            <div class="stat"><b>{guild.roles.length}</b><span>ролей</span></div>
          </div>
        </section>
        <section>
          <h2>Владелец</h2>
          <div class="user-line">
            <Avatar user={app.userById(guild.owner_id)} size={32} />
            <span>{app.userById(guild.owner_id)?.global_name ?? app.userById(guild.owner_id)?.username ?? `id:${guild.owner_id.slice(-6)}`}</span>
          </div>
        </section>
      {/if}

      <!-- ============ КАНАЛЫ ============ -->
      {#if tab === 'channels'}
        <section>
          <h2>Создать канал</h2>
          <div class="row wrap">
            <input placeholder="имя-канала" bind:value={newName} />
            <select bind:value={newType}>
              <option value={0}>Текстовый</option>
              <option value={2}>Голосовой</option>
              <option value={4}>Категория</option>
            </select>
            {#if newType !== 4}
              <select bind:value={newParent}>
                <option value="">Без категории</option>
                {#each categories as cat (cat.id)}
                  <option value={cat.id}>{cat.name}</option>
                {/each}
              </select>
            {/if}
            <button class="btn-blurple" onclick={() => void createChannel()} disabled={creatingChannel || !newName.trim()}>
              Создать
            </button>
          </div>
          <p class="hint">Требуется MANAGE_CHANNELS.</p>
        </section>

        <section>
          <h2>Каналы ({plainChannels.length})</h2>
          {#each plainChannels as c (c.id)}
            <div class="list-row">
              <span class="mono muted">{channelTypeLabel(c.type)}</span>
              {#if renameId === c.id}
                <input
                  class="grow"
                  bind:value={renameValue}
                  onkeydown={(e) => { if (e.key === 'Enter') void commitRename(c); if (e.key === 'Escape') renameId = ''; }}
                />
                <button class="icon-btn" title="Сохранить" onclick={() => void commitRename(c)}>
                  <Icon name="check" size={16} color="#57f287" />
                </button>
              {:else}
                <span class="grow">{c.type === 2 || c.type === 13 ? '🔊' : '#'} {c.name}</span>
                <button class="icon-btn" title="Переименовать" onclick={() => startRename(c)}>
                  <Icon name="pencil" size={16} color="#b5bac1" />
                </button>
              {/if}
              <button class="icon-btn danger" title="Удалить" onclick={() => void deleteChannel(c)}>
                <Icon name="trash" size={16} color="#f23f43" />
              </button>
            </div>
          {/each}
        </section>
      {/if}

      <!-- ============ РОЛИ ============ -->
      {#if tab === 'roles'}
        <div class="two-col">
          <div class="col">
            <button class="btn-blurple full" onclick={() => { selectedRoleId = ''; roleName = 'новая роль'; roleColor = '#99aab5'; roleHoist = false; rolePerms = 0n; }}>
              + Создать роль
            </button>
            {#each roles as r (r.id)}
              <button class="role-row" class:active={selectedRoleId === r.id} onclick={() => (selectedRoleId = r.id)}>
                <span class="role-dot" style="background: {r.color ? '#' + r.color.toString(16).padStart(6, '0') : '#99aab5'}"></span>
                <span class="grow left">{r.name}</span>
                {#if r.managed}<span class="muted tag">bot</span>{/if}
              </button>
            {/each}
          </div>
          <div class="col">
            {#if selectedRole}
              <h2>Роль «{selectedRole.name}»</h2>
              <div class="row">
                <input class="grow" bind:value={roleName} maxlength={100} />
                <input type="color" bind:value={roleColor} title="Цвет" />
                <label class="check"><input type="checkbox" bind:checked={roleHoist} /> показывать отдельно</label>
              </div>
              <h3>Права</h3>
              <div class="perm-grid">
                {#each (showAllPerms ? ALL_PERMS : KEY_PERMS) as key}
                  <label class="check">
                    <input type="checkbox" checked={hasPerm(key)} onchange={() => togglePerm(key)} />
                    {PERMISSION_LABELS_RU[key] ?? key}
                  </label>
                {/each}
              </div>
              <button class="btn-ghost" onclick={() => (showAllPerms = !showAllPerms)}>
                {showAllPerms ? '← Только основные' : `Все права (${ALL_PERMS.length}) →`}
              </button>
              <div class="row">
                <button class="btn-blurple" onclick={() => void saveRole()} disabled={savingRole}>
                  {savingRole ? 'Сохраняем…' : 'Сохранить роль'}
                </button>
                {#if selectedRole.id !== guild.id}
                  <button class="btn-danger" onclick={() => void deleteRole()}>Удалить</button>
                {/if}
              </div>
            {:else}
              <h2>{selectedRoleId === '' ? 'Новая роль' : 'Роль'}</h2>
              <div class="row">
                <input class="grow" bind:value={roleName} placeholder="имя роли" />
                <input type="color" bind:value={roleColor} title="Цвет" />
                <label class="check"><input type="checkbox" bind:checked={roleHoist} /> отдельно</label>
              </div>
              <div class="perm-grid">
                {#each KEY_PERMS as key}
                  <label class="check">
                    <input type="checkbox" checked={hasPerm(key)} onchange={() => togglePerm(key)} />
                    {PERMISSION_LABELS_RU[key] ?? key}
                  </label>
                {/each}
              </div>
              <button class="btn-blurple" onclick={() => void createRole()} disabled={creatingRole || !roleName.trim()}>
                {creatingRole ? 'Создаём…' : 'Создать роль'}
              </button>
            {/if}
          </div>
        </div>
      {/if}

      <!-- ============ УЧАСТНИКИ ============ -->
      {#if tab === 'members'}
        <section>
          <input class="search" placeholder="Поиск участника (имя / ник)…" bind:value={memberQuery} />
          <p class="hint">Найдено: {filteredMembers.length} из {members.length} (в кэше). Полный список подтягивается чанками автоматически.</p>
        </section>
        {#each filteredMembers as m (m.user?.id ?? m.nick)}
          {#if m.user}
            <div class="list-row member">
              <Avatar user={m.user} size={28} />
              <span class="grow">{m.nick || m.user.global_name || m.user.username}</span>
              {#if nickEditId === m.user.id}
                <input bind:value={nickValue} placeholder="новый ник" onkeydown={(e) => { if (e.key === 'Enter') void saveNick(m); if (e.key === 'Escape') nickEditId = ''; }} />
                <button class="icon-btn" onclick={() => void saveNick(m)}><Icon name="check" size={16} color="#57f287" /></button>
              {:else}
                <button class="icon-btn" title="Сменить ник" onclick={() => { nickEditId = m.user!.id; nickValue = m.nick ?? ''; }}>
                  <Icon name="pencil" size={16} color="#b5bac1" />
                </button>
              {/if}
              <button class="icon-btn" title="Роли" onclick={() => (editRolesOf = editRolesOf === m.user!.id ? '' : m.user!.id)}>
                <Icon name="shield" size={16} color="#b5bac1" />
              </button>
              <button class="icon-btn danger" title="Кикнуть" onclick={() => void app.kickMember(gid, m.user!.id)}>
                <Icon name="kick" size={16} color="#f23f43" />
              </button>
              <button class="icon-btn danger" title="Забанить" onclick={() => void app.banMember(gid, m.user!.id)}>
                <Icon name="ban" size={16} color="#f23f43" />
              </button>
            </div>
            {#if editRolesOf === m.user.id}
              <div class="role-assign">
                {#each roles as r (r.id)}
                  {#if r.id !== guild.id}
                    <label class="check">
                      <input type="checkbox" checked={m.roles.includes(r.id)} onchange={() => void toggleMemberRole(m, r.id)} />
                      <span style="color: {r.color ? '#' + r.color.toString(16).padStart(6, '0') : 'inherit'}">{r.name}</span>
                    </label>
                  {/if}
                {/each}
              </div>
            {/if}
          {/if}
        {/each}
      {/if}

      <!-- ============ БАНЫ ============ -->
      {#if tab === 'bans'}
        {#if bansBusy}
          <p class="hint">Загружаем…</p>
        {:else if !bans.length}
          <p class="hint">Банов нет (или нет права BAN_MEMBERS для просмотра).</p>
        {:else}
          {#each bans as b (b.user.id)}
            <div class="list-row">
              <Avatar user={b.user} size={28} />
              <span class="grow">{b.user.global_name || b.user.username}</span>
              <span class="muted grow reason">{b.reason ?? 'без причины'}</span>
              <button class="btn-ghost" onclick={() => void unban(b.user.id)}>Разбанить</button>
            </div>
          {/each}
        {/if}
      {/if}

      <!-- ============ ИНВАЙТЫ ============ -->
      {#if tab === 'invites'}
        <section>
          <button class="btn-blurple" onclick={() => void createInvite()}>Создать инвайт (24 часа)</button>
          <p class="hint">Создаётся для первого текстового канала. Требуется CREATE_INSTANT_INVITE.</p>
        </section>
        {#if invitesBusy}
          <p class="hint">Загружаем…</p>
        {:else if !invites.length}
          <p class="hint">Активных инвайтов нет.</p>
        {:else}
          {#each invites as inv (inv.code)}
            <div class="list-row">
              <code class="mono">discord.gg/{inv.code}</code>
              <span class="muted">#{inv.channel?.name ?? '?'}</span>
              <span class="muted">{inv.uses}{inv.max_uses ? `/${inv.max_uses}` : ''} использований</span>
              <span class="grow"></span>
              <button class="icon-btn danger" title="Отозвать" onclick={() => void deleteInvite(inv.code)}>
                <Icon name="trash" size={16} color="#f23f43" />
              </button>
            </div>
          {/each}
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    z-index: 60;
  }
  .modal {
    position: fixed;
    inset: 48px 8vw;
    background: var(--bg-primary, #313338);
    border-radius: var(--radius-md, 8px);
    display: flex;
    flex-direction: column;
    z-index: 61;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    border-bottom: 1px solid #1e1f22;
    flex-shrink: 0;
  }
  h1 {
    font-size: 19px;
    color: var(--header-primary, #f2f3f5);
  }
  h2 { font-size: 15px; color: var(--header-primary, #f2f3f5); margin-bottom: 10px; }
  h3 { font-size: 13px; color: var(--text-muted, #949ba4); text-transform: uppercase; margin: 14px 0 8px; }
  .close {
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 4px;
  }
  .tabs {
    display: flex;
    gap: 4px;
    padding: 10px 18px 0;
    border-bottom: 1px solid #1e1f22;
    flex-shrink: 0;
    flex-wrap: wrap;
  }
  .tabs button {
    background: transparent;
    border: none;
    color: var(--text-muted, #949ba4);
    font-size: 14px;
    font-weight: 600;
    padding: 8px 12px;
    border-bottom: 2px solid transparent;
  }
  .tabs button.active {
    color: var(--text-normal, #dbdee1);
    border-bottom-color: #5865f2;
  }
  .content {
    flex: 1;
    overflow-y: auto;
    padding: 18px;
  }
  section { margin-bottom: 22px; }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .row .grow { flex: 1; min-width: 160px; }
  input, select {
    background: #1e1f22;
    border: none;
    border-radius: 4px;
    padding: 8px 10px;
    color: var(--text-normal, #dbdee1);
    font-size: 14px;
  }
  input[type='color'] {
    padding: 2px;
    width: 42px;
    height: 34px;
  }
  input.search { width: 100%; margin-bottom: 8px; }
  .btn-blurple {
    background: #5865f2;
    color: #fff;
    border: none;
    border-radius: 4px;
    padding: 8px 16px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-blurple:disabled { opacity: 0.5; cursor: default; }
  .btn-blurple.full { width: 100%; margin-bottom: 10px; }
  .btn-danger {
    background: #da373c;
    color: #fff;
    border: none;
    border-radius: 4px;
    padding: 8px 16px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .btn-ghost {
    background: transparent;
    border: none;
    color: var(--text-link, #00a8fc);
    cursor: pointer;
    font-size: 13px;
    padding: 4px;
  }
  .hint {
    color: var(--text-muted, #949ba4);
    font-size: 12px;
    margin-top: 6px;
    line-height: 1.5;
  }
  .hint code { background: #1e1f22; padding: 1px 5px; border-radius: 3px; }
  .mono { font-family: var(--font-code, monospace); font-size: 13px; }
  .muted { color: var(--text-muted, #949ba4); font-size: 13px; }

  .stats { display: flex; gap: 12px; }
  .stat {
    background: #2b2d31;
    border-radius: 8px;
    padding: 14px 18px;
    text-align: center;
    min-width: 96px;
  }
  .stat b { display: block; font-size: 22px; color: var(--header-primary, #f2f3f5); }
  .stat span { color: var(--text-muted, #949ba4); font-size: 12px; }

  .user-line { display: flex; align-items: center; gap: 10px; color: var(--text-normal, #dbdee1); }

  .list-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 6px;
    background: #2b2d31;
    margin-bottom: 6px;
  }
  .list-row .grow { flex: 1; }
  .list-row .reason { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .icon-btn {
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 4px;
    display: flex;
  }
  .list-row input { padding: 4px 8px; }

  .two-col { display: flex; gap: 18px; min-height: 0; }
  .col { flex: 1; min-width: 0; }
  .role-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text-normal, #dbdee1);
    padding: 8px 10px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 14px;
  }
  .role-row:hover { background: #2b2d31; }
  .role-row.active { background: #404249; }
  .role-row .left { text-align: left; }
  .role-dot { width: 12px; height: 12px; border-radius: 50%; flex-shrink: 0; }
  .tag {
    font-size: 10px;
    border: 1px solid #4e5058;
    border-radius: 4px;
    padding: 0 4px;
  }
  .perm-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: 6px 12px;
    margin-bottom: 10px;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--text-normal, #dbdee1);
    font-size: 13px;
    cursor: pointer;
  }
  .role-assign {
    background: #232428;
    border-radius: 6px;
    padding: 10px 12px;
    margin: -2px 0 8px 16px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 6px 10px;
  }
  .member input { padding: 4px 8px; width: 140px; }

  @media (max-width: 720px) {
    .modal { inset: 8px; }
    .two-col { flex-direction: column; }
  }
</style>
