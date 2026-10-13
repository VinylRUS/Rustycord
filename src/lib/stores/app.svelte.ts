/**
 * AppStore — central state on Svelte 5 runes.
 *
 * One class instance owns everything: connection, cache (mirroring what the
 * Rust core keeps in memory), UI selection state and all actions.
 * Components read reactive fields; only this file mutates them.
 */
import { createBackend, type Backend } from '../ipc';
import type {
  Channel,
  ConnStage,
  Guild,
  Member,
  Message,
  Presence,
  Role,
  User,
  VoiceState,
} from '../types';
import { displayName } from '../avatars';

const TOKEN_KEY = 'rustycord.token';
const TYPING_TTL = 8000;
const SPEAKING_TTL = 2500;

interface TypingEntry {
  name: string;
  until: number;
}

interface MyVoice {
  guildId: string;
  channelId: string;
  selfMute: boolean;
  selfDeaf: boolean;
  mode?: string;
  endpoint?: string;
  ssrc?: number;
}

export class AppStore {
  /* ---------- connection ---------- */
  backend: Backend | null = null;
  unlisten: (() => void) | null = null;
  mode = $state<'none' | 'tauri' | 'mock'>('none');
  connStage = $state<ConnStage | 'offline'>('offline');
  loginBusy = $state(false);
  loginError = $state('');
  booted = false;

  /* ---------- cache (mirror of Rust cache) ---------- */
  user = $state<User | null>(null);
  sessionId = $state('');
  guilds = $state<Guild[]>([]);
  dms = $state<Channel[]>([]);
  membersByGuild = $state<Record<string, Record<string, Member>>>({});
  presencesByGuild = $state<Record<string, Record<string, Presence>>>({});
  voiceByGuild = $state<Record<string, VoiceState[]>>({});
  chunksDone = $state<Record<string, boolean>>({});
  messagesByChannel = $state<Record<string, Message[]>>({});
  messagesLoaded = $state<Record<string, boolean>>({});
  loadingOlder = $state<Record<string, boolean>>({});

  /* ---------- voice ---------- */
  myVoice = $state<MyVoice | null>(null);
  speaking = $state<Record<string, boolean>>({});

  /* ---------- ui ---------- */
  activeGuildId = $state<string | null>(null); // null → DM home
  activeChannelId = $state<string | null>(null);
  lastChannelByGuild = $state<Record<string, string>>({});
  showMembers = $state(true);
  showSettings = $state(false);
  unread = $state<Record<string, boolean>>({});
  typingByChannel = $state<Record<string, Record<string, TypingEntry>>>({});
  drafts = $state<Record<string, string>>({});
  editing = $state<{ channelId: string; messageId: string } | null>(null);
  profileCard = $state<{ userId: string; guildId: string } | null>(null);
  toasts = $state<{ id: number; text: string; kind: 'info' | 'error' }[]>([]);

  private toastSeq = 1;
  private typingTimers: Record<string, ReturnType<typeof setTimeout>> = {};
  private speakingTimers: Record<string, ReturnType<typeof setTimeout>> = {};
  private lastTypingSent: Record<string, number> = {};

  /* ================= derived ================= */

  get activeGuild(): Guild | null {
    return this.guilds.find((g) => g.id === this.activeGuildId) ?? null;
  }

  get activeChannel(): Channel | null {
    if (!this.activeChannelId) return null;
    const fromGuild = this.activeGuild?.channels?.find((c) => c.id === this.activeChannelId);
    if (fromGuild) return fromGuild;
    return this.dms.find((c) => c.id === this.activeChannelId) ?? null;
  }

  get activeMessages(): Message[] {
    return (this.activeChannelId && this.messagesByChannel[this.activeChannelId]) || [];
  }

  /** All channels of the active guild sorted like Discord does. */
  get activeGuildChannels(): Channel[] {
    const chans = this.activeGuild?.channels ?? [];
    return [...chans].sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
  }

