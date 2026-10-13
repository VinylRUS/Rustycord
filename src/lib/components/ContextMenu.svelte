<script lang="ts">
  import { menu } from '../stores/menu.svelte';
</script>

{#if menu.open}
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => menu.close()} oncontextmenu={(e) => { e.preventDefault(); menu.close(); }}></div>
  <div class="menu" style="left: {menu.open.x}px; top: {menu.open.y}px">
    {#each menu.open.items as item, i (i)}
      <button class="item" class:danger={item.danger} onclick={() => menu.run(item)}>
        {item.label}
      </button>
    {/each}
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 90; }
  .menu {
    position: fixed;
    z-index: 91;
    background: #111214;
    border-radius: var(--radius-sm);
    padding: 6px 8px;
    min-width: 190px;
    box-shadow: 0 8px 16px rgba(0, 0, 0, 0.5);
    animation: popIn 0.1s ease;
  }
  .item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    font-size: 14px;
    color: var(--text-normal);
    font-weight: 500;
  }
  .item:hover { background: var(--blurple); color: #fff; }
  .item.danger { color: var(--red); }
  .item.danger:hover { background: var(--red); color: #fff; }
</style>
