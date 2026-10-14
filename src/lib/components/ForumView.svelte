<script lang="ts">
  /**
   * Форум (type 15): список постов-обсуждений — каждый пост это тред.
   * Список — из активных тредов гильдии (+ REST-догрузка), клик по посту
   * открывает обычную ленту сообщений треда. Как в оригинале, только
   * списком вместо сетки карточек.
   */
  import { app } from '../stores/app.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  const channel = $derived(app.activeChannel);
  const posts = $derived(channel ? app.forumPosts(channel.id) : []);
  const guild = $derived(app.activeGuild);

  let creating = $state(false);
  let title = $state('');
  let firstMsg = $state('');
  let busy = $state(false);

  // Открытие форума — подтянуть свежий список активных тредов.
  $effect(() => {
    const g = app.activeGuild;
    if (g && app.isForumView) void app.refreshActiveThreads(g.id);
  });

  async function createPost() {
    if (!channel || !title.trim()) return;
    busy = true;
    try {
      await app.createThread(channel.id, title.trim(), firstMsg.trim() || undefined);
      creating = false;
      title = '';
      firstMsg = '';
    } finally {
      busy = false;
    }
  }

  function authorOf(id: string | undefined) {
    if (!id || !guild) return null;
    return app.memberById(guild.id, id)?.user ?? null;
  }
</script>

<div class="forum">
  <header class="head">
    <Icon name="members" size={22} color="#f2f3f5" />
    <h1>{channel?.name ?? 'форум'}</h1>
    <span class="count">{posts.length} {posts.length === 1 ? 'пост' : 'постов'}</span>
    <button class="new-post" onclick={() => (creating = !creating)}>+ Новый пост</button>
  </header>

  {#if channel?.topic}
    <p class="topic">{channel.topic}</p>
  {/if}

  {#if creating}
    <form class="composer" onsubmit={(e) => { e.preventDefault(); void createPost(); }}>
      <input placeholder="Заголовок поста" bind:value={title} maxlength={100} />
      <textarea placeholder="Первое сообщение (необязательно)" bind:value={firstMsg} rows={3} maxlength={2000}></textarea>
      <div class="row">
        <button type="submit" class="submit" disabled={!title.trim() || busy}>
          {busy ? 'Создаём…' : 'Создать пост'}
        </button>
        <button type="button" class="ghost" onclick={() => (creating = false)}>Отмена</button>
      </div>
    </form>
  {/if}

  <div class="posts">
    {#if posts.length === 0}
      <div class="empty">
        <p>Постов пока нет.</p>
        {#if !creating}
          <button class="new-post" onclick={() => (creating = true)}>Создай первый 🦀</button>
        {/if}
      </div>
    {:else}
      {#each posts as p (p.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div
          class="post clickable"
          role="button"
          tabindex="0"
          onclick={() => app.selectChannel(p.id)}
          onkeydown={(e) => e.key === 'Enter' && app.selectChannel(p.id)}
        >
          <div class="avatar-col">
            <Avatar user={authorOf(p.owner_id)} size={36} ringColor="var(--bg-secondary)" />
          </div>
          <div class="post-body">
            <div class="post-title truncate">{p.name}</div>
            <div class="post-meta">
              {#if p.thread_metadata?.archived}<span class="archived">в архиве</span>{/if}
              {#if p.thread_metadata?.locked}<span class="locked"><Icon name="lock" size={11} color="#949ba4" /> закрыт</span>{/if}
              {#if authorOf(p.owner_id)?.username}<span>автор @{authorOf(p.owner_id)?.username}</span>{/if}
            </div>
          </div>
          <Icon name="chevron-right" size={16} color="#949ba4" />
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .forum {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-primary);
    overflow-y: auto;
    padding: 16px 24px 32px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--bg-secondary);
  }
  h1 {
    color: var(--header-primary);
    font-size: 19px;
    font-weight: 700;
  }
  .count {
    color: var(--text-muted);
    font-size: 12.5px;
    margin-left: 4px;
  }
  .new-post {
    margin-left: auto;
    background: var(--blurple);
    color: #fff;
    border: none;
    border-radius: var(--radius-sm);
    padding: 8px 14px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .new-post:hover { filter: brightness(1.1); }
  .topic {
    color: var(--text-muted);
    font-size: 13px;
    margin: 10px 0 4px;
  }
  .composer {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--bg-secondary);
    border-radius: var(--radius-md);
    padding: 14px;
    margin: 14px 0;
  }
  .composer input, .composer textarea {
    background: var(--bg-primary);
    border: 1px solid #1e1f22;
    border-radius: var(--radius-sm);
    padding: 10px;
    font-size: 14px;
    color: var(--text-normal);
    resize: vertical;
  }
  .row { display: flex; gap: 10px; }
  .submit {
    background: var(--blurple);
    color: #fff;
    border: none;
    border-radius: var(--radius-sm);
    padding: 8px 16px;
    font-size: 13.5px;
    font-weight: 600;
    cursor: pointer;
  }
  .ghost {
    background: transparent;
    color: var(--text-muted);
    border: none;
    font-size: 13.5px;
    cursor: pointer;
  }
  .posts {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 12px;
  }
  .post {
    display: flex;
    align-items: center;
    gap: 12px;
    background: var(--bg-secondary);
    border: 1px solid #26272b;
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    transition: border-color 0.15s, background 0.15s;
  }
  .post:hover {
    border-color: #3f4147;
    background: #313338;
  }
  .post-body { flex: 1; min-width: 0; }
  .post-title {
    color: var(--header-primary);
    font-size: 15px;
    font-weight: 600;
  }
  .post-meta {
    display: flex;
    gap: 10px;
    align-items: center;
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 3px;
  }
  .locked { display: inline-flex; align-items: center; gap: 3px; }
  .archived { color: #f0b232; }
  .empty {
    text-align: center;
    color: var(--text-muted);
    padding: 48px 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: center;
  }
</style>