  get channelTitle(): string {
    const ch = this.activeChannel;
    if (!ch) return '';
    if (ch.type === 1 || ch.type === 3) return ch.recipients?.map((r) => displayName(r)).join(', ') || 'Личные сообщения';
    return ch.name ?? '';
  }

  /* ================= lifecycle ================= */

  async boot() {
    if (this.booted) return;
    this.booted = true;
    const token = localStorage.getItem(TOKEN_KEY);
    if (token && detectTauri()) {
      await this.login(token, true);
    }
  }

  async login(token: string, silent = false) {
    this.loginBusy = true;
    this.loginError = '';
    try {
      const backend = await createBackend('tauri');
      this.attach(backend);
      const user = await backend.connect(token);
      this.user = user;
      localStorage.setItem(TOKEN_KEY, token);
    } catch (e) {
      this.loginError = e instanceof Error ? e.message : String(e);
      this.detach();
      if (!silent) {
        // stay on login screen
      }
    } finally {
      this.loginBusy = false;
    }
  }

  async startDemo() {
    this.loginBusy = true;
    this.loginError = '';
    try {
      const backend = await createBackend('mock');
      this.attach(backend);
      const user = await backend.connect('demo');
      this.user = user;
    } catch (e) {
      this.loginError = e instanceof Error ? e.message : String(e);
      this.detach();
    } finally {
      this.loginBusy = false;
    }
  }

  async logout() {
    localStorage.removeItem(TOKEN_KEY);
    try {
      await this.backend?.disconnect();
    } catch {
      /* ignore */
    }
    this.detach();
    this.reset();
  }

  private attach(backend: Backend) {
    this.detach();
    this.backend = backend;
    this.mode = backend.mode;
    this.unlisten = backend.onEvent((ev) => this.handleEvent(ev));
  }

  private detach() {
    this.unlisten?.();
    this.unlisten = null;
    this.backend = null;
  }

  private reset() {
    this.user = null;
    this.guilds = [];
    this.dms = [];
    this.membersByGuild = {};
    this.presencesByGuild = {};
    this.voiceByGuild = {};
    this.chunksDone = {};
    this.messagesByChannel = {};
    this.messagesLoaded = {};
    this.activeGuildId = null;
    this.activeChannelId = null;
    this.myVoice = null;
    this.speaking = {};
    this.unread = {};
    this.connStage = 'offline';
    this.mode = 'none';
    this.showSettings = false;
  }

  /* ================= event stream ================= */

  handleEvent(ev: import('../types').BackendEvent) {
    switch (ev.type) {
      case 'status':
        this.connStage = ev.stage;
        if (ev.stage === 'connected' || ev.stage === 'reconnecting' || ev.stage === 'connecting') {
          // keep UI alive; banner shows stage
        }
        if (ev.stage === 'disconnected' && !this.user) {
          this.loginError = ev.detail ?? 'Соединение потеряно';
        }
        break;
      case 'ready':
        this.user = ev.user;
        this.sessionId = ev.session_id;
        break;
      case 'dispatch':
        this.handleDispatch(ev.t, ev.d);
        break;
      case 'voice':
        this.handleVoice(ev.kind, ev.data);
        break;
      case 'warning':
        this.toast(ev.message, 'error');
        break;
    }
  }

  handleVoice(
    kind: 'connected' | 'disconnected' | 'speaking',
    data: { guild_id: string; user_id?: string; channel_id?: string | null; speaking?: boolean; mode?: string; endpoint?: string; ssrc?: number; ip?: string; port?: number },
  ) {
    if (kind === 'connected') {
      if (this.myVoice) {
        this.myVoice = {
          ...this.myVoice,
          mode: data.mode,
          endpoint: data.endpoint,
          ssrc: data.ssrc,
        };
      }
      this.toast(`Голос подключён · ${data.mode ?? ''} · ${data.ip}:${data.port}`, 'info');
    } else if (kind === 'disconnected') {
      this.toast('Голосовой канал отключён', 'info');
    } else if (kind === 'speaking' && data.user_id) {
      const uid = data.user_id;
      this.speaking[uid] = !!data.speaking;
      clearTimeout(this.speakingTimers[uid]);
      if (data.speaking) {
        this.speakingTimers[uid] = setTimeout(() => {
          this.speaking[uid] = false;
        }, SPEAKING_TTL);
      }
    }
  }

