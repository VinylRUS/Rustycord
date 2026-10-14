<script lang="ts">
  import { app } from '../stores/app.svelte';
  import type { Attachment, Message, MessageEmbed, StickerItem } from '../types';
  import { renderMarkdown } from '../markdown';
  import { formatStamp, formatTime } from '../time';
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
  let lightbox = $state<Attachment | null>(null);

  const guildId = $derived(app.activeGuild?.id ?? '');
  const own = $derived(message.author.id === app.user?.id);
  const color = $derived(app.memberColor(guildId, message.author.id));
  const name = $derived(app.memberById(guildId, message.author.id)?.nick || message.author.global_name || message.author.username);

  /** Сообщение-родитель для превью ответа (REST вкладывает его целиком). */
  const referenced = $derived.by(() => {
    const ref = message.referenced_message;
    if (ref) return ref;
    const refId = message.message_reference?.message_id;
    if (!refId) return null;
    return app.messagesByChannel[message.channel_id]?.find((m) => m.id === refId) ?? null;
  });
  const referencedName = $derived(
    referenced
      ? app.memberById(guildId, referenced.author.id)?.nick || referenced.author.global_name || referenced.author.username
      : '',
  );

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
      { label: 'Ответить', action: () => app.startReply(message) },
      { label: 'Копировать текст', action: () => void navigator.clipboard?.writeText(message.content) },
      { label: 'Копировать ID сообщения', action: () => void navigator.clipboard?.writeText(message.id) },
      { label: message.pinned ? 'Открепить' : 'Закрепить', action: () => void app.togglePin(message) },
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

  /* ---------- вложения / эмбеды ---------- */

  function isImage(a: Attachment): boolean {
    if (a.content_type?.startsWith('image/')) return true;
    return /\.(png|jpe?g|gif|webp|bmp)$/i.test(a.filename);
  }
  function isVideo(a: Attachment): boolean {
    if (a.content_type?.startsWith('video/')) return true;
    return /\.(mp4|webm|mov)$/i.test(a.filename);
  }
  function isAudio(a: Attachment): boolean {
    if (a.content_type?.startsWith('audio/')) return true;
    return /\.(mp3|ogg|wav|flac|m4a)$/i.test(a.filename);
  }
  function fmtSize(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} МБ`;
    if (bytes >= 1024) return `${Math.round(bytes / 1024)} КБ`;
    return `${bytes} Б`;
  }

  /** CDN-URL стикера: PNG/APNG рендерятся как <img>, Lottie — заглушка. */
  function stickerUrl(s: StickerItem): string {
    return `https://cdn.discordapp.com/stickers/${s.id}.png`;
  }

  function embedColor(e: MessageEmbed): string | null {
    return typeof e.color === 'number' && e.color !== 0 ? `#${e.color.toString(16).padStart(6, '0')}` : null;
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
        {#if message.pinned}
          <span class="pin-mark" title="Закреплено">📌</span>
        {/if}
      </div>
    {/if}

    <!-- Превью сообщения-родителя (ответ) -->
    {#if message.message_reference && !grouped}
      <div class="reply-line" title={referenced ? referenced.content : 'Сообщение недоступно'}>
        <span class="reply-spacer"></span>
        {#if referenced}
          <Avatar user={referenced.author} size={16} />
          <span class="reply-author" style="color: {app.memberColor(guildId, referenced.author.id)}">{referencedName}</span>
          <span class="reply-snippet muted">{referenced.content || (referenced.attachments.length ? 'вложение' : '…')}</span>
        {:else}
          <span class="reply-snippet muted">начало беседы</span>
        {/if}
      </div>
    {/if}

    <div class="content">
      {@html html}
      {#if message.edited_timestamp}
        <span class="edited muted"> (изменено)</span>
      {/if}
    </div>

    <!-- Стикеры -->
    {#if message.sticker_items?.length}
      <div class="stickers">
        {#each message.sticker_items as s (s.id)}
          {#if s.format_type !== 3}
            <img class="sticker" src={stickerUrl(s)} alt={s.name} width={120} height={120} loading="lazy"
              onerror={(ev) => ((ev.currentTarget as HTMLImageElement).style.display = 'none')} />
          {:else}
            <div class="sticker-lottie" title="{s.name} (Lottie)">🎬 {s.name}</div>
          {/if}
        {/each}
      </div>
    {/if}

    <!-- Вложения: картинки / видео / аудио / файлы -->
    {#if message.attachments.length}
      <div class="attachments">
        {#each message.attachments as a (a.id)}
          {#if isImage(a)}
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <img
              class="attachment-img"
              src={a.url}
              alt={a.filename}
              loading="lazy"
              onclick={() => (lightbox = a)}
              onkeydown={() => (lightbox = a)}
            />
          {:else if isVideo(a)}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video class="attachment-video" controls preload="metadata" src={a.url}></video>
          {:else if isAudio(a)}
            <audio controls preload="metadata" src={a.url}></audio>
          {:else}
            <a class="file-chip" href={a.url} target="_blank" rel="noreferrer">
              <span class="file-icon">📄</span>
              <span class="file-meta">
                <span class="file-name">{a.filename}</span>
                <span class="file-size muted">{fmtSize(a.size)}</span>
              </span>
            </a>
          {/if}
        {/each}
      </div>
    {/if}

    <!-- Эмбеды (превью ссылок) -->
    {#if message.embeds?.length}
      {#each message.embeds as e, i (i)}
        {#if e.type !== 'image' || e.image}
          <div class="embed" style={embedColor(e) ? `--embed-color: ${embedColor(e)}` : ''}>
            <div class="embed-body">
              {#if e.author?.name}
                <div class="embed-author">{e.author.name}</div>
              {/if}
              {#if e.title}
                <div class="embed-title">{e.title}</div>
              {/if}
              {#if e.description}
                <div class="embed-desc">{e.description}</div>
              {/if}
              {#if e.fields?.length}
                <div class="embed-fields">
                  {#each e.fields as f, fi (fi)}
                    <div class="embed-field" class:inline={f.inline}>
                      <div class="embed-field-name">{f.name}</div>
                      <div class="embed-field-value">{f.value}</div>
                    </div>
                  {/each}
                </div>
              {/if}
              {#if e.image?.url}
                <img class="embed-image" src={e.image.proxy_url ?? e.image.url} alt="" loading="lazy" />
              {/if}
              {#if e.footer?.text}
                <div class="embed-footer muted">{e.footer.text}</div>
              {/if}
            </div>
            {#if e.thumbnail?.url}
              <img class="embed-thumb" src={e.thumbnail.proxy_url ?? e.thumbnail.url} alt="" loading="lazy" />
            {/if}
          </div>
        {/if}
      {/each}
    {/if}

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
    <button class="act" title="Ответить" onclick={() => app.startReply(message)}>
      <Icon name="reply" size={16} color="#b5bac1" />
    </button>
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

{#if lightbox}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events, a11y_interactive_supports_focus -->
  <div
    class="lightbox"
    onclick={() => (lightbox = null)}
    role="dialog"
    aria-label={lightbox.filename}
    tabindex="-1"
  >
    <img src={lightbox.url} alt={lightbox.filename} />
    <a class="lightbox-dl" href={lightbox.url} target="_blank" rel="noreferrer">Открыть оригинал</a>
    <button class="lightbox-close" onclick={() => (lightbox = null)}>✕</button>
  </div>
{/if}

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
  .pin-mark { font-size: 11px; }

  .reply-line {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
    font-size: 13px;
    max-width: 100%;
  }
  .reply-line::before {
    content: '';
    width: 30px;
    height: 10px;
    border-left: 2px solid #4e5058;
    border-top: 2px solid #4e5058;
    border-top-left-radius: 6px;
    margin-left: -16px;
    flex-shrink: 0;
    align-self: flex-start;
    margin-top: 6px;
  }
  .reply-author { font-weight: 600; flex-shrink: 0; }
  .reply-snippet {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 420px;
    font-size: 13px;
  }

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

  /* ---------- вложения ---------- */
  .attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 6px;
  }
  .attachment-img {
    max-width: min(400px, 100%);
    max-height: 300px;
    border-radius: 8px;
    cursor: zoom-in;
    object-fit: contain;
  }
  .attachment-video {
    max-width: min(480px, 100%);
    max-height: 320px;
    border-radius: 8px;
  }
  .file-chip {
    display: flex;
    align-items: center;
    gap: 10px;
    background: #2b2d31;
    border: 1px solid #1e1f22;
    border-radius: 8px;
    padding: 10px 14px;
    max-width: 400px;
  }
  .file-chip:hover { border-color: #4e5058; }
  .file-icon { font-size: 26px; }
  .file-meta { display: flex; flex-direction: column; min-width: 0; }
  .file-name {
    color: var(--text-link);
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 280px;
  }
  .file-size { font-size: 12px; }

  /* ---------- стикеры ---------- */
  .stickers { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 6px; }
  .sticker { object-fit: contain; }
  .sticker-lottie {
    width: 120px;
    height: 60px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #2b2d31;
    border-radius: 8px;
    color: var(--text-muted);
    font-size: 13px;
    padding: 0 8px;
  }

  /* ---------- эмбеды ---------- */
  .embed {
    display: flex;
    gap: 12px;
    background: #2b2d31;
    border-left: 4px solid var(--embed-color, #4e5058);
    border-radius: 6px;
    margin-top: 6px;
    padding: 10px 12px;
    max-width: 460px;
  }
  .embed-body { flex: 1; min-width: 0; }
  .embed-author { font-size: 12px; font-weight: 600; color: var(--text-muted); margin-bottom: 2px; }
  .embed-title { font-size: 14px; font-weight: 700; color: var(--text-link); margin: 2px 0; }
  .embed-desc { font-size: 13px; line-height: 1.35; color: var(--text-normal); overflow-wrap: anywhere; }
  .embed-desc :global(a) { color: var(--text-link); }
  .embed-fields { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 6px; }
  .embed-field { flex: 1 1 100%; }
  .embed-field.inline { flex: 1 1 30%; min-width: 100px; }
  .embed-field-name { font-size: 12px; font-weight: 700; }
  .embed-field-value { font-size: 12px; color: var(--text-muted); }
  .embed-image { max-width: 100%; max-height: 240px; border-radius: 4px; margin-top: 8px; object-fit: contain; }
  .embed-thumb { width: 80px; height: 80px; border-radius: 4px; object-fit: cover; flex-shrink: 0; align-self: flex-start; }
  .embed-footer { font-size: 11px; margin-top: 6px; }

  /* ---------- лайтбокс ---------- */
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 120;
    background: rgba(0, 0, 0, 0.85);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
  }
  .lightbox img { max-width: 90vw; max-height: 82vh; border-radius: 4px; }
  .lightbox-dl { color: #fff; font-size: 14px; text-decoration: underline; }
  .lightbox-close {
    position: absolute;
    top: 16px;
    right: 20px;
    color: #fff;
    font-size: 22px;
    width: 40px;
    height: 40px;
    border-radius: 50%;
  }
  .lightbox-close:hover { background: #ffffff22; }

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
