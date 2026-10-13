<script lang="ts">
  import { app } from '../stores/app.svelte';
  import MessageList from './MessageList.svelte';
  import MessageInput from './MessageInput.svelte';
  import Icon from './Icon.svelte';
  import { menu } from '../stores/menu.svelte';
  import type { Channel } from '../types';

  const channel = $derived(app.activeChannel);
  const isDm = $derived(channel?.type === 1 || channel?.type === 3);
  const guild = $derived(app.activeGuild);

  function headerContext(e: MouseEvent) {
    e.preventDefault();
    if (!channel) return;
    menu.show(e.clientX, e.clientY, [
      { label: 'Копировать ID канала', action: () => void navigator.clipboard?.writeText(channel.id) },
      { label: 'Копировать название', action: () => void navigator.clipboard?.writeText(channel.name ?? '') },
    ]);
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
      <button class="hbtn" title="Закреплённые сообщения (в разработке)" disabled>
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
        <input placeholder="Поиск" disabled />
        <Icon name="search" size={16} color="#b5bac1" />
      </div>
    </div>
  </header>

  <MessageList />
  <MessageInput />
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
  .hbtn:disabled { opacity: 0.4; cursor: not-allowed; }
  .hbtn.off { color: var(--text-muted); }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--bg-base);
    border-radius: var(--radius-sm);
    padding: 0 8px;
    height: 26px;
    width: 144px;
    opacity: 0.7;
  }
  .search input { width: 100%; font-size: 13px; }
</style>
