<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { Message } from '../types';
  import { renderMarkdown } from '../markdown';
  import { formatStamp, formatTime, snowflakeDate } from '../time';
  import { emojiUrl } from '../avatars';
  import Avatar from './Avatar.svelte';
  import EmojiPicker from './EmojiPicker.svelte';
  import Icon from './Icon.svelte';
  import { menu, type MenuItem } from '../stores/menu.svelte';

  let {
    message,
    grouped,
  }: {
    message: Message;
    grouped: boolean; // true → compact continuation (no avatar/header)
  } = $props();

  let showPicker = $state(false);

  const guildId = $derived(app.activeGuild?.id ?? '');
  const own = $derived(message.author.id === app.user?.id);
  const color = $derived(app.memberColor(guildId, message.author.id));
  const name = $derived(app.memberById(guildId, message.author.id)?.nick || message.author.global_name || message.author.username);

  const html = $derived(
    renderMarkdown(message.content, {
      user: (id) => app.memberName(guildId, id),
      channel: (id) => {
        const c = app.channelById(id);
        return c ? `#${c.name ?? 'канал'}` : '#неизвестный-канал';
      },
      isMentioningMe: (id) => id === app.user?.id,
    }),
  );

  function onContext(e: MouseEvent) {
    e.preventDefault();
    const items: MenuItem[] = [
      { label: 'Копировать текст', action: () => void navigator.clipboard?.writeText(message.content) },
      { label: 'Копировать ID сообщения', action: () => void navigator.clipboard?.writeText(message.id) },
    ];
    if (own) {
      items.push({ label: 'Изменить', action: () => app.startEdit(message) });
    }
    if (own) {
      items.push({ label: 'Удалить', danger: true, action: () => void app.deleteMessage(message) });
    }
    menu.show(e.clientX, e.clientY, items);
  }

  function react(emoji: { id: string | null; name: string | null; animated?: boolean }) {
    void app.react(message, emoji);
  }

  function reactKey(r: { emoji: { id: string | null; name: string | null } }): string {
    return `${r.emoji.id ?? 'u'}:${r.emoji.name ?? ''}`;
  }

  /** Фолбэк, если CDN-картинка кастомного эмодзи недоступна: показываем :name:.
   */
  function emojiImgFallback(ev: Event, name: string | null) {
    const el = ev.currentTarget as HTMLImageElement;
    el.style.display = 'none';
    const span = document.createElement('span');
    span.textContent = name ? `:${name}:` : '❓';
    el.parentElement?.appendChild(span);
  }

  let pickerStyle = $state('');
  function openPicker(e: MouseEvent) {
    // Пикер над сообщением, у правого края — как в оригинале.
    const anchor = (e.currentTarget as HTMLElement).closest('.message') as HTMLElement | null;
    if (anchor) {
      const rect = anchor.getBoundingClientRect();
      const top = rect.top - 410;
      pickerStyle = `top: ${Math.max(12, top)}px; right: ${Math.max(12, window.innerWidth - rect.right)}px;`;
    }
    showPicker = true;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="message selectable" class:grouped oncontextmenu={onContext}>
  <div class="gutter">
    {#if !grouped}
      <Avatar user={message.author} size={40} ringColor="var(--bg-primary)" />
    {:else}
      <span class="hover-time muted">{formatTime(message.id)}</span>
    {/if}
  </div>

  <div class="body">
    {#if !grouped}
      <div class="header">
        <span class="author clickable" style="color: {color}">{name}</span>
        {#if message.author.bot}
          <span class="bot-tag">BOT</span>
        {/if}
        <span class="timestamp muted">{formatStamp(message.id)}</span>
      </div>
    {/if}

    <div class="content">
      {@html html}
      {#if message.edited_timestamp}
        <span class="edited muted"> (изменено)</span>
      {/if}
    </div>

    {#if message.reactions && message.reactions.length}
      <div class="reactions">
        {#each message.reactions as r (reactKey(r))}
          <button
            class="reaction"
            class:mine={r.me}
            onclick={() => react(r.emoji)}
            title={`:${r.emoji.name}: — ${r.count}`}
          >
            {#if emojiUrl(r.emoji)}
              <img
                src={emojiUrl(r.emoji) ?? ''}
                alt={r.emoji.name ?? ''}
                width={16}
                height={16}
                onerror={(ev) => emojiImgFallback(ev, r.emoji.name)}
              />
            {:else}
              <span>{r.emoji.name}</span>
            {/if}
            <span class="count">{r.count}</span>
          </button>
        {/each}
        <button class="reaction add" onclick={openPicker} title="Добавить реакцию">+</button>
      </div>
    {:else}
      <div class="reactions-placeholder"></div>
    {/if}
  </div>

  <div class="actions">
    <button class="act" title="Добавить реакцию" onclick={openPicker}>😀</button>
    {#if own}
      <button class="act" title="Изменить" onclick={() => app.startEdit(message)}>
        <Icon name="pencil" size={16} color="#b5bac1" />
      </button>
    {/if}
    {#if own}
      <button class="act" title="Удалить" onclick={() => void app.deleteMessage(message)}>
        <Icon name="trash" size={16} color="#b5bac1" />
      </button>
    {/if}
    <button class="act" title="Ещё" onclick={(e) => onContext(e)}>
      <Icon name="dots" size={16} color="#b5bac1" />
    </button>
  </div>

  {#if showPicker}
    <div class="picker-anchor" style={pickerStyle}>
      <EmojiPicker onpick={react} onclose={() => (showPicker = false)} />
    </div>
  {/if}
</div>

<style>
  .message {
    display: flex;
    gap: 16px;
    padding: 2px 48px 2px 16px;
    position: relative;
    margin-top: 16px;
    min-height: 44px;
  }
  .message.grouped { margin-top: 0; }
  .message:hover { background: #2e3035; }
  .message:hover .actions { display: flex; }

  .gutter { width: 40px; flex-shrink: 0; display: flex; justify-content: center; }
  .hover-time {
    font-size: 11px;
    opacity: 0;
    align-self: center;
  }
  .message:hover .hover-time { opacity: 1; }

  .body { flex: 1; min-width: 0; }
  .header {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-bottom: 2px;
  }
  .author { font-weight: 600; font-size: 15px; }
  .bot-tag {
    background: var(--blurple);
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 3px;
    vertical-align: 2px;
    line-height: 14px;
  }
  .timestamp { font-size: 12px; }
  .content {
    color: var(--text-normal);
    font-size: 15px;
    line-height: 1.375;
    overflow-wrap: anywhere;
  }
  .content :global(.mention) {
    background: rgba(88, 101, 242, 0.3);
    color: var(--mention-text);
    border-radius: 3px;
    padding: 0 2px;
    font-weight: 500;
    cursor: pointer;
  }
  .content :global(.mention.self-mention) {
    background: rgba(250, 166, 26, 0.25);
    color: #faa81a;
  }
  .content :global(.emoji) {
    width: 22px;
    height: 22px;
    vertical-align: -4px;
  }
  .content :global(.inline) {
    background: #1e1f22;
    border-radius: 3px;
    padding: 1px 4px;
    font-family: var(--font-code);
    font-size: 13px;
  }
  .content :global(.codeblock) {
    margin: 6px 0;
    border-radius: 4px;
    border: 1px solid #1e1f22;
    background: #2b2d31;
    overflow: hidden;
  }
  .content :global(.codeblock pre) {
    margin: 0;
    padding: 8px 10px;
    font-family: var(--font-code);
    font-size: 13px;
    line-height: 1.3;
    white-space: pre-wrap;
    color: #b5bac1;
  }
  .content :global(.spoiler) {
    background: #1e1f22;
    color: transparent;
    border-radius: 3px;
    padding: 0 3px;
    cursor: pointer;
  }
  .content :global(.spoiler:hover) { color: var(--text-normal); }
  .content :global(.quote-line) {
    display: inline-block;
    border-left: 4px solid #4e5058;
    padding-left: 8px;
    opacity: 0.85;
  }
  .edited { font-size: 10px; }

  .reactions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .reactions-placeholder {
    height: 0;
  }
  .picker-anchor {
    position: fixed;
    z-index: 91;
  }
  .reaction {
    display: flex;
    align-items: center;
    gap: 5px;
    background: #2b2d31;
    border: 1px solid transparent;
    border-radius: 8px;
    padding: 2px 7px;
    font-size: 14px;
    color: var(--text-muted);
  }
  .reaction:hover { border-color: rgba(255, 255, 255, 0.2); }
  .reaction.mine {
    background: rgba(88, 101, 242, 0.2);
    border-color: var(--blurple);
    color: var(--mention-text);
  }
  .reaction.add { color: var(--text-muted); font-weight: 600; }
  .count { font-size: 12px; font-weight: 600; }

  .actions {
    position: absolute;
    top: -16px;
    right: 16px;
    display: none;
    gap: 2px;
    background: var(--bg-secondary);
    border: 1px solid #1e1f22;
    border-radius: var(--radius-sm);
    padding: 2px;
  }
  .act {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
  }
  .act:hover { background: var(--bg-modifier-hover); }
</style>
