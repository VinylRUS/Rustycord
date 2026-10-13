<script lang="ts">
  import { app } from '../stores/app.svelte';
  import { displayName } from '../avatars';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import type { Channel, Relationship } from '../types';

  type Tab = 'online' | 'all' | 'pending' | 'blocked' | 'add';

  let tab = $state<Tab>('online');
  let query = $state('');
  let addHandle = $state('');
  let addBusy = $state(false);
  let addResult = $state<{ ok: boolean; text: string } | null>(null);
  let confirmingRemove = $state<string | null>(null);

  const onlineFriends = $derived(
    app.friendsList.filter((r) => app.friendStatus(r.user.id) !== 'offline'),
  );
  const pendingCount = $derived(app.incomingReqs.length + app.outgoingReqs.length);

  /** Строки активного таба (кроме «Добавить»). */
  const rows = $derived.by(() => {
    switch (tab) {
      case 'online':
        return onlineFriends;
      case 'all':
        return app.friendsList;
      case 'pending':
        return [...app.incomingReqs, ...app.outgoingReqs];
      case 'blocked':
        return app.blockedList;
      default:
        return [];
    }
  });

  const listTitle = $derived.by(() => {
    switch (tab) {
      case 'online':
        return 'В сети';
      case 'all':
        return 'Все друзья';
      case 'pending':
        return 'Ожидающие';
      case 'blocked':
        return 'Заблокированные';
      default:
        return '';
    }
  });

  /** Поиск сверху: друзья + существующие ЛС, кому можно написать. */
  const searchHits = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return { friends: [] as Relationship[], dms: [] as Channel[] };
    const friends = app.friendsList.filter((r) => {
      const hay = `${displayName(r.user)} ${r.user.username}`.toLowerCase();
      return hay.includes(q);
    });
    const dms = app.dmRows.filter((c) =>
      (c.recipients ?? []).some((r) => `${displayName(r)} ${r.username}`.toLowerCase().includes(q)),
    );
    return { friends, dms };
  });

  function statusText(status: string): string {
    switch (status) {
      case 'online':
        return 'В сети';
      case 'idle':
        return 'Отошёл';
      case 'dnd':
        return 'Не беспокоить';
      default:
        return 'Не в сети';
    }
  }

  /** Подзаголовок строки друга: кастомный статус → игра → статус. */
  function subtitleOf(r: Relationship): string {
    const pres = app.friendPresences[r.user.id];
    const custom = pres?.activities?.find((a) => a.type === 4 && a.state)?.state;
    if (custom) return custom;
    const game = pres?.activities?.find((a) => a.type !== 4)?.name;
    if (game) return `Играет в ${game}`;
    return statusText(app.friendStatus(r.user.id));
  }

  function dmName(c: Channel): string {
    return c.recipients?.map((r) => displayName(r)).join(', ') || 'Личные сообщения';
  }

  async function submitAdd() {
    const handle = addHandle.trim();
    if (!handle || addBusy) return;
    addBusy = true;
    addResult = null;
    const ok = await app.sendFriendRequest(handle);
    addBusy = false;
    if (ok) {
      addResult = { ok: true, text: `Заявка для @${handle.replace(/^@/, '').split('#')[0]} отправлена — ждём ответа` };
      addHandle = '';
    } else {
      addResult = { ok: false, text: 'Не получилось — проверь имя и попробуй ещё раз' };
    }
  }

  function startDm(userId: string) {
    void app.openDmWith(userId);
  }

  function removeFriend(r: Relationship) {
    if (confirmingRemove !== r.user.id) {
      confirmingRemove = r.user.id;
      return;
    }
    confirmingRemove = null;
    void app.removeRelationship(r.user.id, `${displayName(r.user)} удалён из друзей`);
  }
</script>

