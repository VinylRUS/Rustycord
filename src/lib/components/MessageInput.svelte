<script lang="ts">
  import { app } from '../stores/app.svelte';

  let textareaEl = $state<HTMLTextAreaElement | null>(null);

  const placeholder = $derived(`Написать в ${app.channelTitle ? (app.activeChannel?.type === 1 ? app.channelTitle : '#' + app.channelTitle) : 'канал'}`);
  const editing = $derived(app.editing !== null && app.editing.channelId === app.activeChannelId);

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (editing) {
        void app.commitEdit();
      } else {
        void app.sendMessage();
      }
    } else if (e.key === 'Escape' && editing) {
      app.cancelEdit();
    } else {
      void app.typingPing();
    }
  }
</script>

<div class="input-wrap">
  {#if editing}
    <div class="edit-banner">
      <span>Изменение сообщения · <b>Esc</b> — отмена</span>
      <button class="cancel" onclick={() => app.cancelEdit()}>Отмена</button>
    </div>
  {/if}
  <div class="input">
    <button class="plus" title="Прикрепить файл (скоро)">+</button>
    <textarea
      bind:this={textareaEl}
      placeholder={placeholder}
      rows={1}
      bind:value={app.draft}
      onkeydown={onKeydown}
      oninput={() => {
        const el = textareaEl;
        if (el) {
          el.style.height = 'auto';
          el.style.height = Math.min(el.scrollHeight, 300) + 'px';
        }
      }}
    ></textarea>
    <button class="send emoji-btn" title="Реакция" onclick={() => (app.draft += '👍')}>👍</button>
  </div>
</div>

<style>
  .input-wrap { padding: 0 16px 24px; }
  .edit-banner {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: #111214;
    color: var(--text-muted);
    font-size: 12px;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    padding: 6px 16px;
  }
  .cancel { color: var(--text-link); font-size: 12px; }
  .cancel:hover { text-decoration: underline; }
  .input {
    background: var(--input-background);
    border-radius: var(--radius-md);
    display: flex;
    align-items: flex-start;
    padding: 0 4px;
  }
  :global(.edit-banner) + .input { border-top-left-radius: 0; border-top-right-radius: 0; }
  .plus {
    width: 40px;
    height: 44px;
    color: #b5bac1;
    font-size: 22px;
    flex-shrink: 0;
    border-radius: 50%;
    transition: background 0.1s;
  }
  .plus:hover { background: #ffffff14; }
  textarea {
    flex: 1;
    resize: none;
    padding: 11px 0;
    font-size: 15px;
    line-height: 1.375;
    max-height: 300px;
    color: var(--text-normal);
    background: transparent;
  }
  textarea::placeholder { color: var(--input-placeholder); }
  .emoji-btn {
    width: 40px;
    height: 44px;
    font-size: 20px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
