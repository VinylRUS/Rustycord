<script lang="ts">
  /**
   * Quick Switcher (Ctrl+K) — мгновенный прыжок в любой канал/ЛС/сервер,
   * как в оригинале. Sublime-стиль: подстрочный матч с ранжированием
   * (префикс > слово > подстрока), стрелки/Enter/Esc.
   */
  import { app } from '../stores/app.svelte';
  import { displayName } from '../avatars';
  import type { Channel } from '../types';

  let query = $state('');
  let cursor = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);

  interface Hit {
    kind: 'guild' | 'channel' | 'dm';
    id: string;
    guildId: string | null;
    label: string;
    sub: string;
    where: string;
    score: number;
  }

  /** Sublime-подобный матч: префикс — 3, слово-старт — 2, подстрока — 1. */
  function matchScore(text: string, q: string): number {
    const t = text.toLowerCase();
    if (!q) return 0.5;
    if (t.startsWith(q)) return 3;
    const idx = t.indexOf(q);
    if (idx < 0) return -1;
    // старт слова (после пробела/дефиса/#)?
    const ch = idx === 0 ? ' ' : t[idx - 1];
    if (ch === ' ' || ch === '-' || ch === '_') return 2;
    return 1;
  }

  const hits = $derived.by(() => {
    const q = query.trim().toLowerCase().replace(/^[#@]/, '');
    const out: Hit[] = [];
    // Гильдии
    for (const g of app.guilds) {
      const s = matchScore(g.name ?? '', q);
      if (s >= 0) {
        out.push({
          kind: 'guild', id: g.id, guildId: g.id,
          label: g.name ?? 'Без названия', sub: 'Сервер', where: '',
          score: s + 0.2,
        });
      }
    }
    // Каналы всех гильдий (только видимые — как сайдбар)
    for (const g of app.guilds) {
      const channels = (g.channels ?? []).filter((c) => app.canViewChannel(g.id, c));
      for (const c of channels as Channel[]) {
        if (c.type === 4) continue;
        const s = matchScore(c.name ?? '', q);
        if (s >= 0) {
          const icon =
            c.type === 2 || c.type === 13 ? '🔊' :
            c.type === 5 ? '📣' :
            c.type === 15 ? '🗣' : '#';
          out.push({
            kind: 'channel', id: c.id, guildId: g.id,
            label: `${icon} ${c.name ?? 'без имени'}`,
            sub: 'Канал', where: g.name ?? '',
            score: s,
          });
        }
      }
    }
    // ЛС и группы (recipients — полные объекты User в сторе)
    for (const dm of app.dms) {
      const names = (dm.recipients ?? []).map((u) => displayName(u));
      const label = names.length ? names.join(', ') : 'Личный канал';
      const s = matchScore(label, q);
      if (s >= 0) {
        out.push({
          kind: 'dm', id: dm.id, guildId: null,
          label: (dm.recipients?.length ?? 0) > 1 ? `👥 ${label}` : label,
          sub: (dm.recipients?.length ?? 0) > 1 ? 'Группа' : 'Личные сообщения',
          where: '',
          score: s + 0.1,
        });
      }
    }
    out.sort((a, b) => b.score - a.score || a.label.localeCompare(b.label, 'ru'));
    return out.slice(0, 12);
  });

  $effect(() => {
    hits.length; // реактивность списка сбрасывает курсор
    cursor = 0;
  });

  $effect(() => {
    if (app.showQuickSwitcher) {
      query = '';
      // фокус после отрисовки
      requestAnimationFrame(() => inputEl?.focus());
    }
  });

  function jump(h: Hit) {
    app.showQuickSwitcher = false;
    if (h.kind === 'guild') {
      app.selectGuild(h.id);
    } else if (h.kind === 'channel' && h.guildId) {
      app.selectGuild(h.guildId);
      app.selectChannel(h.id);
    } else {
      app.selectGuild(null);
      app.selectChannel(h.id);
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      app.showQuickSwitcher = false;
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      cursor = Math.min(cursor + 1, hits.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      cursor = Math.max(cursor - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const h = hits[cursor] ?? hits[0];
      if (h) jump(h);
    }
  }
</script>

{#if app.showQuickSwitcher}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="qs-backdrop" onmousedown={() => (app.showQuickSwitcher = false)}>
    <div class="qs" role="dialog" aria-label="Быстрый переход" tabindex="-1" onmousedown={(e) => e.stopPropagation()}>
      <div class="qs-input-row">
        <span class="qs-hint">Куда бы вы хотели перейти?</span>
        <input
          bind:this={inputEl}
          bind:value={query}
          onkeydown={onKeydown}
          placeholder="Каналы, ЛС, серверы…"
          spellcheck="false"
        />
      </div>
      {#if hits.length}
        <div class="qs-list">
          {#each hits as h, i (h.kind + h.id)}
            <button class="qs-item" class:selected={i === cursor} onclick={() => jump(h)}>
              <span class="qs-label truncate">{h.label}</span>
              {#if h.where}<span class="qs-where truncate">в {h.where}</span>{/if}
              <span class="qs-kind">{h.sub}</span>
            </button>
          {/each}
        </div>
      {:else if query.trim()}
        <div class="qs-empty">Ничего не найдено</div>
      {:else}
        <div class="qs-empty">Начните печатать: каналы, ЛС или сервер</div>
      {/if}
      <div class="qs-footer">
        <span><b>↑↓</b> выбирать</span>
        <span><b>Enter</b> перейти</span>
        <span><b>Esc</b> закрыть</span>
      </div>
    </div>
  </div>
{/if}

<style>
  .qs-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    z-index: 200;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
  }
  .qs {
    width: min(560px, 90vw);
    background: var(--bg-primary, #313338);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
    overflow: hidden;
    animation: qsIn 0.12s ease;
  }
  @keyframes qsIn {
    from { opacity: 0; transform: scale(0.98); }
    to { opacity: 1; transform: scale(1); }
  }
  .qs-input-row {
    padding: 14px 16px 10px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }
  .qs-hint {
    display: block;
    color: var(--text-muted, #949ba4);
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    margin-bottom: 8px;
  }
  .qs input {
    width: 100%;
    background: var(--bg-base, #1e1f22);
    border-radius: 4px;
    padding: 10px 12px;
    font-size: 16px;
    color: var(--text-normal, #dbdee1);
  }
  .qs-list {
    max-height: 320px;
    overflow-y: auto;
    padding: 8px;
  }
  .qs-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 10px;
    border-radius: 4px;
  }
  .qs-item:hover, .qs-item.selected { background: var(--bg-modifier-hover, #3f4147); }
  .qs-label {
    color: var(--text-normal, #dbdee1);
    font-size: 15px;
    font-weight: 600;
    flex: 1;
    text-align: left;
  }
  .qs-where {
    color: var(--text-muted, #949ba4);
    font-size: 12px;
    max-width: 160px;
  }
  .qs-kind {
    color: var(--text-muted, #949ba4);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    flex-shrink: 0;
  }
  .qs-empty {
    padding: 24px 16px;
    color: var(--text-muted, #949ba4);
    font-size: 14px;
    text-align: center;
  }
  .qs-footer {
    display: flex;
    gap: 16px;
    padding: 8px 16px;
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    color: var(--text-muted, #949ba4);
    font-size: 11px;
  }
  .qs-footer b { color: var(--text-normal, #dbdee1); }
</style>