  handleDispatch(t: string, d: unknown) {
    switch (t) {
      case 'READY': {
        const p = d as { user: User; session_id: string };
        this.user = p.user;
        this.sessionId = p.session_id;
        break;
      }
      case 'GUILD_CREATE': {
        const g = d as Guild;
        this.guilds = this.guilds.filter((x) => x.id !== g.id);
        this.guilds.push(g);
        // seed caches
        const members: Record<string, Member> = {};
        for (const m of g.members ?? []) if (m.user) members[m.user.id] = m;
        this.membersByGuild[g.id] = { ...(this.membersByGuild[g.id] ?? {}), ...members };
        const pres: Record<string, Presence> = {};
        for (const p of g.presences ?? []) pres[p.user.id] = p;
        this.presencesByGuild[g.id] = pres;
        this.voiceByGuild[g.id] = g.voice_states ?? [];
        // auto-select first guild for a lively landing view
        if (!this.activeGuildId) this.selectGuild(g.id);
        break;
      }
      case 'GUILD_UPDATE': {
        const g = d as Guild;
        const idx = this.guilds.findIndex((x) => x.id === g.id);
        if (idx >= 0) this.guilds[idx] = { ...this.guilds[idx], ...g };
        break;
      }
      case 'GUILD_DELETE': {
        const g = d as { id: string };
        this.guilds = this.guilds.filter((x) => x.id !== g.id);
        if (this.activeGuildId === g.id) this.selectGuild(this.guilds[0]?.id ?? null);
        break;
      }
      case 'GUILD_ROLE_UPDATE':
      case 'GUILD_ROLE_CREATE': {
        const p = d as { guild_id: string; role: Role };
        const g = this.guilds.find((x) => x.id === p.guild_id);
        if (g) {
          g.roles = g.roles.filter((r) => r.id !== p.role.id);
          g.roles.push(p.role);
        }
        break;
      }
      case 'GUILD_ROLE_DELETE': {
        const p = d as { guild_id: string; role_id: string };
        const g = this.guilds.find((x) => x.id === p.guild_id);
        if (g) g.roles = g.roles.filter((r) => r.id !== p.role_id);
        break;
      }
      case 'GUILD_MEMBERS_CHUNK': {
        const p = d as { guild_id: string; members: Member[]; chunk_index: number; chunk_count: number };
        const map = { ...(this.membersByGuild[p.guild_id] ?? {}) };
        for (const m of p.members) if (m.user) map[m.user.id] = m;
        this.membersByGuild[p.guild_id] = map;
        if (p.chunk_index + 1 >= p.chunk_count) this.chunksDone[p.guild_id] = true;
        break;
      }
      case 'GUILD_MEMBER_ADD': {
        const p = d as { guild_id: string; member: Member };
        if (p.member.user) {
          const map = { ...(this.membersByGuild[p.guild_id] ?? {}) };
          map[p.member.user.id] = p.member;
          this.membersByGuild[p.guild_id] = map;
        }
        break;
      }
      case 'GUILD_MEMBER_UPDATE': {
        const p = d as { guild_id: string; member: Member };
        if (p.member.user) {
          const map = { ...(this.membersByGuild[p.guild_id] ?? {}) };
          map[p.member.user.id] = { ...map[p.member.user.id], ...p.member };
          this.membersByGuild[p.guild_id] = map;
        }
        break;
      }
      case 'GUILD_MEMBER_REMOVE': {
        const p = d as { guild_id: string; user: { id: string } };
        const map = { ...(this.membersByGuild[p.guild_id] ?? {}) };
        delete map[p.user.id];
        this.membersByGuild[p.guild_id] = map;
        break;
      }
      case 'CHANNEL_CREATE': {
        const c = d as Channel;
        if (c.type === 1 || c.type === 3) {
          if (!this.dms.some((x) => x.id === c.id)) this.dms.push(c);
        } else if (c.guild_id) {
          const g = this.guilds.find((x) => x.id === c.guild_id);
          if (g && !g.channels?.some((x) => x.id === c.id)) g.channels?.push(c);
        }
        break;
      }
      case 'CHANNEL_UPDATE': {
        const c = d as Channel;
        if (c.guild_id) {
          const g = this.guilds.find((x) => x.id === c.guild_id);
          if (g?.channels) {
            const i = g.channels.findIndex((x) => x.id === c.id);
            if (i >= 0) g.channels[i] = c;
          }
        }
        break;
      }
      case 'CHANNEL_DELETE': {
        const c = d as Channel;
        this.dms = this.dms.filter((x) => x.id !== c.id);
        const g = this.guilds.find((x) => x.id === c.guild_id);
        if (g?.channels) g.channels = g.channels.filter((x) => x.id !== c.id);
        if (this.activeChannelId === c.id) this.selectChannel(null);
        break;
      }
      case 'MESSAGE_CREATE': {
        const m = d as Message;
        this.appendMessage(m.channel_id, m);
        if (m.channel_id !== this.activeChannelId) this.unread[m.channel_id] = true;
        this.clearTyping(m.channel_id, m.author.id);
        break;
      }
      case 'MESSAGE_UPDATE': {
        const m = d as Message;
        const list = this.messagesByChannel[m.channel_id];
        if (list) {
          const i = list.findIndex((x) => x.id === m.id);
          if (i >= 0) list[i] = { ...list[i], ...m };
        }
        break;
      }
      case 'MESSAGE_DELETE': {
        const p = d as { channel_id: string; id: string };
        const list = this.messagesByChannel[p.channel_id];
        if (list) this.messagesByChannel[p.channel_id] = list.filter((m) => m.id !== p.id);
        break;
      }
      case 'MESSAGE_REACTION_ADD':
      case 'MESSAGE_REACTION_REMOVE': {
        const p = d as {
          channel_id: string;
          message_id: string;
          user_id: string;
          emoji: { id: string | null; name: string | null };
        };
        const list = this.messagesByChannel[p.channel_id];
        const m = list?.find((x) => x.id === p.message_id);
        if (!m) break;
        const reactions = [...(m.reactions ?? [])];
        const key = (e: { id: string | null; name: string | null }) => `${e.id ?? ''}:${e.name ?? ''}`;
        const i = reactions.findIndex((r) => key(r.emoji) === key(p.emoji));
        const isAdd = t === 'MESSAGE_REACTION_ADD';
        const isMe = p.user_id === this.user?.id;
        if (isAdd) {
          if (i >= 0) reactions[i] = { ...reactions[i], count: reactions[i].count + 1, me: reactions[i].me || isMe };
          else reactions.push({ count: 1, me: isMe, emoji: p.emoji });
        } else if (i >= 0) {
          const count = reactions[i].count - 1;
          if (count <= 0) reactions.splice(i, 1);
          else reactions[i] = { ...reactions[i], count, me: isMe ? false : reactions[i].me };
        }
        m.reactions = reactions;
        break;
      }
      case 'TYPING_START': {
        const p = d as { channel_id: string; user_id: string };
        if (p.user_id === this.user?.id) break;
        const guildId = this.activeGuild?.id;
        const name = this.memberName(guildId ?? '', p.user_id);
        const chMap = { ...(this.typingByChannel[p.channel_id] ?? {}) };
        chMap[p.user_id] = { name, until: Date.now() + TYPING_TTL };
        this.typingByChannel[p.channel_id] = chMap;
        const timerKey = `${p.channel_id}:${p.user_id}`;
        clearTimeout(this.typingTimers[timerKey]);
        this.typingTimers[timerKey] = setTimeout(() => this.clearTyping(p.channel_id, p.user_id), TYPING_TTL);
        break;
      }
      case 'PRESENCE_UPDATE': {
        const p = d as Presence;
        for (const gid of Object.keys(this.presencesByGuild)) {
          const map = this.presencesByGuild[gid];
          if (map[p.user.id] || this.membersByGuild[gid]?.[p.user.id]) {
            map[p.user.id] = p;
          }
        }
        break;
      }
      case 'VOICE_STATE_UPDATE': {
        const vs = d as VoiceState;
        if (!vs.guild_id) break;
        const list = (this.voiceByGuild[vs.guild_id] ?? []).filter((v) => v.user_id !== vs.user_id);
        if (vs.channel_id) list.push(vs);
        this.voiceByGuild[vs.guild_id] = list;
        if (vs.user_id === this.user?.id) {
          if (vs.channel_id) {
            this.myVoice = {
              guildId: vs.guild_id,
              channelId: vs.channel_id,
              selfMute: !!vs.self_mute,
              selfDeaf: !!vs.self_deaf,
            };
          } else {
            this.myVoice = null;
          }
        }
        break;
      }
      default:
        // Unknown events are intentionally ignored — like the real client,
        // forward-compatibility comes from graceful skipping.
        break;
    }
  }

