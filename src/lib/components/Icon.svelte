<script lang="ts">
  // Minimal stroke icon set (24x24 grid) — Discord-flavoured glyphs.
  let {
    name,
    size = 20,
    color = 'currentColor',
  }: { name: string; size?: number; color?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke={color}
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'hash'}
    <line x1="9" y1="3" x2="7.5" y2="21" />
    <line x1="16.5" y1="3" x2="15" y2="21" />
    <line x1="4" y1="9" x2="20" y2="9" />
    <line x1="3" y1="15" x2="19" y2="15" />
  {:else if name === 'speaker'}
    <path d="M11 5 6.5 8.5H3v7h3.5L11 19z" fill="currentColor" stroke="none" />
    <path d="M15 9.5a3.5 3.5 0 0 1 0 5" />
    <path d="M17.5 7a7 7 0 0 1 0 10" />
  {:else if name === 'lock'}
    <rect x="5" y="11" width="14" height="9" rx="2" />
    <path d="M8 11V7a4 4 0 0 1 8 0v4" />
  {:else if name === 'chevron'}
    <polyline points="6 9 12 15 18 9" />
  {:else if name === 'chevron-right'}
    <polyline points="9 6 15 12 9 18" />
  {:else if name === 'mic'}
    <rect x="9" y="3" width="6" height="11" rx="3" />
    <path d="M5 11a7 7 0 0 0 14 0" />
    <line x1="12" y1="18" x2="12" y2="21" />
  {:else if name === 'mic-off'}
    <line x1="4" y1="4" x2="20" y2="20" />
    <path d="M9 5a3 3 0 0 1 6 0v6" />
    <path d="M5 11a7 7 0 0 0 10.7 6" />
    <line x1="12" y1="18" x2="12" y2="21" />
  {:else if name === 'headphones'}
    <path d="M4 14v-2a8 8 0 0 1 16 0v2" />
    <rect x="3" y="14" width="4" height="6" rx="2" />
    <rect x="17" y="14" width="4" height="6" rx="2" />
  {:else if name === 'headphones-off'}
    <line x1="4" y1="4" x2="20" y2="20" />
    <path d="M20 14v-2a8 8 0 0 0-14.5-4.6" />
    <rect x="3" y="14" width="4" height="6" rx="2" />
    <rect x="17" y="14" width="4" height="6" rx="2" />
  {:else if name === 'settings'}
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M19.1 4.9 17 7M7 17l-2.1 2.1" />
  {:else if name === 'members'}
    <circle cx="9" cy="8" r="3.5" />
    <path d="M2.5 20c0-3.6 2.9-6.5 6.5-6.5s6.5 2.9 6.5 6.5" />
    <path d="M16 4.6a3.5 3.5 0 0 1 0 6.8" />
    <path d="M17.5 13.7c2.4.9 4 3.2 4 5.8" />
  {:else if name === 'pin'}
    <path d="M14 2l8 8-4 1-4 6-2-1-5 6-1-1 6-5-1-2 6-4z" />
  {:else if name === 'search'}
    <circle cx="10.5" cy="10.5" r="6.5" />
    <line x1="15.5" y1="15.5" x2="21" y2="21" />
  {:else if name === 'phone-off'}
    <path d="M3 15c4-5 9-7 15-7l3 3-4 3-3-2c-2 1-4 2.5-5 5l-2-3-4 1z" fill="currentColor" stroke="none" transform="rotate(-8 12 12)" />
    <line x1="3" y1="3" x2="21" y2="21" />
  {:else if name === 'plus'}
    <line x1="12" y1="5" x2="12" y2="19" />
    <line x1="5" y1="12" x2="19" y2="12" />
  {:else if name === 'close'}
    <line x1="5" y1="5" x2="19" y2="19" />
    <line x1="19" y1="5" x2="5" y2="19" />
  {:else if name === 'dots'}
    <circle cx="5" cy="12" r="1.6" fill="currentColor" stroke="none" />
    <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
    <circle cx="19" cy="12" r="1.6" fill="currentColor" stroke="none" />
  {:else if name === 'copy'}
    <rect x="9" y="9" width="12" height="12" rx="2" />
    <path d="M5 15V5a2 2 0 0 1 2-2h10" />
  {:else if name === 'trash'}
    <path d="M4 7h16M9 7V4h6v3M6 7l1 14h10l1-14" />
  {:else if name === 'pencil'}
    <path d="M4 20h4L20.5 7.5a2.1 2.1 0 0 0-3-3L5 17z" />
  {:else if name === 'reply'}
    <path d="M10 9V5l-7 7 7 7v-4.1c5 0 8.5 1.6 11 5.1-1-5-4-10-11-11z" />
  {:else if name === 'emoji'}
    <circle cx="12" cy="12" r="9.5" />
    <circle cx="9" cy="10" r="1" fill="currentColor" stroke="none" />
    <circle cx="15" cy="10" r="1" fill="currentColor" stroke="none" />
    <path d="M8 14.5c1 1.5 2.4 2.2 4 2.2s3-.7 4-2.2" />
  {:else if name === 'chat'}
    <path d="M21 11.5c0 4.1-4 7.5-9 7.5-1 0-2-.1-2.9-.4L4 20l1.2-3.6C3.8 15.1 3 13.4 3 11.5 3 7.4 7 4 12 4s9 3.4 9 7.5z" />
  {:else if name === 'logout'}
    <path d="M9 4H5a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1h4" />
    <path d="M15 8l4 4-4 4M19 12H9" />
  {:else if name === 'ban'}
    <circle cx="12" cy="12" r="9" />
    <line x1="5.6" y1="5.6" x2="18.4" y2="18.4" />
  {:else if name === 'kick'}
    <path d="M14 8V4h6v6" />
    <path d="M20 4 10 14" />
    <path d="M10 8H5v7h5l4 4v-15z" fill="currentColor" stroke="none" />
  {:else if name === 'signal'}
    <path d="M4 18v-3M9 18v-7M14 18V7M19 18V3" />
  {:else if name === 'screen'}
    <rect x="3" y="4.5" width="18" height="12.5" rx="1.5" />
    <path d="M9 20.5h6M12 17v3.5" />
  {:else if name === 'camera'}
    <rect x="2.5" y="6.5" width="13" height="11" rx="2" />
    <path d="M15.5 10.5 21 7.5v9l-5.5-3z" />
  {:else if name === 'check'}
    <polyline points="4 12.5 9.5 18 20 6.5" />
  {:else if name === 'shield'}
    <path d="M12 2 4 5.5v6C4 17 7.5 20.5 12 22c4.5-1.5 8-5 8-10.5v-6z" />
  {:else if name === 'info'}
    <circle cx="12" cy="12" r="9.5" />
    <line x1="12" y1="11" x2="12" y2="16.5" />
    <circle cx="12" cy="7.8" r="1" fill="currentColor" stroke="none" />
  {:else if name === 'logo'}
    <path d="M12 3C6.9 3 3 6.9 3 11.6c0 2.7 1.4 5.1 3.5 6.7-.1 1.5-.7 2.9-1.7 4 2.1-.3 4-1.2 5.5-2.5.6.1 1.1.1 1.7.1 5.1 0 9-3.9 9-8.4S17.1 3 12 3z" fill="currentColor" stroke="none" />
    <circle cx="8.6" cy="11.4" r="1.3" fill="var(--bg-base)" stroke="none" />
    <circle cx="12" cy="11.4" r="1.3" fill="var(--bg-base)" stroke="none" />
    <circle cx="15.4" cy="11.4" r="1.3" fill="var(--bg-base)" stroke="none" />
  {:else}
    <circle cx="12" cy="12" r="9" />
  {/if}
</svg>
