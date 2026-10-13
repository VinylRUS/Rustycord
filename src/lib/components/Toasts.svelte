<script lang="ts">
  import { app } from '../stores/app.svelte';
  import Icon from './Icon.svelte';
</script>

{#if app.toasts.length}
  <div class="toasts">
    {#each app.toasts as t (t.id)}
      <div class="toast" class:error={t.kind === 'error'}>
        <Icon name={t.kind === 'error' ? 'info' : 'check'} size={16} color={t.kind === 'error' ? '#f23f43' : '#23a55a'} />
        <span>{t.text}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toasts {
    position: fixed;
    top: 16px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 120;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 520px;
  }
  .toast {
    background: #111214;
    color: var(--text-normal);
    border: 1px solid #26282c;
    border-radius: var(--radius-sm);
    padding: 10px 14px;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    box-shadow: 0 8px 16px rgba(0, 0, 0, 0.4);
    animation: popIn 0.15s ease;
  }
  .toast.error { border-color: #f23f4366; }
</style>