  /* ================= actions ================= */

  selectGuild(id: string | null) {
    this.activeGuildId = id;
    const guild = this.guilds.find((g) => g.id === id);
    if (!guild) {
      this.activeChannelId = null;
      return;
    }
    const remembered = id !== null ? this.lastChannelByGuild[id] : undefined;
    const channels = (guild.channels ?? []).filter((c) => c.type !== 4);
    const target = channels.find((c) => c.id === remembered) ?? channels.find((c) => c.type === 0) ?? channels[0];
    this.selectChannel(target?.id ?? null);
  }

  selectChannel(id: string | null) {
    this.activeChannelId = id;
    if (id) {
      delete this.unread[id];
      if (this.activeGuildId) this.lastChannelByGuild[this.activeGuildId] = id;
      if (!this.messagesLoaded[id]) void this.loadMessages(id);
    }
  }

  async loadMessages(channelId: string) {
    if (!this.backend) return;
    try {
      const list = await this.backend.getMessages(channelId, { limit: 50 });
      this.messagesByChannel[channelId] = list.reverse();
      this.messagesLoaded[channelId] = true;
    } catch (e) {
      this.toast(`Не удалось загрузить историю: ${errMsg(e)}`, 'error');
    }
  }

  async loadOlder(channelId: string) {
    if (!this.backend || this.loadingOlder[channelId]) return;
    const list = this.messagesByChannel[channelId] ?? [];
    if (!list.length) return;
    this.loadingOlder[channelId] = true;
    try {
      const older = await this.backend.getMessages(channelId, { limit: 50, before: list[0].id });
      if (older.length) {
        this.messagesByChannel[channelId] = [...older.reverse(), ...list];
      }
    } catch {
      /* silent */
    } finally {
      this.loadingOlder[channelId] = false;
    }
  }