<div class="dm-home">
  <!-- Шапка: поиск с выпадашкой «начать беседу» как в оригинале -->
  <div class="hero">
    <div class="search-wrap">
      <div class="search-pill">
        <Icon name="search" size={18} color="#949ba4" />
        <input placeholder="Найти или начать беседу" bind:value={query} />
      </div>
      {#if query.trim()}
        <div class="search-drop">
          {#each searchHits.dms as c (c.id)}
            <button class="hit" onclick={() => app.selectChannel(c.id)}>
              <Avatar user={c.recipients?.[0]} size={24} ringColor="var(--bg-secondary)" />
              <span class="hit-name">{dmName(c)}</span>
              <span class="hit-kind">беседа</span>
            </button>
          {/each}
          {#each searchHits.friends as r (r.user.id)}
            <button class="hit" onclick={() => startDm(r.user.id)}>
              <Avatar user={r.user} size={24} status={app.friendStatus(r.user.id)} ringColor="var(--bg-secondary)" />
              <span class="hit-name">{displayName(r.user)}</span>
              <span class="hit-kind">написать</span>
            </button>
          {/each}
          {#if !searchHits.dms.length && !searchHits.friends.length}
            <div class="hit empty">Никого не нашли — проверь имя или отправь заявку</div>
          {/if}
        </div>
      {/if}
    </div>
  </div>

  <div class="friends">
    {#if app.isBotAccount}
      <!-- Бот-аккаунт: друзей не бывает, показываем честную заметку -->
      <div class="f-head">
        <Icon name="members" size={22} color="#23a55a" />
        <h2>Друзья</h2>
        <span class="badge">{app.dms.length} бесед</span>
      </div>
      <div class="bot-note">
        <div class="bot-note-icon">🤖</div>
        <h3>Это бот-аккаунт</h3>
        <p>
          Боты не заводят друзей и не пишут первыми в ЛС. Когда пользователь напишет вашему боту,
          беседа появится здесь (событие <code>CHANNEL_CREATE</code>, интент <code>DIRECT_MESSAGES</code>).
          Для полного пользовательского опыта войдите своим аккаунтом (QR или email).
        </p>
      </div>
    {:else}
      <div class="tabs">
        <button class="tab" class:active={tab === 'online'} onclick={() => (tab = 'online')}>
          <Icon name="members" size={18} color="currentColor" />
          Онлайн
          {#if onlineFriends.length}<span class="count">{onlineFriends.length}</span>{/if}
        </button>
        <button class="tab" class:active={tab === 'all'} onclick={() => (tab = 'all')}>
          Все
          {#if app.friendsList.length}<span class="count">{app.friendsList.length}</span>{/if}
        </button>
        <button class="tab" class:active={tab === 'pending'} onclick={() => (tab = 'pending')}>
          Ожидание
          {#if pendingCount}<span class="count red">{pendingCount}</span>{/if}
        </button>
        <button class="tab" class:active={tab === 'blocked'} onclick={() => (tab = 'blocked')}>
          Заблокированные
        </button>
        <button class="tab add" class:active={tab === 'add'} onclick={() => (tab = 'add')}>
          <Icon name="plus" size={16} color="currentColor" />
          Добавить в друзья
        </button>
      </div>

      {#if tab === 'add'}
        <div class="add-pane">
          <h3>Добавить в друзья</h3>
          <p class="hint">Ты можешь добавить друга по его имени пользователя Discord. Регистр важен!</p>
          <form
            class="add-form"
            onsubmit={(e) => {
              e.preventDefault();
              void submitAdd();
            }}
          >
            <input
              placeholder="Введи имя пользователя, например peregovich"
              bind:value={addHandle}
              spellcheck="false"
              autocomplete="off"
            />
            <button type="submit" class="btn-blurple" disabled={!addHandle.trim() || addBusy}>
              {addBusy ? 'Отправляем…' : 'Отправить заявку'}
            </button>
          </form>
          {#if addResult}
            <div class="add-result" class:ok={addResult.ok} class:err={!addResult.ok}>
              {addResult.ok ? '✅ ' : '⚠️ '}{addResult.text}
            </div>
          {/if}
        </div>
      {:else}
        <div class="list-head">
          {listTitle} — {rows.length}
        </div>
        <div class="list">
          {#each rows as r (r.user.id)}
            {@const incoming = r.type === 3}
            {@const outgoing = r.type === 4}
            {@const blocked = r.type === 2}
            <div class="row">
              <Avatar
                user={r.user}
                size={36}
                status={blocked ? 'offline' : app.friendStatus(r.user.id)}
                ringColor="var(--bg-primary)"
              />
              <div class="meta">
                <span class="name">{displayName(r.user)}</span>
                <span class="sub" class:dim={blocked || outgoing}>
                  {#if blocked}
                    заблокирован
                  {:else if outgoing}
                    ожидает подтверждения
                  {:else if incoming}
                    входящая заявка в друзья
                  {:else}
                    {subtitleOf(r)}
                  {/if}
                </span>
              </div>
              <div class="actions">
                {#if blocked}
                  <button
                    class="act"
                    title="Разблокировать"
                    onclick={() => void app.removeRelationship(r.user.id, `${displayName(r.user)} разблокирован`)}
                  >
                    <Icon name="ban" size={18} color="#b5bac1" />
                  </button>
                {:else if incoming}
                  <button class="act" title="Принять" onclick={() => void app.acceptFriend(r.user.id)}>
                    <Icon name="check" size={18} color="#b5bac1" />
                  </button>
                  <button
                    class="act"
                    title="Отклонить"
                    onclick={() => void app.removeRelationship(r.user.id, 'Заявка отклонена')}
                  >
                    <Icon name="close" size={18} color="#b5bac1" />
                  </button>
                {:else if outgoing}
                  <button
                    class="act"
                    title="Отменить заявку"
                    onclick={() => void app.removeRelationship(r.user.id, 'Заявка отменена')}
                  >
                    <Icon name="close" size={18} color="#b5bac1" />
                  </button>
                {:else}
                  <button class="act" title="Написать" onclick={() => startDm(r.user.id)}>
                    <Icon name="chat" size={18} color="#b5bac1" />
                  </button>
                  {#if confirmingRemove === r.user.id}
                    <button class="act confirm" title="Точно удалить" onclick={() => removeFriend(r)}>
                      <span>Удалить?</span>
                    </button>
                  {:else}
                    <button
                      class="act"
                      title="Удалить из друзей"
                      onclick={() => (confirmingRemove = r.user.id)}
                    >
                      <Icon name="close" size={18} color="#b5bac1" />
                    </button>
                  {/if}
                {/if}
              </div>
            </div>
          {:else}
            <div class="empty-list">
              {#if tab === 'online'}
                <p>Сейчас никого нет в сети.</p>
              {:else if tab === 'all'}
                <p>Здесь пока пусто. Добавь друзей по имени пользователя!</p>
                <button class="btn-blurple small" onclick={() => (tab = 'add')}>
                  <Icon name="plus" size={14} color="#fff" />
                  Добавить в друзья
                </button>
              {:else if tab === 'pending'}
                <p>Нет заявок — всё чисто. 🎉</p>
              {:else}
                <p>Никто не заблокирован.</p>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    {/if}

    <!-- Недавние беседы: юзер пишет первым, когда захочет -->
    <div class="section-title">Недавние беседы</div>
    <div class="dm-rows">
      {#each app.dmRows as c (c.id)}
        <button class="dm-row" onclick={() => app.selectChannel(c.id)}>
          <Avatar
            user={c.recipients?.[0]}
            size={36}
            status={app.statusOf('', c.recipients?.[0]?.id ?? '')}
            ringColor="var(--bg-primary)"
          />
          <span class="dm-name">{dmName(c)}</span>
          <Icon name="chevron-right" size={16} color="#949ba4" />
        </button>
      {:else}
        <div class="muted empty">
          {#if app.isBotAccount}
            Пока никто не написал — пользователь напишет боту первым, и канал появится.
          {:else}
            Пока нет бесед. Нажми 💬 рядом с другом выше — и переписка начнётся прямо отсюда.
          {/if}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .dm-home {
    flex: 1;
    background: var(--bg-primary);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .hero {
    width: 100%;
    display: flex;
    justify-content: center;
    padding: 24px 16px 0;
    position: relative;
    z-index: 5;
  }
  .search-wrap {
    width: min(560px, 100%);
    position: relative;
  }
  .search-pill {
    background: var(--bg-base);
    border-radius: 8px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    height: 40px;
  }
  .search-pill input {
    flex: 1;
    font-size: 15px;
  }
  .search-drop {
    position: absolute;
    top: 46px;
    left: 0;
    right: 0;
    background: var(--bg-secondary);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .hit {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    color: var(--text-normal);
    font-size: 14px;
    font-weight: 600;
    text-align: left;
  }
  .hit:hover { background: var(--bg-modifier-hover); }
  .hit.empty { color: var(--text-muted); font-weight: 400; cursor: default; }
  .hit.empty:hover { background: transparent; }
  .hit-name { flex: 1; }
  .hit-kind {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .friends {
    width: min(820px, 100%);
    padding: 16px;
  }
  .f-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 0 16px;
  }
  h2 { color: var(--header-primary); font-size: 19px; flex: 1; }
  .badge {
    background: var(--green);
    color: #fff;
    font-size: 12px;
    font-weight: 700;
    border-radius: 20px;
    padding: 2px 10px;
  }

  .bot-note {
    background: var(--bg-secondary);
    border-radius: var(--radius-md);
    padding: 24px;
    text-align: center;
  }
  .bot-note-icon { font-size: 36px; margin-bottom: 8px; }
  .bot-note h3 { color: var(--header-primary); font-size: 18px; margin-bottom: 8px; }
  .bot-note p {
    color: var(--text-muted);
    font-size: 14px;
    line-height: 1.6;
    max-width: 560px;
    margin: 0 auto;
  }
  .bot-note code {
    background: #1e1f22;
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 12px;
    color: var(--mention-text);
  }

  /* ---- табы как в официальном клиенте ---- */
  .tabs {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 0;
    border-bottom: 1px solid #3f414755;
    flex-wrap: wrap;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-size: 15px;
    font-weight: 600;
    background: var(--bg-secondary);
  }
  .tab:hover { color: var(--text-normal); background: var(--bg-modifier-hover); }
  .tab.active {
    color: #fff;
    background: #5865f2;
  }
  .tab.add { color: var(--green); }
  .tab.add.active { color: #fff; background: var(--green); }
  .count {
    background: var(--bg-base);
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 700;
    border-radius: 8px;
    padding: 0 6px;
  }
  .tab.active .count { background: #ffffff33; color: #fff; }
  .count.red { color: #f23f43; }
  .tab.active .count.red { color: #fff; }

  /* ---- добавление в друзья ---- */
  .add-pane { padding: 24px 0 8px; }
  .add-pane h3 { color: var(--header-primary); font-size: 17px; font-weight: 700; }
  .add-pane .hint { color: var(--text-muted); font-size: 13px; margin: 6px 0 14px; }
  .add-form { display: flex; gap: 8px; }
  .add-form input {
    flex: 1;
    background: var(--bg-base);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    font-size: 15px;
    color: var(--text-normal);
  }
  .add-form input:focus { border-color: #5865f2; }
  .add-form button { padding: 10px 16px; font-size: 14px; white-space: nowrap; }
  .add-result { margin-top: 12px; font-size: 14px; font-weight: 600; }
  .add-result.ok { color: var(--green); }
  .add-result.err { color: #f23f43; }

  /* ---- список ---- */
  .list-head {
    margin: 16px 0 4px;
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .list { display: flex; flex-direction: column; }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 8px;
    border-bottom: 1px solid #3f414733;
    border-radius: var(--radius-sm);
  }
  .row:hover { background: var(--bg-modifier-hover); }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .name { color: var(--header-primary); font-size: 15px; font-weight: 600; }
  .sub { color: var(--text-muted); font-size: 12px; margin-top: 1px; }
  .sub.dim { opacity: 0.8; font-style: italic; }
  .actions { display: flex; gap: 4px; opacity: 0; transition: opacity 0.12s ease; }
  .row:hover .actions { opacity: 1; }
  .act {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--bg-secondary);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .act:hover { background: var(--bg-modifier-active); }
  .act:hover :global(svg) { color: #f2f3f5; }
  .act.confirm {
    width: auto;
    padding: 0 12px;
    background: #f23f43;
    color: #fff;
    font-size: 13px;
    font-weight: 700;
    border-radius: 18px;
  }
  .act.confirm:hover { background: #d83c3e; }

  .empty-list {
    padding: 32px 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 14px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }

  .section-title {
    margin: 24px 0 8px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .dm-rows { display: flex; flex-direction: column; }
  .dm-row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    border-bottom: 1px solid #3f414722;
  }
  .dm-row:hover { background: var(--bg-modifier-hover); }
  .dm-name { flex: 1; text-align: left; font-size: 15px; font-weight: 600; color: var(--text-normal); }
  .empty { padding: 16px; font-size: 14px; }

  .btn-blurple {
    background: #5865f2;
    color: #fff;
    border-radius: var(--radius-sm);
    font-weight: 600;
  }
  .btn-blurple:hover:not(:disabled) { background: #4752c4; }
  .btn-blurple:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-blurple.small {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    font-size: 13px;
  }
</style>
