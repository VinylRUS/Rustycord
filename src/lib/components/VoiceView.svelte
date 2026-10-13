<script lang="ts">
  import { app } from '../stores/app.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  const guild = $derived(app.activeGuild);
  const channel = $derived(app.activeChannel);
  const channelId = $derived(channel?.id ?? '');
  const participants = $derived(guild ? app.voiceUsers(guild.id, channelId) : []);
</script>

<section class="voice-view">
  <header>
    <Icon name="speaker" size={22} color="#23a55a" />
    <h1>{channel?.name ?? 'Голосовой канал'}</h1>
    {#if app.myVoice?.guildId === guild?.id && app.myVoice?.channelId === channelId}
      <span class="connected"><Icon name="signal" size={14} color="#23a55a" /> Подключено</span>
    {:else}
      <button class="btn-blurple join" onclick={() => guild && channel && void app.toggleVoice(guild.id, channel.id)}>
        Подключиться к голосу
      </button>
    {/if}
    {#if app.myVoice?.guildId === guild?.id && app.myVoice?.channelId === channelId}
      <button class="leave" title="Отключиться" onclick={() => void app.leaveVoice()}>
        <Icon name="phone-off" size={20} color="#f23f43" />
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
        <h2>Здесь тихо…</h2>
        <p class="muted">Подключись первым — покажем протокол в деле: op 4 → VOICE_SERVER_UPDATE → UDP handshake → xsalsa20_poly1305.</p>
      </div>
    {/each}
  </div>

  {#if app.myVoice?.mode}
    <footer class="proto-info mono">
      protocol v4 · mode {app.myVoice.mode} · ssrc {app.myVoice.ssrc} · {app.myVoice.endpoint}
    </footer>
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
  .join { font-size: 13px; padding: 8px 14px; }
  .leave {
    width: 34px;
    height: 34px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .leave:hover { background: #f23f4322; }
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
  .proto-info {
    padding: 8px 16px;
    color: var(--text-muted);
    font-size: 11px;
    border-top: 1px solid #26282c;
    flex-shrink: 0;
  }
</style>