  get draft(): string {
    return (this.activeChannelId && this.drafts[this.activeChannelId]) || '';
  }

  set draft(v: string) {
    if (this.activeChannelId) this.drafts[this.activeChannelId] = v;
  }

  async sendMessage() {
    const ch = this.activeChannelId;
    if (!ch || !this.backend) return;
    const content = this.draft.trim();
    if (!content) return;
    this.draft = '';
    try {
      const m = await this.backend.sendMessage(ch, content);
      this.appendMessage(ch, m);
    } catch (e) {
      this.draft = content;
      this.toast(`Не удалось отправить: ${errMsg(e)}`, 'error');
    }
  }

  appendMessage(channelId: string, m: Message) {
    const list = this.messagesByChannel[channelId];
    if (!list) return; // history not loaded — will come from REST
    const i = list.findIndex((x) => x.id === m.id);
    if (i >= 0) list[i] = m;
    else list.push(m);
  }

  startEdit(message: Message) {
    this.editing = { channelId: message.channel_id, messageId: message.id };
    this.drafts[message.channel_id] = message.content;
  }

  async commitEdit() {
    if (!this.editing || !this.backend) return;
    const { channelId, messageId } = this.editing;
    const content = (this.drafts[channelId] ?? '').trim();
    if (!content) return;
    try {
      await this.backend.editMessage(channelId, messageId, content);
      const list = this.messagesByChannel[channelId];
      const m = list?.find((x) => x.id === messageId);
      if (m) m.edited_timestamp = new Date().toISOString();
    } catch (e) {
      this.toast(`Не удалось изменить: ${errMsg(e)}`, 'error');
    } finally {
      this.cancelEdit();
    }
  }

