<script lang="ts">
  /**
   * Пикер эмодзи для реакций — как в оригинале: поиск, «Часто используемые»,
   * секция кастомных эмодзи сервера, категории с заголовками и сайдбаром.
   */
  import { app } from '../stores/app.svelte';
  import { emojiUrl } from '../avatars';
  import { EMOJI_CATEGORIES, EMOJI_NAMES } from '../emoji-data';

  let {
    onpick,
    onclose,
  }: {
    onpick: (emoji: { id: string | null; name: string; animated?: boolean }) => void;
    onclose: () => void;
  } = $props();

  let query = $state('');
  let section = $state<'all' | string>('all');

  const guild = $derived(app.activeGuild);
  const serverEmoji = $derived(guild?.emojis ?? []);

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return EMOJI_CATEGORIES;
    // Поиск по трём уровням: имена популярных эмодзи (ru/en), ключевые
    // слова категории и её название. Пустые категории скрываются.
    const out = [];
    for (const c of EMOJI_CATEGORIES) {
      const byCategory =
        c.keywords.some((k) => k.toLowerCase().includes(q)) ||
        c.name.toLowerCase().includes(q);
      const emojis = byCategory
        ? c.emojis
        : c.emojis.filter(
            (e) =>
              EMOJI_NAMES[e]?.toLowerCase().includes(q) ?? false,
          );
      if (emojis.length) out.push({ ...c, emojis });
    }
    return out;
  });

  const serverFiltered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return serverEmoji;
    return serverEmoji.filter((e) => (e.name ?? '').toLowerCase().includes(q));
  });

  const hasResults = $derived(filtered.length > 0 || serverFiltered.length > 0);

  function pick(emoji: { id: string | null; name: string; animated?: boolean }) {
    onpick(emoji);
    onclose();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onclose} oncontextmenu={(e) => e.preventDefault()}></div>

<div class="picker" role="dialog" aria-label="Выбор эмодзи">
  <div class="head">
    <input
      type="text"
      placeholder="Поиск эмодзи"
      bind:value={query}
      spellcheck="false"
      maxlength="50"
    />
  </div>

  <div class="body">
    {#if app.recentEmoji.length && !query.trim()}
      <section>
        <h4>Часто используемые</h4>
        <div class="grid">
          {#each app.recentEmoji as e (e.id + ':' + e.name)}
            <button class="cell" title={`:${e.name}:`} onclick={() => pick(e)}>
              {#if e.id}
                <img
                  src={emojiUrl(e) ?? ''}
                  alt={e.name}
                  width={22}
                  height={22}
                  loading="lazy"
                  onerror={(ev) => {
                    const el = ev.currentTarget as HTMLImageElement;
                    el.style.display = 'none';
                    const fb = document.createElement('span');
                    fb.className = 'fb';
                    fb.textContent = (e.name ?? '?').slice(0, 6);
                    el.parentElement?.appendChild(fb);
                  }}
                />
              {:else}
                <span class="uni">{e.name}</span>
              {/if}
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if serverFiltered.length}
      <section>
        <h4>{guild ? `Эмодзи сервера — ${guild.name}` : 'Эмодзи сервера'}</h4>
        <div class="grid">
          {#each serverFiltered as e (e.id ?? e.name)}
            <button class="cell" title={`:${e.name}:`} onclick={() => pick({ id: e.id, name: e.name ?? '', animated: e.animated })}>
              <img
                src={emojiUrl(e) ?? ''}
                alt={e.name ?? ''}
                width={22}
                height={22}
                loading="lazy"
                onerror={(ev) => {
                  // CDN недоступен / эмодзи удалён — компактный :name: чип
                  // вместо вываливающейся из клетки строки.
                  const el = ev.currentTarget as HTMLImageElement;
                  el.style.display = 'none';
                  const fb = document.createElement('span');
                  fb.className = 'fb';
                  fb.textContent = (e.name ?? '?').slice(0, 6);
                  el.parentElement?.appendChild(fb);
                }}
              />
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#each filtered as cat (cat.id)}
      <section id={`cat-${cat.id}`}>
        <h4>{cat.name}</h4>
        <div class="grid">
          {#each cat.emojis as emoji (emoji)}
            <button class="cell" title={emoji} onclick={() => pick({ id: null, name: emoji })}>
              <span class="uni">{emoji}</span>
            </button>
          {/each}
        </div>
      </section>
    {/each}
    {#if !hasResults}
      <p class="empty">Ничего не найдено 🔍</p>
    {/if}
  </div>

  <div class="foot">
    {#each EMOJI_CATEGORIES.slice(0, 9) as cat (cat.id)}
      <button class="nav" title={cat.name} onclick={() => { section = cat.id; document.getElementById(`cat-${cat.id}`)?.scrollIntoView({ behavior: 'instant', block: 'start' }); }}>
        {cat.emojis[0] ?? '😀'}
      </button>
    {/each}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
  }
  .picker {
    position: fixed;
    z-index: 91;
    width: 342px;
    max-height: 420px;
    display: flex;
    flex-direction: column;
    background: var(--bg-floating, #2b2d31);
    border: 1px solid #1e1f22;
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    animation: pop 0.12s ease;
  }
  @keyframes pop {
    from { opacity: 0; transform: scale(0.96); }
    to { opacity: 1; transform: scale(1); }
  }
  .head {
    padding: 12px 12px 8px;
  }
  .head input {
    width: 100%;
    background: #1e1f22;
    border: 1px solid transparent;
    border-radius: 4px;
    padding: 7px 10px;
    font-size: 14px;
    color: var(--text-normal);
  }
  .head input:focus {
    outline: none;
    border-color: var(--blurple);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 0 8px 8px;
    scrollbar-width: thin;
  }
  section h4 {
    position: sticky;
    top: 0;
    background: #2b2d31;
    color: #b5bac1;
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    padding: 10px 4px 4px;
    z-index: 1;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, 38px);
    gap: 2px;
  }
  .cell {
    width: 38px;
    height: 38px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 5px;
    font-size: 22px;
    background: transparent;
  }
  .cell:hover {
    background: rgba(255, 255, 255, 0.08);
    transform: scale(1.15);
    transition: transform 0.08s ease;
  }
  .cell .uni {
    line-height: 1;
  }
  .cell img {
    object-fit: contain;
  }
  /* Фолбэк для кастомных эмодзи без картинки (CDN недоступен): короткое
     имя в 3 строки максимум — не ломает сетку. */
  .cell :global(.fb) {
    font-size: 9px;
    font-weight: 600;
    color: var(--text-muted);
    line-height: 1.1;
    text-align: center;
    word-break: break-all;
    max-width: 34px;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .empty {
    color: var(--text-muted);
    font-size: 14px;
    text-align: center;
    padding: 24px 0;
  }
  .foot {
    display: flex;
    gap: 2px;
    padding: 6px 10px 8px;
    background: #232428;
    border-top: 1px solid #1e1f22;
  }
  .nav {
    width: 32px;
    height: 28px;
    font-size: 17px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .nav:hover {
    background: rgba(255, 255, 255, 0.08);
  }
</style>
