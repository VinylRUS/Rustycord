<script lang="ts">
  import { defaultAvatarColor, displayName, userAvatarUrl } from '../avatars';
  import type { User } from '../types';

  let {
    user,
    size = 40,
    status = '',
    speaking = false,
    ringColor = 'var(--bg-secondary)',
  }: {
    user: User | null | undefined;
    size?: number;
    status?: string;
    speaking?: boolean;
    ringColor?: string;
  } = $props();

  let failed = $state(false);
  const url = $derived(user ? userAvatarUrl(user, size <= 32 ? 64 : 128) : null);
  const name = $derived(user ? displayName(user) : '?');
  const color = $derived(user ? defaultAvatarColor(user.id) : '#757e8a');
  const initial = $derived((name || '?').trim().charAt(0).toUpperCase());

  const statusColor = $derived(
    { online: '#23a55a', idle: '#f0b232', dnd: '#f23f43', offline: '#80848e' }[status] ?? 'transparent',
  );
</script>

<div class="avatar-wrap" style="--sz: {size}px">
  <div class="avatar" class:speaking style="--ring: {ringColor}">
    {#if url && !failed}
      <img src={url} alt={name} width={size} height={size} loading="lazy" onerror={() => (failed = true)} />
    {:else}
      <div class="fallback" style="background: {color}; font-size: {Math.round(size * 0.42)}px">{initial}</div>
    {/if}
  </div>
  {#if status}
    <span class="dot" class:offline={status === 'offline'} style="--dot: {Math.max(10, Math.round(size * 0.31))}px">
      <span class="dot-inner" style="background: {statusColor}"></span>
    </span>
  {/if}
</div>

<style>
  .avatar-wrap {
    position: relative;
    width: var(--sz);
    height: var(--sz);
    flex-shrink: 0;
  }
  .avatar {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    overflow: hidden;
    position: relative;
  }
  .avatar.speaking {
    box-shadow: 0 0 0 2px var(--ring), 0 0 0 4px var(--green);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .fallback {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    font-weight: 600;
  }
  .dot {
    position: absolute;
    right: -2px;
    bottom: -2px;
    width: var(--dot);
    height: var(--dot);
    border-radius: 50%;
    background: var(--ring);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .dot-inner {
    width: 72%;
    height: 72%;
    border-radius: 50%;
    display: block;
  }
  .dot.offline .dot-inner {
    background: #80848e;
    mask: radial-gradient(circle at 50% 50%, transparent 38%, #80848e 40%);
    -webkit-mask: radial-gradient(circle at 50% 50%, transparent 38%, #80848e 40%);
  }
</style>