  cancelEdit() {
    if (this.editing) this.drafts[this.editing.channelId] = '';
    this.editing = null;
  }

  async deleteMessage(message: Message) {
    if (!this.backend) return;
    try {
      await this.backend.deleteMessage(message.channel_id, message.id);
    } catch (e) {
      this.toast(`Не удалось удалить: ${errMsg(e)}`, 'error');
    }
  }

  async react(message: Message, emoji: string) {
    if (!this.backend) return;
    try {
      await this.backend.addReaction(message.channel_id, message.id, emoji);
    } catch (e) {
      this.toast(`Реакция не удалась: ${errMsg(e)}`, 'error');
    }
  }

  async typingPing() {
    const ch = this.activeChannelId;
    if (!ch || !this.backend) return;
    const now = Date.now();
    if (now - (this.lastTypingSent[ch] ?? 0) < 7000) return;
    this.lastTypingSent[ch] = now;
    try {
      await this.backend.triggerTyping(ch);
    } catch {
      /* silent */
    }
  }

  async toggleVoice(guildId: string, channelId: string) {
    if (!this.backend) return;
    const joined = this.myVoice?.guildId === guildId && this.myVoice?.channelId === channelId;
    try {
      await this.backend.joinVoice(
        guildId,
        joined ? null : channelId,
        this.myVoice?.selfMute ?? false,
        this.myVoice?.selfDeaf ?? false,
      );
    } catch (e) {
      this.toast(`Ошибка войса: ${errMsg(e)}`, 'error');
    }
  }

  async leaveVoice() {
    if (!this.backend || !this.myVoice) return;
    try {
      await this.backend.joinVoice(this.myVoice.guildId, null, false, false);
    } catch (e) {
      this.toast(`Ошибка выхода: ${errMsg(e)}`, 'error');
    }
  }

  async setSelfMute(mute: boolean) {
    if (!this.backend || !this.myVoice) return;
    try {
      await this.backend.joinVoice(this.myVoice.guildId, this.myVoice.channelId, mute, this.myVoice.selfDeaf);
    } catch {
      /* silent */
    }
  }

  async setSelfDeaf(deaf: boolean) {
    if (!this.backend || !this.myVoice) return;
    try {
      await this.backend.joinVoice(this.myVoice.guildId, this.myVoice.channelId, this.myVoice?.selfMute ?? false, deaf);
    } catch {
      /* silent */
    }
  }

  async kickMember(guildId: string, userId: string) {
    if (!this.backend) return;
    try {
      await this.backend.kickMember(guildId, userId);
      this.toast('Участник исключён', 'info');
    } catch (e) {
      this.toast(`Кик не удался: ${errMsg(e)}`, 'error');
    }
  }

  async banMember(guildId: string, userId: string) {
    if (!this.backend) return;
    try {
      await this.backend.banMember(guildId, userId);
      this.toast('Участник забанен', 'info');
    } catch (e) {
      this.toast(`Бан не удался: ${errMsg(e)}`, 'error');
    }
  }

  async parseSnowflake(id: string) {
    if (!this.backend) throw new Error('Нет подключения');
    return this.backend.parseSnowflake(id);
  }

  /** Escape hatch: сырой JSON-кадр прямо в gateway websocket. */
  async gatewaySendRaw(frame: string) {
    if (!this.backend) throw new Error('Нет подключения');
    return this.backend.gatewaySendRaw(frame);
  }

