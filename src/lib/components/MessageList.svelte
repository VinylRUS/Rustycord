<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { Message } from '../types';
  import { formatDivider, snowflakeToMs } from '../time';
  import MessageItem from './MessageItem.svelte';
  import Icon from './Icon.svelte';

  let listEl = $state<HTMLDivElement | null>(null);

  const messages = $derived(app.activeMessages);
  const channelId = $derived(app.activeChannelId);

  interface Row {
    kind: 'divider';
    label: string;
    key: string;
  }
  interface MsgRow {
    kind: 'message';
    message: Message;
    grouped: boolean;
    key: string;
  }
  type RenderRow = Row | MsgRow;

  /** Discord grouping rules: same author, < 7 min gap, same day. */
  const rows = $derived.by(() => {
    const out: RenderRow[] = [];
    let prev: Message | null = null;
    for (const m of messages) {
      const prevTs = prev ? snowflakeToMs(prev.id) : 0;
      const ts = snowflakeToMs(m.id);
      const newDay = !prev || new Date(prevTs).toDateString() !== new Date(ts).toDateString();
      if (newDay) {
        out.push({ kind: 'divider', label: formatDivider(ts), key: `d-${m.id}` });
      }
      const grouped =
        !!prev &&
        !newDay &&
        prev.author.id === m.author.id &&
        ts - prevTs < 7 * 60_000 &&
        m.edited_timestamp === null;
      out.push({ kind: 'message', message: m, grouped, key: m.id });
      prev = m;
    }
    return out;
  });

  // Scroll management: follow new messages when pinned to bottom.
  let stick = true;
  $effect(() => {
    const len = messages.length;
    if (listEl && stick && len) {
      requestAnimationFrame(() => {
        listEl?.scrollTo({ top: listEl.scrollHeight });
      });
    }
  });
  $effect(() => {
    // channel switch → jump to bottom, enable stickiness
    void channelId;
    stick = true;
    requestAnimationFrame(() => listEl?.scrollTo({ top: listEl.scrollHeight }));
  });

  function onScroll() {
    if (!listEl) return;
    const nearBottom = listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight < 120;
    stick = nearBottom;
    if (listEl.scrollTop < 80 && channelId && app.messagesLoaded[channelId]) {
      void app.loadOlder(channelId);
    }
  }

  const typing = $derived(app.typingNames(channelId ?? ''));
</script>

<div class="messages" bind:this={listEl} onscroll={onScroll}>
  {#if app.loadingOlder[channelId ?? '']}
    <div class="older-loading muted">Загружаем историю…</div>
  {/if}

  {#if messages.length === 0 && app.messagesLoaded[channelId ?? '']}
    {@const ch = app.activeChannel}
    {@const isDm = ch?.type === 1 || ch?.type === 3}
    <div class="intro">
      <div class="intro-icon">
        <Icon name={isDm ? 'chat' : 'hash'} size={44} color="#949ba4" />
      </div>
      <h2>{isDm ? '' : '#'}{app.channelTitle}</h2>
      <p class="muted">
        {#if isDm}
          Это начало вашей переписки с {app.channelTitle} — напиши первым 👋
        {:else}
          Это начало канала #{app.channelTitle}.
        {/if}
      </p>
    </div>
  {/if}

  {#each rows as row (row.key)}
    {#if row.kind === 'divider'}
      <div class="divider">
        <span class="line"></span>
        <span class="label">{row.label}</span>
        <span class="line"></span>
      </div>
    {:else}
      <div data-mid={row.message.id}>
        <MessageItem message={row.message} grouped={row.grouped} />
      </div>
    {/if}
  {/each}
</div>

{#if typing.length}
  <div class="typing muted">
    <span class="dots"><span></span><span></span><span></span></span>
    <b>{typing.slice(0, 3).join(', ')}</b>
    {typing.length > 1 ? ' печатают…' : ' печатает…'}
  </div>
{:else}
  <div class="typing-spacer"></div>
{/if}

<style>
  .messages {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding-bottom: 4px;
  }
  .older-loading {
    text-align: center;
    font-size: 12px;
    padding: 10px;
  }
  .intro {
    padding: 24px 16px 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 8px;
    margin-top: 24px;
  }
  .intro-icon {
    width: 68px;
    height: 68px;
    border-radius: 50%;
    background: #41434a;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  h2 { color: var(--header-primary); font-size: 32px; font-weight: 700; }
  .intro p { font-size: 15px; }

  .divider {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 16px 16px 4px;
  }
  .line { flex: 1; height: 1px; background: #3f4147; }
  .label { font-size: 12px; font-weight: 600; color: var(--text-muted); flex-shrink: 0; }

  .typing {
    height: 24px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    font-size: 13px;
  }
  .typing-spacer { height: 24px; flex-shrink: 0; }
  .dots {
    display: inline-flex;
    gap: 3px;
  }
  .dots span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-muted);
    animation: typingDot 1.4s infinite ease-in-out;
  }
  .dots span:nth-child(2) { animation-delay: 0.2s; }
  .dots span:nth-child(3) { animation-delay: 0.4s; }
</style>
