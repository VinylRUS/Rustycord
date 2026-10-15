<script lang="ts">
  import { app } from '../stores/app.svelte';
  import MessageList from './MessageList.svelte';
  import MessageInput from './MessageInput.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import { menu } from '../stores/menu.svelte';
  import { formatStamp } from '../time';
  import { renderMarkdown } from '../markdown';
  import type { Message } from '../types';

  const channel = $derived(app.activeChannel);
  const isDm = $derived(channel?.type === 1 || channel?.type === 3);
  const guild = $derived(app.activeGuild);

  /** Идёт наш звонок в этой беседе (ЛС/группа)? */
  const inDmCall = $derived(
    !!channel && isDm && app.myVoice?.channelId === channel.id && app.myVoice.guildId === '',
  );
  /** Кто-то уже в звонке этой беседы (приглашение присоединиться). */
  const dmCallOthers = $derived(
    !!channel && isDm && !inDmCall && app.dmCallUsers(channel.id).some((v) => v.user_id !== app.user?.id),
  );

  let pins = $state<Message[]>([]);
  let pinsBusy = $state(false);
  let searchQuery = $state('');
  let searchBusy = $state(false);
  let results = $state<Message[]>([]);
  let searched = $state(false);

  const sidePanel = $derived<'none' | 'pins' | 'search'>(
    app.showPins ? 'pins' : app.showSearch ? 'search' : 'none',
  );

  async function togglePins() {
    app.showSearch = false;
    app.showPins = !app.showPins;
    if (app.showPins && channel) {
      pinsBusy = true;
      try {
        pins = await app.listPins(channel.id);
      } catch (e) {
        app.toast(`Закреплённые не загрузились: ${e instanceof Error ? e.message : String(e)}`, 'error');
      } finally {
        pinsBusy = false;
      }
    }
  }

  function openSearch() {
    if (!channel) return;
    app.showPins = false;
    app.showSearch = true;
  }

  async function runSearch() {
    const q = searchQuery.trim();
    if (!q || !channel) return;
    searchBusy = true;
    searched = true;
    try {
      results = await app.searchChannel(channel.id, q);
    } catch (e) {
      app.toast(`Поиск не удался: ${e instanceof Error ? e.message : String(e)}`, 'error');
      results = [];
    } finally {
      searchBusy = false;
    }
  }

  /** Прыжок к найденному сообщению, если оно уже в кэше канала. */
  function jumpTo(m: Message) {
    const list = app.messagesByChannel[m.channel_id];
    if (list?.some((x) => x.id === m.id)) {
      // Сначала просим MessageList расширить окно рендера до сообщения
      // (вне окна элемента с data-mid просто нет в DOM), затем скроллим.
      app.requestReveal(m.id);
    } else {
      app.toast('Сообщение вне загруженной истории — прокрути вверх', 'info');
    }
  }

  function snippet(m: Message): string {
    const text = m.content || (m.attachments.length ? 'вложение' : '…');
    return text.length > 90 ? `${text.slice(0, 90)}…` : text;
  }

  function headerContext(e: MouseEvent) {
    e.preventDefault();
    if (!channel) return;
    menu.show(e.clientX, e.clientY, [
      { label: 'Копировать ID канала', action: () => void navigator.clipboard?.writeText(channel.id) },
      { label: 'Копировать название', action: () => void navigator.clipboard?.writeText(channel.name ?? '') },
    ]);
  }

  /** Позвонить в этой беседе / положить трубку. */
  function toggleDmCall() {
    if (!channel) return;
    void app.toggleVoice(null, channel.id);
  }
</script>

