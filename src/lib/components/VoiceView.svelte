<script lang="ts">
  import { app } from '../stores/app.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import type { VoiceDspStatus } from '../types';

  const guild = $derived(app.activeGuild);
  const channel = $derived(app.activeChannel);
  const channelId = $derived(channel?.id ?? '');
  const participants = $derived(guild ? app.voiceUsers(guild.id, channelId) : []);
  const streams = $derived(app.streamsInMyChannel());
  const connected = $derived(
    !!app.myVoice && app.myVoice.guildId === (guild?.id ?? '') && app.myVoice.channelId === channelId,
  );

  /** Уровень своего микрофона после обработки (DSP-телеметрия, 7 Гц). */
  let micLevel = $state<VoiceDspStatus | null>(null);
  $effect(() => {
    if (!connected || !app.backend) {
      micLevel = null;
      return;
    }
    let alive = true;
    const poll = async () => {
      try {
        const st = await app.backend!.getVoiceDspStatus();
        if (alive) micLevel = st;
      } catch {
        /* сессия закрылась — индикатор погаснет через connected */
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 150);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  });

  /** dBFS → проценты шкалы (−60…0 → 0…100). */
  const micPct = $derived(
    micLevel?.active
      ? Math.max(0, Math.min(100, ((micLevel.metrics.output_db + 60) / 60) * 100))
      : 0,
  );

  // Локальное превью своего захвата (экран/камера) — биндим MediaStream
  // к <video> и обновляем при смене источника.
  let previewEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    const stream = app.localPreview;
    if (previewEl && stream) {
      previewEl.srcObject = stream;
      void previewEl.play().catch(() => undefined);
    }
  });
</script>

<section class="voice-view">
  <header>
    <Icon name="speaker" size={22} color="#23a55a" />
    <h1>{channel?.name ?? 'Голосовой канал'}</h1>
    {#if connected}
      <span class="connected"><Icon name="signal" size={14} color="#23a55a" /> Подключено</span>
      {#if micLevel?.active}
        <div
          class="mic-meter"
          class:speaking={micLevel.metrics.speech}
          title="Уровень микрофона — ровно то, что уходит собеседникам"
        >
          <Icon name="mic" size={14} color={micLevel.metrics.speech ? '#23a55a' : '#b5bac1'} />
          <div class="mic-track"><div class="mic-fill" style:width="{micPct}%"></div></div>
        </div>
      {/if}
    {:else}
      <button class="btn-blurple join" onclick={() => guild && channel && void app.toggleVoice(guild.id, channel.id)}>
        Подключиться к голосу
      </button>
    {/if}
  </header>

  <div class="tiles">
    {#each participants as vs (vs.user_id)}
      {@const member = guild ? app.memberById(guild.id, vs.user_id) : null}
      <div class="tile" class:speaking={!!app.speaking[vs.user_id]}>
        <Avatar user={member?.user ?? null} size={96} speaking={!!app.speaking[vs.user_id]} ringColor="#1e1f22" />
        <div class="tile-name">
          <span>{member?.nick || member?.user?.global_name || member?.user?.username || vs.user_id}</span>
          {#if vs.self_mute}<Icon name="mic-off" size={13} color="#f23f43" />{/if}
          {#if vs.self_deaf}<Icon name="headphones-off" size={13} color="#f23f43" />{/if}
        </div>
      </div>
    {:else}
      <div class="empty">
        <div class="big"><Icon name="speaker" size={48} color="#4e5058" /></div>
        <h2>Здесь пока никого нет</h2>
        <p class="muted">Подключись к голосовому каналу — друзья увидят, что ты здесь, и присоединятся.</p>
      </div>
    {/each}
  </div>

  {#if app.localPreview}
    <div class="preview">
      <div class="preview-head">
        <Icon name={app.localPreviewKind === 'camera' ? 'camera' : 'screen'} size={16} color="#23a55a" />
        <span>{app.localPreviewKind === 'camera' ? 'Камера (превью)' : 'Экран (превью)'}</span>
        <span class="muted experimental">эксперимент · передача в след. обновлении</span>
        <button class="preview-stop" title="Остановить" onclick={() => void app.stopLocalPreview()}>
          <Icon name="close" size={14} color="#f23f43" />
        </button>
      </div>
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={previewEl} muted playsinline></video>
    </div>
  {/if}

  {#if streams.length}
    <div class="streams">
      {#each streams as s (s.streamKey)}
        <div class="stream-chip" class:paused={s.paused}>
          <Icon name={s.type === 'camera' ? 'camera' : 'screen'} size={14} color={s.paused ? '#f0b232' : '#23a55a'} />
          <span class="truncate">
            {s.userId === app.user?.id
              ? 'Вы в эфире'
              : guild
                ? (app.memberById(guild.id, s.userId)?.nick ??
                  app.memberById(guild.id, s.userId)?.user?.global_name ??
                  app.memberById(guild.id, s.userId)?.user?.username ?? 'Участник')
                : 'Участник'}
          </span>
          <span class="muted">{s.type === 'camera' ? 'камера' : 'экран'}{s.paused ? ' · пауза' : ''}</span>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .voice-view {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--bg-primary);
    min-width: 0;
  }
  .preview {
    margin: 8px 16px 0;
    border-radius: 8px;
    overflow: hidden;
    background: #000;
  }
  .preview-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    color: var(--header-primary);
    font-size: 12px;
    font-weight: 600;
  }
  .experimental { font-weight: 400; font-size: 11px; }
  .preview-stop {
    margin-left: auto;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .preview-stop:hover { background: #f23f4322; }
  .preview video {
    width: 100%;
    max-height: 320px;
    display: block;
    background: #000;
  }
  .streams {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 10px 16px 0;
  }
  .stream-chip {
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--bg-secondary);
    border-radius: 6px;
    padding: 5px 10px;
    font-size: 12px;
    color: var(--header-primary);
    max-width: 260px;
  }
  .stream-chip.paused { opacity: 0.7; }
  header {
    height: 48px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px;
    box-shadow: 0 1px 0 rgba(0, 0, 0, 0.2);
    flex-shrink: 0;
  }
  h1 { font-size: 16px; color: var(--header-primary); flex: 1; }
  .connected {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--green);
    font-size: 13px;
    font-weight: 600;
  }
  .mic-meter {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    background: var(--bg-secondary);
  }
  .mic-meter.speaking { box-shadow: inset 0 0 0 1px #23a55a66; }
  .mic-track {
    width: 72px;
    height: 5px;
    border-radius: 3px;
    background: #1e1f22;
    overflow: hidden;
  }
  .mic-fill {
    height: 100%;
    border-radius: 3px;
    background: #23a55a;
    transition: width 0.12s linear;
  }
  .join { font-size: 13px; padding: 8px 14px; }
  .tiles {
    flex: 1;
    display: flex;
    flex-wrap: wrap;
    align-content: flex-start;
    gap: 16px;
    padding: 24px;
    overflow-y: auto;
  }
  .tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 16px;
    border-radius: var(--radius-md);
    min-width: 160px;
  }
  .tile.speaking { background: #23a55a22; }
  .tile-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    color: var(--text-normal);
    font-size: 14px;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    text-align: center;
    max-width: 480px;
    margin: 0 auto;
  }
  .big {
    width: 100px;
    height: 100px;
    border-radius: 50%;
    background: #2b2d31;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  h2 { color: var(--header-primary); font-size: 24px; }
</style>
