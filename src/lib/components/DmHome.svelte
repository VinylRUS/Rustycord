<script lang="ts">
  import { app } from '../stores/app.svelte';
  import { displayName } from '../avatars';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
</script>

<div class="dm-home">
  <div class="hero">
    <div class="search-pill">
      <Icon name="search" size={18} color="#949ba4" />
      <input placeholder="Найти или начать беседу" />
    </div>
  </div>

  <div class="friends">
    <div class="f-head">
      <Icon name="members" size={22} color="#23a55a" />
      <h2>Друзья</h2>
      <span class="badge">{app.dms.length} бесед</span>
    </div>
    <div class="bot-note">
      <div class="bot-note-icon">🤖</div>
      <h3>У ботов нет друзей — и это нормально</h3>
      <p>
        Bot-аккаунты не могут отправлять заявки в друзья и первыми писать в ЛС.
        Зато когда пользователь напишет твоему боту, переписка появится здесь —
        с событием <code>CHANNEL_CREATE</code> и интентом <code>DIRECT_MESSAGES</code>.
      </p>
    </div>

    <div class="section-title">Недавние беседы</div>
    <div class="dm-rows">
      {#each app.dms as c (c.id)}
        <button class="dm-row" onclick={() => app.selectChannel(c.id)}>
          <Avatar user={c.recipients?.[0]} size={36} status={app.statusOf('', c.recipients?.[0]?.id ?? '')} ringColor="#313338" />
          <span class="dm-name">{c.recipients?.map((r) => displayName(r)).join(', ')}</span>
          <Icon name="chevron-right" size={16} color="#949ba4" />
        </button>
      {:else}
        <div class="muted empty">Пока никто не написал. Напиши своему боту первым — и канал появится.</div>
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
  }
  .search-pill {
    width: min(560px, 100%);
    background: var(--bg-base);
    border-radius: 8px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    height: 40px;
  }
  .search-pill input { flex: 1; font-size: 15px; }
  .friends { width: min(760px, 100%); padding: 16px; }
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
  h3 { color: var(--header-primary); font-size: 18px; margin-bottom: 8px; }
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
  .section-title {
    margin: 20px 0 8px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .dm-row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    border-top: 1px solid #3f414722;
  }
  .dm-row:hover { background: var(--bg-modifier-hover); }
  .dm-name { flex: 1; text-align: left; font-size: 15px; font-weight: 600; color: var(--text-normal); }
  .empty { padding: 16px; font-size: 14px; }
</style>