  /* ================= helpers ================= */

  memberName(guildId: string, userId: string): string {
    const m = this.membersByGuild[guildId]?.[userId];
    if (m) return m.nick || displayName(m.user);
    const dm = this.dms.find((c) => c.recipients?.some((r) => r.id === userId));
    const u = dm?.recipients?.find((r) => r.id === userId);
    if (u) return displayName(u);
    const g = this.guilds.find((x) => x.id === guildId);
    const viaVoice = this.voiceByGuild[guildId]?.find((v) => v.user_id === userId);
    if (viaVoice && g) {
      // fall back to a member-less presence name
      const p = this.presencesByGuild[guildId]?.[userId];
      if (p) return `id:${userId.slice(-4)}`;
    }
    return `id:${userId.slice(-6)}`;
  }

  memberById(guildId: string, userId: string): Member | null {
    return this.membersByGuild[guildId]?.[userId] ?? null;
  }

  userById(userId: string): User | null {
    for (const map of Object.values(this.membersByGuild)) {
      const m = map[userId];
      if (m?.user) return m.user;
    }
    for (const c of this.dms) {
      const u = c.recipients?.find((r) => r.id === userId);
      if (u) return u;
    }
    return null;
  }

  channelById(id: string): Channel | null {
    for (const g of this.guilds) {
      const c = g.channels?.find((x) => x.id === id);
      if (c) return c;
    }
    return this.dms.find((c) => c.id === id) ?? null;
  }

  /** Highest hoisted role of a member — defines the member-list group & color. */
  topRole(guildId: string, userId: string): Role | null {
    const guild = this.guilds.find((g) => g.id === guildId);
    const member = this.membersByGuild[guildId]?.[userId];
    if (!guild || !member) return null;
    const roles = guild.roles
      .filter((r) => member.roles.includes(r.id))
      .sort((a, b) => b.position - a.position);
    return roles[0] ?? null;
  }

  memberColor(guildId: string, userId: string): string {
    const roles = this.guilds.find((g) => g.id === guildId)?.roles ?? [];
    const member = this.membersByGuild[guildId]?.[userId];
    if (!member) return 'var(--header-primary)';
    const colored = roles
      .filter((r) => member.roles.includes(r.id) && r.color !== 0)
      .sort((a, b) => b.position - a.position);
    return colored[0] ? `#${colored[0].color.toString(16).padStart(6, '0')}` : 'var(--header-primary)';
  }

  presenceOf(guildId: string, userId: string): Presence | null {
    return this.presencesByGuild[guildId]?.[userId] ?? null;
  }

  statusOf(guildId: string, userId: string): string {
    return this.presencesByGuild[guildId]?.[userId]?.status ?? 'offline';
  }

  voiceUsers(guildId: string, channelId: string): VoiceState[] {
    return (this.voiceByGuild[guildId] ?? []).filter((v) => v.channel_id === channelId);
  }

  typingNames(channelId: string): string[] {
    const map = this.typingByChannel[channelId];
    if (!map) return [];
    const now = Date.now();
    return Object.entries(map)
      .filter(([, v]) => v.until > now)
      .map(([, v]) => v.name);
  }

  private clearTyping(channelId: string, userId: string) {
    const map = this.typingByChannel[channelId];
    if (!map || !map[userId]) return;
    const next = { ...map };
    delete next[userId];
    this.typingByChannel[channelId] = next;
    clearTimeout(this.typingTimers[`${channelId}:${userId}`]);
  }

  toast(text: string, kind: 'info' | 'error' = 'info') {
    const id = this.toastSeq++;
    this.toasts.push({ id, text, kind });
    setTimeout(() => {
      this.toasts = this.toasts.filter((t) => t.id !== id);
    }, 4200);
  }

  get isDemo(): boolean {
    return this.mode === 'mock';
  }

  get connected(): boolean {
    return this.connStage === 'connected';
  }
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function detectTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export const app = new AppStore();