<section class="chat">
<!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="chat-header" oncontextmenu={headerContext}>
    <div class="left">
      {#if isDm}
        <span class="at">@</span>
      {:else}
        <Icon name="hash" size={24} color="#80848e" />
      {/if}
      <h1 class="title">{app.channelTitle}</h1>
      {#if channel?.topic}
        <span class="sep"></span>
        <span class="topic truncate">{channel.topic}</span>
      {/if}
    </div>
    <div class="right">
      {#if isDm}
        <button
          class="hbtn call"
          class:active={inDmCall}
          title={inDmCall ? 'Положить трубку' : 'Позвонить'}
          onclick={toggleDmCall}
        >
          {#if inDmCall}
            <Icon name="phone-off" size={22} color="#f23f43" />
          {:else}
            <Icon name="phone" size={20} color="#b5bac1" />
          {/if}
        </button>
      {/if}
      <button class="hbtn" class:active={app.showPins} title="Закреплённые сообщения" onclick={togglePins}>
        <Icon name="pin" size={22} color="#b5bac1" />
      </button>
      <button
        class="hbtn"
        class:off={!app.showMembers}
        title="Показать/скрыть участников"
        onclick={() => (app.showMembers = !app.showMembers)}
      >
        <Icon name="members" size={22} color="#b5bac1" />
      </button>
      <div class="search">
        <input
          placeholder="Поиск"
          value={searchQuery}
          onkeydown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              openSearch();
              void runSearch();
            }
          }}
          oninput={(e) => (searchQuery = (e.target as HTMLInputElement).value)}
        />
        <button class="search-go" title="Искать в канале" onclick={() => { openSearch(); void runSearch(); }}>
          <Icon name="search" size={16} color="#b5bac1" />
        </button>
      </div>
    </div>
  </header>

  {#if inDmCall && channel}
    <!-- Активный звонок в этой беседе: участники + управление -->
    <div class="call-bar">
      <div class="call-peers">
        {#each app.dmCallUsers(channel.id) as vs (vs.user_id)}
          {@const u = app.userById(vs.user_id)}
          <div class="peer" class:speaking={!!app.speaking[vs.user_id]}>
            <Avatar user={u} size={36} speaking={!!app.speaking[vs.user_id]} ringColor="#111214" />
            <span class="peer-name truncate">
              {u?.global_name || u?.username || 'Участник'}
              {#if vs.user_id === app.user?.id}(вы){/if}
            </span>
            {#if vs.self_mute}<Icon name="mic-off" size={12} color="#f23f43" />{/if}
          </div>
        {/each}
        {#if !app.dmCallUsers(channel.id).length}
          <div class="peer">
            <Avatar user={app.user} size={36} ringColor="#111214" />
            <span class="peer-name">Соединяемся…</span>
          </div>
        {/if}
      </div>
      <div class="call-actions">
        <span class="call-timer muted">{app.myVoice?.mode ? 'на связи' : 'подключаемся…'}</span>
        <button
          class="cbtn"
          class:off={app.myVoice?.selfMute}
          title={app.myVoice?.selfMute ? 'Включить микрофон' : 'Выключить микрофон'}
          onclick={() => void app.setSelfMute(!app.myVoice?.selfMute)}
        >
          <Icon name={app.myVoice?.selfMute ? 'mic-off' : 'mic'} size={18} color={app.myVoice?.selfMute ? '#f23f43' : '#b5bac1'} />
        </button>
        <button
          class="cbtn"
          class:off={app.myVoice?.selfDeaf}
          title={app.myVoice?.selfDeaf ? 'Включить звук' : 'Выключить звук'}
          onclick={() => void app.setSelfDeaf(!app.myVoice?.selfDeaf)}
        >
          <Icon name={app.myVoice?.selfDeaf ? 'headphones-off' : 'headphones'} size={18} color={app.myVoice?.selfDeaf ? '#f23f43' : '#b5bac1'} />
        </button>
        <button class="cbtn hangup" title="Завершить звонок" onclick={toggleDmCall}>
          <Icon name="phone-off" size={18} color="#fff" />
        </button>
      </div>
    </div>
  {:else if dmCallOthers && channel}
    <!-- Кто-то уже в звонке: приглашение как в оригинале -->
    <div class="call-bar incoming">
      <Icon name="phone" size={16} color="#23a55a" />
      <span class="call-line">Идёт звонок — {app.dmCallUsers(channel.id).length} участн.</span>
      <button class="join-call" onclick={toggleDmCall}>Присоединиться</button>
    </div>
  {/if}

  <div class="chat-body">
    <div class="chat-main">
      <MessageList />
      <MessageInput />
    </div>

    {#if sidePanel !== 'none'}
      <aside class="side-panel">
        {#if sidePanel === 'pins'}
          <div class="panel-head">
            <h2>Закреплённые</h2>
            <button class="close-btn" onclick={() => (app.showPins = false)}>✕</button>
          </div>
          <div class="panel-list">
            {#if pinsBusy}
              <div class="panel-empty">Загружаем…</div>
            {:else if !pins.length}
              <div class="panel-empty">В этом канале пока нет закреплённых сообщений.</div>
            {:else}
              {#each pins as m (m.id)}
                <div class="hit">
                  <Avatar user={m.author} size={28} />
                  <div class="hit-body">
                    <div class="hit-head">
                      <b>{m.author.global_name || m.author.username}</b>
                      <span class="muted">{formatStamp(m.id)}</span>
                    </div>
                    <div class="hit-text">{snippet(m)}</div>
                    {#if m.pinned}
                      <button class="unpin" onclick={() => { void app.togglePin(m); pins = pins.filter((x) => x.id !== m.id); }}>Открепить</button>
                    {/if}
                  </div>
                </div>
              {/each}
            {/if}
          </div>
        {:else if sidePanel === 'search'}
          <div class="panel-head">
            <h2>Поиск</h2>
            <button class="close-btn" onclick={() => (app.showSearch = false)}>✕</button>
          </div>
          <div class="panel-list">
            {#if searchBusy}
              <div class="panel-empty">Ищем…</div>
            {:else if !searched}
              <div class="panel-empty">Введи запрос в строке поиска сверху и нажми Enter.</div>
            {:else if !results.length}
              <div class="panel-empty">Ничего не найдено по «{searchQuery}».</div>
            {:else}
              <div class="panel-empty muted">{results.length} результатов</div>
              {#each results as m (m.id)}
                <button class="hit clickable" onclick={() => jumpTo(m)}>
                  <Avatar user={m.author} size={28} />
                  <div class="hit-body">
                    <div class="hit-head">
                      <b>{m.author.global_name || m.author.username}</b>
                      <span class="muted">{formatStamp(m.id)}</span>
                    </div>
                    <div class="hit-text">{snippet(m)}</div>
                  </div>
                </button>
              {/each}
            {/if}
          </div>
        {/if}
      </aside>
    {/if}
  </div>
</section>

<style>
  .chat {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--bg-primary);
  }
  .chat-header {
    height: 48px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 8px 0 16px;
    box-shadow: 0 1px 0 rgba(0, 0, 0, 0.2);
    gap: 8px;
  }
  .left {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .at { color: #80848e; font-weight: 700; font-size: 20px; }
  .title {
    color: var(--header-primary);
    font-size: 16px;
    font-weight: 700;
    white-space: nowrap;
  }
  .sep { width: 1px; height: 20px; background: #3f4147; flex-shrink: 0; }
  .topic { color: var(--text-muted); font-size: 14px; max-width: 400px; }
  .right { display: flex; align-items: center; gap: 4px; }
  .hbtn {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .hbtn:hover:not(:disabled) { background: var(--bg-modifier-hover); }
  .hbtn.active { color: var(--header-primary); background: var(--bg-modifier-hover); }
  .hbtn.off { color: var(--text-muted); }
  .hbtn.call.active { background: #f23f4322; }

  /* ---- звонок в беседе ---- */
  .call-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 12px;
    background: var(--bg-secondary);
    box-shadow: 0 1px 0 rgba(0, 0, 0, 0.2);
    flex-shrink: 0;
  }
  .call-bar.incoming {
    justify-content: flex-start;
  }
  .call-peers {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .call-peers::-webkit-scrollbar { display: none; }
  .peer {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .peer-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--header-primary);
    max-width: 140px;
  }
  .call-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .call-timer { font-size: 12px; margin-right: 4px; }
  .cbtn {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .cbtn:hover { background: var(--bg-modifier-hover); }
  .cbtn.hangup { background: #f23f43; }
  .cbtn.hangup:hover { background: #d8383c; }
  .call-line {
    flex: 1;
    font-size: 13px;
    color: var(--text-normal);
    text-align: left;
  }
  .join-call {
    background: #23a55a;
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    flex-shrink: 0;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 4px;
    background: var(--bg-base);
    border-radius: var(--radius-sm);
    padding: 0 8px;
    height: 26px;
    width: 160px;
  }
  .search input { width: 100%; font-size: 13px; }
  .search-go { display: flex; align-items: center; }

  .chat-body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .chat-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .side-panel {
    width: 340px;
    flex-shrink: 0;
    border-left: 1px solid #1e1f22;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .panel-head {
    height: 48px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px 0 16px;
    box-shadow: 0 1px 0 rgba(0, 0, 0, 0.2);
  }
  .panel-head h2 { font-size: 15px; font-weight: 700; color: var(--header-primary); }
  .close-btn {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
  }
  .close-btn:hover { background: var(--bg-modifier-hover); }
  .panel-list {
    flex: 1;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .panel-empty { color: var(--text-muted); font-size: 13px; padding: 8px 4px; }
  .hit {
    display: flex;
    gap: 10px;
    padding: 8px;
    border-radius: var(--radius-sm);
    background: #2b2d31;
    text-align: left;
    width: 100%;
  }
  .hit.clickable:hover { background: #32353b; }
  .hit-body { flex: 1; min-width: 0; }
  .hit-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 13px;
  }
  .hit-head .muted { font-size: 11px; }
  .hit-text {
    color: var(--text-normal);
    font-size: 13px;
    margin-top: 2px;
    overflow-wrap: anywhere;
  }
  .unpin {
    color: var(--text-link);
    font-size: 12px;
    margin-top: 4px;
  }
  .unpin:hover { text-decoration: underline; }
  .muted { color: var(--text-muted); }
</style>
