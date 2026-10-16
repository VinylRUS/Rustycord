/**
 * AppStore — central state on Svelte 5 runes.
 *
 * One class instance owns everything: connection, cache (mirroring what the
 * Rust core keeps in memory), UI selection state and all actions.
 * Components read reactive fields; only this file mutates them.
 */
import { createBackend, type Backend } from '../ipc';
import { plugins, onPluginToast } from '../plugins/manager';
import {
  computeChannelPermissions,
  computePermissions,
  has,
  PERMISSIONS,
} from '../permissions';
import type {
  BanEntry,
  Channel,
  ConnStage,
  Guild,
  Invite,
  Member,
  MemberListData,
  Message,
  Presence,
  QrStage,
  RailOrderItem,
  Relationship,
  Role,
  UpdateInfo,
  User,
  UserProfile,
  VoiceState,
} from '../types';
import { displayName } from '../avatars';

const TOKEN_KEY = 'rustycord.token';
const TYPING_TTL = 8000;
const SPEAKING_TTL = 2500;
const RECENT_EMOJI_KEY = 'rustycord.recentEmoji';
const RECENT_EMOJI_MAX = 24;

/** Объект обновления из plugin-updater (держим для installUpdate;
 *  сознательно вне $state — в рендере не участвует). */
let pendingUpdate: import('@tauri-apps/plugin-updater').Update | null = null;

/** Элемент рейла серверов: отдельная гильдия или папка (index в
 *  guildFolders — раскладка декодируется в Rust и приходит событием). */
export type RailItem = RailOrderItem;

/** Уникальный ключ эмодзи в списке реакций (id:name — кастомные уникальны id). */
export function emojiKey(e: { id: string | null; name: string | null }): string {
  return `${e.id ?? ''}:${e.name ?? ''}`;
}

/** Строка эмодзи для REST-путей реакций: unicode или name:id (кастомный). */
export function emojiApi(e: { id: string | null; name: string | null }): string {
  return e.id ? `${e.name}:${e.id}` : (e.name ?? '👍');
}

interface TypingEntry {
  name: string;
  until: number;
}

interface MyVoice {
  /** '' для звонка в ЛС/группе (протокол DM-звонков: guild_id = null). */
  guildId: string;
  channelId: string;
  selfMute: boolean;
  selfDeaf: boolean;
  /** Камера «включена» на уровне op4 (без передачи видео пока). */
  selfVideo: boolean;
  mode?: string;
  endpoint?: string;
  ssrc?: number;
}

/** Активная трансляция (стрим) в голосовом канале — своя или чужая. */
export interface StreamInfo {
  streamKey: string;
  guildId: string;
  channelId: string;
  userId: string;
  type: 'screen' | 'camera' | string;
  viewerCount?: number;
  paused?: boolean;
}

export class AppStore {
  /* ---------- connection ---------- */
  backend: Backend | null = null;
  unlisten: (() => void) | null = null;
  unlistenToast: (() => void) | null = null;
  mode = $state<'none' | 'tauri' | 'mock'>('none');
  connStage = $state<ConnStage | 'offline'>('offline');
  /** Детали последнего обрыва (close-код/причина) — видны в баннере. */
  connDetail = $state('');
  loginBusy = $state(false);
  loginError = $state('');
  booted = false;

  /* ---------- user auth (v0.3.0) ---------- */
  /** QR-флоу: 'connecting' → 'fingerprint' → qrSvg доступен; 'done' → автологин. */
  qrStage = $state<QrStage | 'idle'>('idle');
  qrSvg = $state('');
  qrFingerprint = $state('');
  /** Кто отсканировал QR (из pending_ticket) — показываем на экране подтверждения. */
  qrScannedBy = $state('');
  /** email-логин дошёл до 2FA — ждём код из приложения. */
  mfaTicket = $state<string | null>(null);

  /* ---------- модалка управления сервером ---------- */
  showGuildSettings = $state<string | null>(null);

  /* ---------- cache (mirror of Rust cache) ---------- */
  user = $state<User | null>(null);
  sessionId = $state('');
  guilds = $state<Guild[]>([]);
  /** Папки серверов и порядок рейла — из user_settings (proto/legacy,
   *  декодирует Rust, событие guildLayout). Порядок папок и отдельных
   *  серверов — как расставил юзер в официальном клиенте. */
  guildFolders = $state<import('../types').GuildFolder[]>([]);
  railOrder = $state<RailItem[]>([]);
  /** Плоский порядок для сортировки this.guilds: railOrder + хвост для
   *  гильдий вне раскладки (вступил после запуска). */
  guildOrder = $state<Record<string, number>>({});
  dms = $state<Channel[]>([]);
  /** Юзер-READY несёт массив users — все упомянутые в ЛС участники.
   *  Кэш «кто есть кто» для recipient_ids-каналов и упоминаний. */
  usersById = $state<Record<string, User>>({});
  /** Друзья/заявки/блок-лист (user_id → relationship). Юзер-аккаунт только. */
  relationships = $state<Record<string, Relationship>>({});
  /** Статусы друзей (PRESENCE_UPDATE без guild_id + READY.presences). */
  friendPresences = $state<Record<string, Presence>>({});
  membersByGuild = $state<Record<string, Record<string, Member>>>({});
  presencesByGuild = $state<Record<string, Record<string, Presence>>>({});
  voiceByGuild = $state<Record<string, VoiceState[]>>({});
  /** Ленивые списки участников (guild_id → снапшот GUILD_MEMBER_LIST_UPDATE,
   *  ответ на op 37 с channels): группы + участники с пресенсами, порядок
   *  уже собран сервером — 1:1 с сайдбаром официального клиента. */
  memberLists = $state<Record<string, MemberListData>>({});
  chunksDone = $state<Record<string, boolean>>({});
  messagesByChannel = $state<Record<string, Message[]>>({});
  messagesLoaded = $state<Record<string, boolean>>({});
  loadingOlder = $state<Record<string, boolean>>({});

  /* ---------- voice ---------- */
  myVoice = $state<MyVoice | null>(null);
  speaking = $state<Record<string, boolean>>({});
  /** Участники звонков в ЛС/группах: channel_id → голосовые состояния
   *  (VOICE_STATE_UPDATE с guild_id = null — как у оригинального клиента). */
  dmCalls = $state<Record<string, VoiceState[]>>({});

  /* ---------- ui ---------- */
  activeGuildId = $state<string | null>(null); // null → DM home
  activeChannelId = $state<string | null>(null);
  lastChannelByGuild = $state<Record<string, string>>({});
  showMembers = $state(true);
  showSettings = $state(false);
  unread = $state<Record<string, boolean>>({});
  /** Непрочитанные упоминания по каналам (счётчик для красного бейджа
   *  в рейле — как в оригинале). */
  mentions = $state<Record<string, number>>({});
  /** Быстрый переключатель (Ctrl+K): открыт/закрыт. */
  showQuickSwitcher = $state(false);
  typingByChannel = $state<Record<string, Record<string, TypingEntry>>>({});
  drafts = $state<Record<string, string>>({});
  editing = $state<{ channelId: string; messageId: string } | null>(null);
  /** Ответ на сообщение: цель ответа (Esc/Отправка сбрасывают). */
  replyTo = $state<Message | null>(null);
  profileCard = $state<{ userId: string; guildId: string } | null>(null);
  /** Расширенные профили (GET /users/{id}/profile) — юзер-аккаунт. */
  profiles = $state<Record<string, UserProfile>>({});
  /** Проверка обновлений: не-null → есть свежий релиз на GitHub. */
  updateInfo = $state<UpdateInfo | null>(null);
  /** Стадия автообновления (для карточки внизу справа). */
  updatePhase = $state<'idle' | 'downloading' | 'ready' | 'error'>('idle');
  /** Прогресс загрузки 0..1 (0 = неизвестно/крутится полоска). */
  updateProgress = $state(0);
  /** Текст ошибки автообновления (updatePhase === 'error'). */
  updateError = $state('');
  /** Боковые панели чата: закреплённые / поиск. */
  showPins = $state(false);
  showSearch = $state(false);
  /** Свой статус (online/idle/dnd/invisible) — для пикера в юзер-панели. */
  myStatus = $state('online');
  /** Трансляции в голосовых каналах: stream_key → инфо (свои и чужие). */
  streams = $state<Record<string, StreamInfo>>({});
  /** Локальное превью своего захвата (экран/камера) — MediaStream webview. */
  localPreview: MediaStream | null = $state(null);
  /** Тип активного собственного захвата для превью. */
  localPreviewKind = $state<'screen' | 'camera' | null>(null);
  /** Запрос «показать сообщение» от поиска: MessageList расширяет окно
   *  рендера до него и скроллит (nonce — повторные запросы того же id). */
  reveal = $state<{ id: string; nonce: number } | null>(null);
  toasts = $state<{ id: number; text: string; kind: 'info' | 'error' }[]>([]);

  private toastSeq = 1;
  private typingTimers: Record<string, ReturnType<typeof setTimeout>> = {};
  private speakingTimers: Record<string, ReturnType<typeof setTimeout>> = {};
  private lastTypingSent: Record<string, number> = {};

  /* ================= derived ================= */

  get activeGuild(): Guild | null {
    return this.guilds.find((g) => g.id === this.activeGuildId) ?? null;
  }

  /** Элементы рейла в порядке пользователя. Если раскладка ещё не приехала
   *  (или её нет) — все гильдии по одному, в порядке this.guilds. */
  get railItems(): RailItem[] {
    if (!this.railOrder.length && !this.guildFolders.length) {
      return this.guilds.map((g) => ({ type: 'guild', id: g.id }) as RailItem);
    }
    const out: RailItem[] = [];
    const inLayout = new Set<string>();
    for (const item of this.railOrder) {
      if (item.type === 'guild') {
        inLayout.add(item.id);
        if (this.guilds.some((g) => g.id === item.id)) out.push(item);
      } else {
        const f = this.guildFolders[item.index];
        if (f) out.push(item);
      }
    }
    // Гильдии вне раскладки (вступил после запуска) — в конец.
    for (const g of this.guilds) {
      if (!inLayout.has(g.id) && !this.guildFolders.some((f) => f.guild_ids.includes(g.id))) {
        out.push({ type: 'guild', id: g.id });
      }
    }
    return out;
  }

  /** Счётчик непрочитанных упоминаний гильдии (для красного бейджа). */
  mentionCountOfGuild(guildId: string): number {
    const g = this.guilds.find((x) => x.id === guildId);
    if (!g) return 0;
    let n = 0;
    for (const c of g.channels ?? []) n += this.mentions[c.id] ?? 0;
    return n;
  }

  /** Непрочитанные упоминания всех гильдий папки (бейдж на папке). */
  mentionCountOfFolder(index: number): number {
    const f = this.guildFolders[index];
    if (!f) return 0;
    return f.guild_ids.reduce((acc, gid) => acc + this.mentionCountOfGuild(gid), 0);
  }

  /** Любой непрочитанный (белая точка) в гильдии. */
  unreadOfGuild(guildId: string): boolean {
    const g = this.guilds.find((x) => x.id === guildId);
    return (g?.channels ?? []).some((c) => this.unread[c.id]);
  }

  unreadOfFolder(index: number): boolean {
    const f = this.guildFolders[index];
    if (!f) return false;
    return f.guild_ids.some((gid) => this.unreadOfGuild(gid));
  }

  /** Бот-аккаунт? Управляет копирайтом и лендингом после входа. */
  get isBotAccount(): boolean {
    return !!this.user?.bot;
  }

  /** Друзья, отсортированные по имени (type 1). */
  get friendsList(): Relationship[] {
    return Object.values(this.relationships)
      .filter((r) => r.type === 1)
      .sort((a, b) => displayName(a.user).localeCompare(displayName(b.user), 'ru'));
  }

  /** Входящие заявки в друзья (type 3) — ждут нашего ответа. */
  get incomingReqs(): Relationship[] {
    return Object.values(this.relationships)
      .filter((r) => r.type === 3)
      .sort((a, b) => displayName(a.user).localeCompare(displayName(b.user), 'ru'));
  }

  /** Исходящие заявки (type 4) — ждём ответа собеседника. */
  get outgoingReqs(): Relationship[] {
    return Object.values(this.relationships)
      .filter((r) => r.type === 4)
      .sort((a, b) => displayName(a.user).localeCompare(displayName(b.user), 'ru'));
  }

  /** Блок-лист (type 2). */
  get blockedList(): Relationship[] {
    return Object.values(this.relationships)
      .filter((r) => r.type === 2)
      .sort((a, b) => displayName(a.user).localeCompare(displayName(b.user), 'ru'));
  }

  /** DM-каналы как в оригинале: свежая переписка сверху (по last_message_id). */
  get dmRows(): Channel[] {
    return [...this.dms].sort((a, b) => (b.last_message_id ?? '0').localeCompare(a.last_message_id ?? '0'));
  }

  get activeChannel(): Channel | null {
    if (!this.activeChannelId) return null;
    const fromGuild = this.activeGuild?.channels?.find((c) => c.id === this.activeChannelId);
    if (fromGuild) return fromGuild;
    // Треды живут отдельно от каналов — выбираемая «ветка» тоже канал.
    const fromThreads = this.activeGuild?.threads?.find((c) => c.id === this.activeChannelId);
    if (fromThreads) return fromThreads;
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

  /** Активные (не в архиве) треды активной гильдии — под родительскими
   *  каналами и в списке постов форума. */
  get activeGuildThreads(): Channel[] {
    return (this.activeGuild?.threads ?? []).filter(
      (t) => (t.type === 11 || t.type === 12) && !t.thread_metadata?.archived,
    );
  }

  /** Открыт форум (type 15) — вместо ленты сообщений рисуем список постов. */
  get isForumView(): boolean {
    return this.activeChannel?.type === 15;
  }

  /** Треды канала (для сайдбара) — не форумные посты. */
  threadsOf(channelId: string): Channel[] {
    return this.activeGuildThreads.filter((t) => t.parent_id === channelId);
  }

  /** Посты форума (type 15). */
  forumPosts(forumId: string): Channel[] {
    return this.activeGuildThreads.filter((t) => t.parent_id === forumId);
  }

  get channelTitle(): string {
    const ch = this.activeChannel;
    if (!ch) return '';
    if (ch.type === 1 || ch.type === 3) {
      return this.dmRecipients(ch)
        .map((r) => displayName(r))
        .join(', ') || 'Личные сообщения';
    }
    return ch.name ?? '';
  }

  /** Участники ЛС/группы: recipients (REST) либо recipient_ids → usersById
   *  (юзер-READY отдаёт каналы с айдишниками, полные объекты — отдельно). */
  dmRecipients(ch: Channel): User[] {
    if (ch.recipients?.length) return ch.recipients;
    const ids = ch.recipient_ids ?? [];
    const users = ids.map((id) => this.usersById[id]).filter(Boolean) as User[];
    if (users.length) return users;
    // фолбэк: друзья и кэш сообщений знают этих людей
    return ids
      .map((id) => this.relationships[id]?.user ?? null)
      .filter(Boolean) as User[];
  }

  /** Насытить DM-канал recipients из usersById — один раз при входе в кэш. */
  private hydrateDm(c: Channel): Channel {
    if (c.recipients?.length) return c;
    const users = this.dmRecipients(c);
    if (users.length) return { ...c, recipients: users };
    return c;
  }

  /* ================= lifecycle ================= */

  async boot() {
    if (this.booted) return;
    this.booted = true;
    this.loadRecentEmoji();
    // Проверка обновлений при старте: свежий релиз на GitHub → уведомление.
    // Не блокирует загрузку и молчит при проблемах сети.
    void this.checkForUpdate();
    const token = localStorage.getItem(TOKEN_KEY);
    if (token && detectTauri()) {
      await this.login(token, true);
    }
  }

  /** Проверить обновление: сначала подписанный latest.json (plugin-updater,
   *  умеет ставить «на место»), при неудаче — GitHub API (баннер со ссылкой).
   *  Не блокирует загрузку и молчит при проблемах сети. */
  async checkForUpdate() {
    if (!detectTauri()) return;
    const seenKey = 'rustycord.seenUpdate';
    const backend = await createBackend('tauri');
    const canAuto = await backend.canAutoUpdate();
    let resolved = false;
    // 1) Основной путь: tauri-plugin-updater (latest.json из релиза,
    //    подпись проверяется против pubkey в tauri.conf.json).
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const upd = await check();
      if (upd?.available) {
        resolved = true;
        pendingUpdate = upd;
        this.updateInfo = {
          current: upd.currentVersion,
          latest: upd.version,
          is_newer: true,
          url: 'https://github.com/VinylRUS/Rustycord/releases/latest',
          notes: (upd.body ?? '').slice(0, 400),
          published_at: upd.date ? new Date(upd.date).toISOString() : '',
          no_releases: false,
          can_auto: canAuto,
          source: 'updater',
        };
        if (localStorage.getItem(seenKey) !== upd.version) {
          this.toast(`Доступна версия ${upd.version} — карточка внизу справа`, 'info');
        }
      }
    } catch {
      /* манифеста нет (старые релизы / нет сети) — пробуем API ниже */
    }
    if (resolved) return;
    // 2) Фолбэк: GitHub API. Баннер без авто-установки — так обновляются
    //    deb/rpm-сборки и клиенты на релизах без latest.json.
    try {
      const info = await backend.checkUpdate();
      if (!info || !info.is_newer) return;
      this.updateInfo = { ...info, can_auto: false, source: 'github' };
      if (localStorage.getItem(seenKey) !== info.latest) {
        this.toast(`Доступна версия ${info.latest} — карточка внизу справа`, 'info');
      }
    } catch {
      /* тихо — проверка обновлений не критична */
    }
  }

  /** Скачать и установить обновление «на место» (plugin-updater). */
  async installUpdate() {
    if (!pendingUpdate) return;
    this.updatePhase = 'downloading';
    this.updateProgress = 0;
    this.updateError = '';
    try {
      let total = 0;
      let downloaded = 0;
      await pendingUpdate.downloadAndInstall((ev) => {
        switch (ev.event) {
          case 'Started':
            total = ev.data.contentLength ?? 0;
            break;
          case 'Progress':
            downloaded += ev.data.chunkLength;
            if (total > 0) this.updateProgress = Math.min(1, downloaded / total);
            break;
          case 'Finished':
            this.updateProgress = 1;
            break;
        }
      });
      // На Windows NSIS-установщик может перезапустить приложение сам —
      // если процесс пережил установку, предлагаем кнопку «Перезапустить».
      this.updatePhase = 'ready';
    } catch (e) {
      this.updatePhase = 'error';
      this.updateError = String(e);
    }
  }

  /** Перезапустить приложение после установки обновления. */
  async relaunchApp() {
    const { relaunch } = await import('@tauri-apps/plugin-process');
    await relaunch();
  }

  /** Скрыть карточку обновления (тост для этой версии больше не покажем). */
  dismissUpdate() {
    this.markUpdateSeen();
    this.updateInfo = null;
    this.updatePhase = 'idle';
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
      void plugins.loadAll();
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

  /* ---------- user auth (v0.3.0) ---------- */

  /** Бэкенд для экрана логина: Tauri если запущены в приложении, mock в браузере. */
  private async ensureBackend(): Promise<Backend> {
    if (!this.backend) {
      const backend = await createBackend(detectTauri() ? 'tauri' : 'mock');
      this.attach(backend);
    }
    return this.backend!;
  }

  /** Общий финал всех трёх способов входа: есть токен → connect. */
  async finishLogin(token: string) {
    this.qrStage = 'idle';
    this.qrSvg = '';
    this.qrFingerprint = '';
    this.qrScannedBy = '';
    this.mfaTicket = null;
    if (this.mode === 'mock') {
      // Демо-режим: моковый QR «выдал» токен — просто стартуем демо-сессию.
      await this.startDemo();
      return;
    }
    await this.login(token);
  }

  /** Шаг 1: email+пароль. Может вернуть mfaTicket — тогда ждём код. */
  async loginEmail(email: string, password: string) {
    const backend = await this.ensureBackend();
    this.loginBusy = true;
    this.loginError = '';
    try {
      const res = await backend.loginEmail(email, password);
      if (res.status === 'ok' && res.token) {
        await this.finishLogin(res.token);
      } else if (res.status === 'mfa' && res.ticket) {
        this.mfaTicket = res.ticket;
      }
    } catch (e) {
      this.loginError = e instanceof Error ? e.message : String(e);
    } finally {
      this.loginBusy = false;
    }
  }

  /** Шаг 2: код из приложения-аутентификатора. */
  async submitMfa(code: string) {
    if (!this.backend || !this.mfaTicket) return;
    this.loginBusy = true;
    this.loginError = '';
    try {
      const res = await this.backend.submitMfa(code.trim(), this.mfaTicket);
      if (res.status === 'ok' && res.token) {
        await this.finishLogin(res.token);
      }
    } catch (e) {
      this.loginError = e instanceof Error ? e.message : String(e);
    } finally {
      this.loginBusy = false;
    }
  }

  /** QR-логин: стартуем сессию, дальнейшее — через события qr. */
  async startQr() {
    const backend = await this.ensureBackend();
    // Оптимистично показываем «подключаемся», чтобы не мигал плейсхолдер.
    this.qrStage = 'connecting';
    this.qrSvg = '';
    this.qrFingerprint = '';
    this.qrScannedBy = '';
    this.loginError = '';
    try {
      await backend.startQr();
    } catch (e) {
      this.loginError = e instanceof Error ? e.message : String(e);
    }
  }

  async cancelQr() {
    try {
      await this.backend?.cancelQr();
    } catch {
      /* silent */
    }
    this.qrStage = 'idle';
    this.qrSvg = '';
  }

  async startDemo() {
    this.loginBusy = true;
    this.loginError = '';
    try {
      const backend = await createBackend('mock');
      this.attach(backend);
      const user = await backend.connect('demo');
      this.user = user;
      void plugins.loadAll();
    } catch (e) {
      this.loginError = e instanceof Error ? e.message : String(e);
      this.detach();
    } finally {
      this.loginBusy = false;
    }
  }

  async logout() {
    localStorage.removeItem(TOKEN_KEY);
    await plugins.stopAll();
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
    plugins.attach(backend);
    this.unlisten = backend.onEvent((ev) => this.handleEvent(ev));
    this.unlistenToast = onPluginToast((text) => this.toast(text));
  }

  private detach() {
    this.unlisten?.();
    this.unlisten = null;
    this.unlistenToast?.();
    this.unlistenToast = null;
    this.backend = null;
  }

  private reset() {
    this.user = null;
    this.guilds = [];
    this.guildFolders = [];
    this.railOrder = [];
    this.guildOrder = {};
    this.dms = [];
    this.usersById = {};
    this.relationships = {};
    this.friendPresences = {};
    this.membersByGuild = {};
    this.presencesByGuild = {};
    this.voiceByGuild = {};
    this.memberLists = {};
    this.chunksDone = {};
    this.messagesByChannel = {};
    this.messagesLoaded = {};
    this.activeGuildId = null;
    this.activeChannelId = null;
    this.myVoice = null;
    this.speaking = {};
    this.dmCalls = {};
    this.streams = {};
    this.unread = {};
    this.mentions = {};
    this.showQuickSwitcher = false;
    this.connStage = 'offline';
    this.connDetail = '';
    this.replyTo = null;
    this.editing = null;
    this.profiles = {};
    this.showPins = false;
    this.showSearch = false;
    this.mode = 'none';
    this.showSettings = false;
    this.showGuildSettings = null;
    this.qrStage = 'idle';
    this.qrSvg = '';
    this.qrFingerprint = '';
    this.qrScannedBy = '';
    this.mfaTicket = null;
  }

  /* ================= event stream ================= */

  handleEvent(ev: import('../types').BackendEvent) {
    switch (ev.type) {
      case 'status':
        this.connStage = ev.stage;
        // Детали — только пополняются: повторный статус без detail не затирает
        // прошлую причину обрыва (иначе баннер мигает «без причины»).
        if ('detail' in ev && (ev as { detail?: string }).detail) {
          this.connDetail = (ev as { detail?: string }).detail ?? '';
        }
        if (ev.stage === 'connected') this.connDetail = '';
        if (ev.stage === 'disconnected' && !this.user) {
          this.loginError = this.connDetail || 'Соединение потеряно';
        }
        break;
      case 'ready':
        this.user = ev.user;
        this.sessionId = ev.session_id;
        plugins.emit('ready', ev.user);
        // Юзер-аккаунт: тихая REST-синхронизация друзей как страховка
        // (READY-dispatch обычно уже всё принёс, но после RESUME массив
        // отношений мог не прийти повторно).
        if (!ev.user.bot) void this.syncRelationships();
        // Переподключение: сервер забывает подписки op 37 (ленивые списки
        // участников) — переигрываем выбор гильдии/канала, как
        // CONNECTION_OPEN → flush подписок в официальном клиенте
        // (Rust параллельно переигрывает свои из кэша member_lists).
        if (!ev.user.bot && this.activeGuildId && this.mode === 'tauri') {
          this.selectGuild(this.activeGuildId);
        }
        break;
      case 'memberList': {
        // Снапшот ленивого списка (GUILD_MEMBER_LIST_UPDATE). Помимо рендера
        // сайдбара — гидратация кэшей: статусы и карточки участников отсюда
        // видны профиль-карточка, ЛС и «кто в сети» (то, что в официальном
        // клиенте делают UserStore/PresenceStore из member-list ops).
        this.memberLists[ev.guild_id] = {
          channel_id: ev.channel_id,
          member_count: ev.member_count,
          online_count: ev.online_count,
          rows: ev.rows,
        };
        const members = this.membersByGuild[ev.guild_id] ?? {};
        const presences = this.presencesByGuild[ev.guild_id] ?? {};
        for (const row of ev.rows) {
          if (row.kind !== 'member') continue;
          presences[row.user_id] = {
            user: {
              id: row.user_id,
              username: row.username,
              discriminator: row.discriminator ?? '0',
              global_name: row.global_name ?? null,
              avatar: row.avatar ?? null,
            },
            status: (row.status as Presence['status']) || 'offline',
            activities: (row.activities ?? []) as never,
          };
          if (!members[row.user_id]) {
            members[row.user_id] = {
              user: {
                id: row.user_id,
                username: row.username,
                discriminator: row.discriminator ?? '0',
                global_name: row.global_name ?? null,
                avatar: row.avatar ?? null,
                bot: row.bot ?? false,
              },
              nick: row.nick ?? null,
              roles: row.roles,
            };
          }
        }
        this.membersByGuild[ev.guild_id] = members;
        this.presencesByGuild[ev.guild_id] = presences;
        break;
      }
      case 'dispatch':
        this.handleDispatch(ev.t, ev.d);
        break;
      case 'guildLayout':
        this.applyGuildLayout(ev.folders, ev.guild_positions, ev.rail_order ?? []);
        break;
      case 'qr':
        this.handleQr(ev);
        break;
      case 'voice':
        this.handleVoice(ev.kind, ev.data);
        break;
      case 'warning':
        this.toast(ev.message, 'error');
        break;
    }
  }

  /** Раскладка рейла из user_settings (событие guildLayout от Rust:
   *  proto-декодер или legacy-JSON, единая нормализованная схема).
   *  Сортируем this.guilds по плоскому порядку и пересобираем рейл.
   *  Вызывается до dispatch-READY (событие эмитится раньше) — тогда просто
   *  запоминаем раскладку, а стабы READY отсортируются по ней на приходе. */
  applyGuildLayout(
    folders: import('../types').GuildFolder[],
    guildPositions: string[],
    railOrder: RailOrderItem[],
  ) {
    // Нормализация: дубликаты id внутри папки и в rail_order роняют
    // keyed-each в ServerRail (each_key_duplicate замораживает DOM —
    // клик по папке «не работает»). Дедупим с сохранением порядка.
    const seenFolderIds = new Set<string>();
    this.guildFolders = folders.map((f) => ({
      ...f,
      guild_ids: f.guild_ids.filter((gid) => {
        if (seenFolderIds.has(`${f.id ?? '_'}:${gid}`)) return false;
        seenFolderIds.add(`${f.id ?? '_'}:${gid}`);
        return true;
      }),
    }));
    const seenRail = new Set<string>();
    this.railOrder = railOrder.filter((item) => {
      const key = item.type === 'folder' ? `f${item.index}` : item.id;
      if (seenRail.has(key)) return false;
      seenRail.add(key);
      return true;
    });
    // Плоский порядок: сначала отдельные гильдии из railOrder, затем
    // гильдии папок (папки хранят порядок внутри себя).
    const order: Record<string, number> = {};
    let i = 0;
    const seen = new Set<string>();
    for (const item of railOrder) {
      if (item.type === 'guild' && !seen.has(item.id)) {
        order[item.id] = i++;
        seen.add(item.id);
      }
    }
    for (const f of folders) {
      for (const gid of f.guild_ids) {
        if (!seen.has(gid)) {
          order[gid] = i++;
          seen.add(gid);
        }
      }
    }
    for (const gid of guildPositions) {
      if (!seen.has(gid)) {
        order[gid] = i++;
        seen.add(gid);
      }
    }
    this.guildOrder = order;
    if (Object.keys(order).length) {
      this.guilds = [...this.guilds].sort(
        (a, b) => (order[a.id] ?? 1e9) - (order[b.id] ?? 1e9),
      );
    }
  }

  /* ---------- QR remote-auth поток ---------- */

  handleQr(ev: { stage: QrStage; svg?: string; fingerprint?: string; token?: string; username?: string; userId?: string; message?: string }) {
    this.qrStage = ev.stage;
    switch (ev.stage) {
      case 'connecting':
        break; // спиннер «Подключаемся к Discord…»
      case 'fingerprint':
        this.qrSvg = ev.svg ?? '';
        this.qrFingerprint = ev.fingerprint ?? '';
        break;
      case 'scanned':
        // pending_ticket: сервер сообщил, кто сканирует — показываем это.
        this.qrScannedBy = ev.username ?? '';
        break; // просто индикатор «подтверди на телефоне»
      case 'done':
        if (ev.token) void this.finishLogin(ev.token);
        break;
      case 'cancelled':
      case 'timeout':
      case 'error':
        this.loginError = ev.message ?? 'QR-сессия прервана';
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
      const where = data.ip ? ` · ${data.ip}:${data.port}` : '';
      this.toast(`Голос подключён · ${data.mode ?? 'opus'}${where}`, 'info');
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
    // Все сырые события — плагинам (как Vencord прокидывает dispatch).
    plugins.emit('dispatch', { t, d });
    switch (t) {
      case 'READY': {
        const p = d as {
          user: User;
          session_id: string;
          users?: User[];
          private_channels?: Channel[];
          relationships?: Relationship[];
          presences?: Presence[];
          guilds?: { id: string; name?: string; icon?: string | null }[];
          guild_folders?: { guild_ids?: string[] }[];
          merged_presences?: { friends?: Presence[]; guilds?: Presence[][] };
          merged_members?: Member[][];
        };
        this.user = p.user;
        this.sessionId = p.session_id;
        // Юзер-READY несёт полный справочник участников (для recipient_ids).
        if (p.users) {
          const map = { ...this.usersById };
          for (const u of p.users) map[u.id] = u;
          this.usersById = map;
        }
        // Юзер-READY несёт DM-каналы, друзей и их статусы сразу — Rust
        // теперь пробрасывает полный payload (раньше отношения и ЛС
        // терялись между ядром и UI, и главная выглядела «пустой»).
        if (p.private_channels) {
          const seen = new Set(this.dms.map((x) => x.id));
          for (const c of p.private_channels) {
            if (!seen.has(c.id)) {
              this.dms.push(this.hydrateDm(c));
              seen.add(c.id);
            }
          }
        }
        if (p.relationships) {
          const map = { ...this.relationships };
          for (const r of p.relationships) map[r.user.id] = r;
          this.relationships = map;
        }
        if (p.presences) {
          // В READY это статусы друзей — без guild_id.
          const map = { ...this.friendPresences };
          for (const pr of p.presences) {
            if (!('guild_id' in pr) || (pr as { guild_id?: string }).guild_id == null) {
              map[pr.user.id] = pr;
            }
          }
          this.friendPresences = map;
        }
        // Гильдии-стабы из READY (id+name+icon): рейл серверов рисуется
        // СРАЗУ — GUILD_CREATE (после op 37-подписки) наполнит каналы
        // и участников, заменив стабы. Так ведёт себя официальный клиент.
        if (p.guilds) {
          const known = new Set(this.guilds.map((g) => g.id));
          for (const g of p.guilds) {
            if (g?.id && !known.has(g.id)) {
              this.guilds.push({
                id: g.id,
                name: g.name ?? '…',
                icon: g.icon ?? null,
                owner_id: '',
                roles: [],
                channels: [],
              } as Guild);
            }
          }
        }
        // Порядок серверов в рейле — событие guildLayout (Rust декодирует
        // user_settings_proto/legacy ДО этого dispatch'а). Если раскладка
        // уже приехала — сортируем стабы по ней; иначе гильдии встанут по
        // прибытию (бот-сессии/демо — папок нет).
        if (Object.keys(this.guildOrder).length) {
          const order = this.guildOrder;
          this.guilds = [...this.guilds].sort(
            (a, b) => (order[a.id] ?? 1e9) - (order[b.id] ?? 1e9),
          );
        }
        this.applyMerged(p.guilds ?? [], p.merged_presences, p.merged_members);
        // Надёжный источник имён/иконок юзер-гильдий: REST-эндпоинт
        // всегда несёт name+icon (gateway-пейлоады юзер-сессий — нет:
        // стабы READY бывают безымянными, GUILD_CREATE держит имя в
        // properties.name). Мержим поверх стабов.
        void this.fetchUserGuilds();
        break;
      }
      case 'READY_SUPPLEMENTAL': {
        // Едет сразу после потока GUILD_CREATE: голосовые состояния
        // гильдий + merged-массивы (статусы друзей и участников).
        const p = d as {
          guilds?: { id: string; voice_states?: VoiceState[] }[];
          merged_presences?: { friends?: Presence[]; guilds?: Presence[][] };
          merged_members?: Member[][];
        };
        for (const g of p.guilds ?? []) {
          this.voiceByGuild[g.id] = g.voice_states ?? [];
        }
        this.applyMerged(p.guilds ?? [], p.merged_presences, p.merged_members);
        break;
      }
      case 'GUILD_CREATE': {
        // Merge, а не replace: юзер-сессии шлют GUILD_CREATE без
        // верхнеуровневого name/icon — если затирать стаб из READY,
        // гильдия останется безымянной (а ServerRail рисует инициалы
        // из name — undefined ронял весь рейл серверов целиком).
        const raw = d as Partial<Guild> & { id: string };
        const existing = this.guilds.find((x) => x.id === raw.id);
        const g = {
          ...existing,
          ...raw,
          name: raw.name ?? existing?.name ?? 'Без названия',
          icon: raw.icon ?? existing?.icon ?? null,
        } as Guild;
        // Замена НА МЕСТЕ (а не в конец): порядок рейла задаётся guild_folders
        // из READY — перенос гильдии в хвост ломал бы сортировку на каждом
        // GUILD_CREATE. Новые гильдии (вступил после запуска) — в конец.
        const gi = this.guilds.findIndex((x) => x.id === g.id);
        if (gi >= 0) this.guilds[gi] = g;
        else this.guilds.push(g);
        plugins.emit('guild:create', g);
        // seed caches
        const members: Record<string, Member> = {};
        for (const m of g.members ?? []) if (m.user) members[m.user.id] = m;
        this.membersByGuild[g.id] = { ...(this.membersByGuild[g.id] ?? {}), ...members };
        const pres: Record<string, Presence> = {};
        for (const p of g.presences ?? []) pres[p.user.id] = p;
        this.presencesByGuild[g.id] = pres;
        this.voiceByGuild[g.id] = g.voice_states ?? [];
        // Каналы только что приехали (GUILD_CREATE после op 37-подписки).
        // Если это активная гильдия и канал ещё не выбран (кликали по стабу
        // без каналов) — автовыбор теперь возможен; он же подпишет список
        // участников текущего канала (см. selectChannel → subscribeChannel).
        if (g.id === this.activeGuildId && !this.isBotAccount && this.mode === 'tauri') {
          void this.backend?.subscribeGuild(g.id).catch(() => undefined);
          if (!this.activeChannelId) this.selectGuild(g.id);
        }
        // Автовыбор первой гильдии — только бот-аккаунтам (демо-режим).
        // Юзер после входа попадает на «Друзья», как в официальном клиенте.
        if (!this.activeGuildId && this.isBotAccount) this.selectGuild(g.id);
        break;
      }
      case 'GUILD_UPDATE': {
        const g = d as Guild;
        const idx = this.guilds.findIndex((x) => x.id === g.id);
        if (idx >= 0) this.guilds[idx] = { ...this.guilds[idx], ...g };
        plugins.emit('guild:update', g);
        break;
      }
      case 'GUILD_DELETE': {
        const g = d as { id: string };
        this.guilds = this.guilds.filter((x) => x.id !== g.id);
        if (this.activeGuildId === g.id) this.selectGuild(this.guilds[0]?.id ?? null);
        plugins.emit('guild:delete', g);
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
        const p = d as {
          guild_id: string;
          members: Member[];
          chunk_index: number;
          chunk_count: number;
          presences?: Presence[];
        };
        const map = { ...(this.membersByGuild[p.guild_id] ?? {}) };
        for (const m of p.members) if (m.user) map[m.user.id] = m;
        this.membersByGuild[p.guild_id] = map;
        // Ответ на op 14 несёт и пресенсы (activities: true) — без них
        // список участников выглядит «все офлайн».
        if (p.presences?.length) {
          const pmap = { ...(this.presencesByGuild[p.guild_id] ?? {}) };
          for (const pr of p.presences) pmap[pr.user.id] = pr;
          this.presencesByGuild[p.guild_id] = pmap;
        }
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
          if (!this.dms.some((x) => x.id === c.id)) this.dms.push(this.hydrateDm(c));
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
      /* ---------- треды и форумы (v0.8.0) ---------- */
      case 'THREAD_CREATE': {
        const t = d as Channel;
        const g = this.guilds.find((x) => x.id === t.guild_id);
        if (g) g.threads = [...(g.threads ?? []).filter((x) => x.id !== t.id), t];
        break;
      }
      case 'THREAD_UPDATE': {
        const t = d as Channel;
        const g = this.guilds.find((x) => x.id === t.guild_id);
        if (g) g.threads = (g.threads ?? []).map((x) => (x.id === t.id ? { ...x, ...t } : x));
        break;
      }
      case 'THREAD_DELETE': {
        const p = d as { id: string; guild_id: string };
        const g = this.guilds.find((x) => x.id === p.guild_id);
        if (g) g.threads = (g.threads ?? []).filter((x) => x.id !== p.id);
        if (this.activeChannelId === p.id) this.selectChannel(null);
        break;
      }
      case 'THREAD_LIST_SYNC': {
        const p = d as { guild_id: string; threads?: Channel[] };
        const g = this.guilds.find((x) => x.id === p.guild_id);
        if (g && p.threads?.length) {
          const map = new Map((g.threads ?? []).map((t) => [t.id, t]));
          for (const t of p.threads) map.set(t.id, t);
          g.threads = [...map.values()];
        }
        break;
      }
      case 'USER_UPDATE': {
        const u = d as User;
        if (u.id === this.user?.id) this.user = { ...this.user, ...u };
        break;
      }
      case 'MESSAGE_CREATE': {
        const m = d as Message;
        this.appendMessage(m.channel_id, m);
        if (m.channel_id !== this.activeChannelId) {
          this.unread[m.channel_id] = true;
          // Красный бейдж: упоминание нас (в тексте или @everyone/@here —
          // их Discord тоже считает упоминанием: у сообщения mentions
          // содержит нашу карточку и в этом случае).
          if (m.mentions?.some((u) => u.id === this.user?.id)) {
            this.mentions[m.channel_id] = (this.mentions[m.channel_id] ?? 0) + 1;
          } else if (m.mention_everyone) {
            this.mentions[m.channel_id] = (this.mentions[m.channel_id] ?? 0) + 1;
          }
        }
        this.clearTyping(m.channel_id, m.author.id);
        // Упомянутые юзеры приходят полными объектами — пополняем справочник.
        if (m.mentions?.length) {
          const map = { ...this.usersById };
          let changed = false;
          for (const u of m.mentions)
            if (!map[u.id]) {
              map[u.id] = u;
              changed = true;
            }
          if (changed) this.usersById = map;
        }
        plugins.emit('message:create', m);
        break;
      }
      case 'MESSAGE_UPDATE': {
        const m = d as Message;
        const list = this.messagesByChannel[m.channel_id];
        if (list) {
          const i = list.findIndex((x) => x.id === m.id);
          if (i >= 0) list[i] = { ...list[i], ...m };
        }
        plugins.emit('message:update', m);
        break;
      }
      case 'MESSAGE_DELETE': {
        const p = d as { channel_id: string; id: string };
        const list = this.messagesByChannel[p.channel_id];
        if (list) this.messagesByChannel[p.channel_id] = list.filter((m) => m.id !== p.id);
        plugins.emit('message:delete', p);
        break;
      }
      case 'MESSAGE_REACTION_ADD':
      case 'MESSAGE_REACTION_REMOVE': {
        const p = d as {
          channel_id: string;
          message_id: string;
          user_id: string;
          emoji: { id: string | null; name: string | null; animated?: boolean };
        };
        const list = this.messagesByChannel[p.channel_id];
        const m = list?.find((x) => x.id === p.message_id);
        if (!m) break;
        const reactions = [...(m.reactions ?? [])];
        const key = emojiKey;
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
      case 'MESSAGE_REACTION_REMOVE_ALL': {
        const p = d as { channel_id: string; message_id: string };
        const m = this.messagesByChannel[p.channel_id]?.find((x) => x.id === p.message_id);
        if (m) m.reactions = [];
        break;
      }
      case 'MESSAGE_REACTION_REMOVE_EMOJI': {
        const p = d as { channel_id: string; message_id: string; emoji: { id: string | null; name: string | null } };
        const m = this.messagesByChannel[p.channel_id]?.find((x) => x.id === p.message_id);
        if (m) m.reactions = (m.reactions ?? []).filter((r) => emojiKey(r.emoji) !== emojiKey(p.emoji));
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
        if ((p as { guild_id?: string }).guild_id == null) {
          // Статус друга (вне гильдии) — обновляем карту друзей.
          this.friendPresences = { ...this.friendPresences, [p.user.id]: p };
        }
        for (const gid of Object.keys(this.presencesByGuild)) {
          const map = this.presencesByGuild[gid];
          if (map[p.user.id] || this.membersByGuild[gid]?.[p.user.id]) {
            map[p.user.id] = p;
          }
        }
        break;
      }
      case 'RELATIONSHIP_ADD': {
        const r = d as Relationship;
        if (r?.user?.id) {
          this.relationships = { ...this.relationships, [r.user.id]: r };
        }
        break;
      }
      case 'RELATIONSHIP_REMOVE': {
        const p = d as { id: string; type?: number; nickname?: string };
        const map = { ...this.relationships };
        delete map[p.id];
        this.relationships = map;
        const pres = { ...this.friendPresences };
        delete pres[p.id];
        this.friendPresences = pres;
        break;
      }
      case 'VOICE_STATE_UPDATE': {
        const vs = d as VoiceState;
        plugins.emit('voice:state', vs);
        // Звонок в ЛС/группе: guild_id = null — участники едут в dmCalls.
        if (!vs.guild_id) {
          const map = { ...this.dmCalls };
          for (const key of Object.keys(map)) {
            map[key] = map[key].filter((v) => v.user_id !== vs.user_id);
          }
          if (vs.channel_id) {
            map[vs.channel_id] = [...(map[vs.channel_id] ?? []), vs];
          } else {
            delete map['']; // страховка от мусорного ключа
          }
          this.dmCalls = map;
          if (vs.user_id === this.user?.id) {
            if (vs.channel_id) {
              this.myVoice = {
                guildId: '',
                channelId: vs.channel_id,
                selfMute: !!vs.self_mute,
                selfDeaf: !!vs.self_deaf,
                selfVideo: !!vs.self_video,
              };
            } else {
              this.myVoice = null;
            }
          }
          break;
        }
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
              selfVideo: !!vs.self_video,
            };
          } else {
            this.myVoice = null;
          }
        }
        break;
      }
      case 'STREAM_CREATE': {
        // Трансляция начата (нами или другим участником канала).
        const s = d as {
          stream_key: string;
          guild_id?: string;
          channel_id?: string;
          user_id?: string;
          type?: string;
          viewer_count?: number;
          paused?: boolean;
        };
        if (s.stream_key) {
          this.streams = {
            ...this.streams,
            [s.stream_key]: {
              streamKey: s.stream_key,
              guildId: s.guild_id ?? '',
              channelId: s.channel_id ?? '',
              userId: s.user_id ?? '',
              type: s.type ?? 'screen',
              viewerCount: s.viewer_count ?? 0,
              paused: !!s.paused,
            },
          };
        }
        break;
      }
      case 'STREAM_UPDATE': {
        const s = d as { stream_key: string; paused?: boolean; viewer_count?: number };
        const cur = this.streams[s.stream_key];
        if (cur) {
          this.streams = {
            ...this.streams,
            [s.stream_key]: {
              ...cur,
              paused: s.paused ?? cur.paused,
              viewerCount: s.viewer_count ?? cur.viewerCount,
            },
          };
        }
        break;
      }
      case 'STREAM_DELETE': {
        const s = d as { stream_key: string };
        if (this.streams[s.stream_key]) {
          const map = { ...this.streams };
          delete map[s.stream_key];
          this.streams = map;
        }
        break;
      }
      default:
        // Unknown events are intentionally ignored — like the real client,
        // forward-compatibility comes from graceful skipping.
        break;
    }
  }

  /** Подтягиваем ПОЛНЫЙ список гильдий юзера через REST
   *  GET /users/@me/guilds: в отличие от gateway-пейлоадов юзер-сессий
   *  (безымянные стабы READY, GUILD_CREATE с именем в properties.name),
   *  этот эндпоинт всегда возвращает name и icon — это авторитетный
   *  источник для рейла серверов. Гильдии, которых ещё нет в кэше,
   *  ДОБАВЛЯЕМСЯ (union): стабы READY могут быть неполными — без этого
   *  часть серверов не отображалась вовсе. Новые гильдии без каналов
   *  подписываемся op 37 — GUILD_CREATE подвезёт каналы. */
  private async fetchUserGuilds(): Promise<void> {
    if (this.mode !== 'tauri' || this.isBotAccount) return;
    // До двух попыток с паузой: на старте сессии REST бывает занят/зарейт-
    // лимичен — без иконок и имён половина рейла повисает инициалами
    // («S», «WS» на скрине v0.9.5), а молчаливый catch это скрывал.
    for (let attempt = 1; attempt <= 2; attempt++) {
      try {
        const res = await this.backend?.apiRequest('GET', '/users/@me/guilds?limit=200');
        const list = res as
          | { id: string; name?: string; icon?: string | null; owner?: boolean }[]
          | undefined;
        if (!Array.isArray(list)) return;
        const fresh: string[] = [];
        for (const u of list) {
          if (!u?.id) continue;
          const idx = this.guilds.findIndex((g) => g.id === u.id);
          if (idx >= 0) {
            this.guilds[idx] = {
              ...this.guilds[idx],
              name: u.name || this.guilds[idx].name,
              icon: u.icon ?? this.guilds[idx].icon ?? null,
            };
          } else {
            // Сервер есть в аккаунте, но стаба READY не было — добавляем.
            this.guilds.push({
              id: u.id,
              name: u.name || '…',
              icon: u.icon ?? null,
              owner_id: '',
              roles: [],
              channels: [],
            } as Guild);
            fresh.push(u.id);
          }
        }
        // Порядок из guild_folders сохраняем, новинки — в конец.
        if (Object.keys(this.guildOrder).length) {
          this.guilds = [...this.guilds].sort(
            (a, b) => (this.guildOrder[a.id] ?? 1e9) - (this.guildOrder[b.id] ?? 1e9),
          );
        }
        // Только что добавленные гильдии без каналов: подписка op 37 —
        // по наблюдениям v0.5.0 юзер-сессии получают GUILD_CREATE на
        // подписанные гильдии.
        for (const gid of fresh) {
          void this.backend?.subscribeGuild(gid).catch(() => undefined);
        }
        return;
      } catch {
        // Не критично: имена доберутся из GUILD_CREATE (properties.name),
        // но даём REST второй шанс через паузу.
        if (attempt < 2) await new Promise((r) => setTimeout(r, 6000));
      }
    }
  }

  /** merged-массивы из READY / READY_SUPPLEMENTAL: статусы друзей и
   *  участников по гильдиям + собственные карточки участников.
   *  merged_presences.guilds[i] и merged_members[i] параллельны guilds[i]. */
  private applyMerged(
    guilds: { id: string }[],
    mergedPresences?: { friends?: Presence[]; guilds?: Presence[][] },
    mergedMembers?: Member[][],
  ) {
    if (mergedPresences?.friends?.length) {
      const map = { ...this.friendPresences };
      for (const pr of mergedPresences.friends) map[pr.user.id] = pr;
      this.friendPresences = map;
    }
    if (mergedPresences?.guilds) {
      for (let i = 0; i < mergedPresences.guilds.length; i++) {
        const gid = guilds[i]?.id;
        const list = mergedPresences.guilds[i];
        if (!gid || !list?.length) continue;
        const map = { ...(this.presencesByGuild[gid] ?? {}) };
        for (const pr of list) map[pr.user.id] = pr;
        this.presencesByGuild[gid] = map;
      }
    }
    if (mergedMembers) {
      for (let i = 0; i < mergedMembers.length; i++) {
        const gid = guilds[i]?.id;
        const list = mergedMembers[i];
        if (!gid || !list?.length) continue;
        const map = { ...(this.membersByGuild[gid] ?? {}) };
        for (const m of list) if (m.user) map[m.user.id] = m;
        this.membersByGuild[gid] = map;
      }
    }
  }

  /* ================= actions ================= */

  /** Прыжок к сообщению в списке (поиск/пины): MessageList расширит окно. */
  requestReveal(id: string) {
    this.reveal = { id, nonce: Date.now() };
  }

  selectGuild(id: string | null) {
    this.activeGuildId = id;
    const guild = this.guilds.find((g) => g.id === id);
    if (!guild) {
      this.activeChannelId = null;
      return;
    }
    // Юзер-аккаунт: подписка на события гильдии — при открытии (op 37 + op 14).
    // Без неё не приедут ни пресенсы участников, ни ленивый список участников.
    if (id && !this.isBotAccount && this.mode === 'tauri') {
      void this.backend?.subscribeGuild(id).catch(() => undefined);
    }
    // Выбор канала: только из ВИДИМЫХ текущей роли (скрытые каналы
    // пропускаем — как в оригинале).
    const remembered = id !== null ? this.lastChannelByGuild[id] : undefined;
    const channels = id !== null ? this.visibleChannels(id).filter((c) => c.type !== 4) : [];
    const target =
      channels.find((c) => c.id === remembered) ??
      channels.find((c) => c.type === 0) ??
      channels[0];
    this.selectChannel(target?.id ?? null);
  }

  selectChannel(id: string | null) {
    this.activeChannelId = id;
    // Ленивый список участников канала: op 37 с ОДНИМ каналом и стартовым
    // диапазоном [[0,99]] — на каждое переключение, как CHANNEL_SELECT →
    // subscribeChannel в официальном клиенте. Ответ —
    // GUILD_MEMBER_LIST_UPDATE (в демо — memberList из локального мира).
    if (id && this.activeGuildId && !this.isBotAccount && this.backend) {
      void this.backend
        ?.subscribeChannel(this.activeGuildId, id, [[0, 99]])
        .catch(() => undefined);
    }
    if (id) {
      delete this.unread[id];
      delete this.mentions[id];
      if (this.activeGuildId) this.lastChannelByGuild[this.activeGuildId] = id;
      if (!this.messagesLoaded[id]) void this.loadMessages(id);
    }
  }

  /** Расширить диапазоны ленивого списка при скролле сайдбара — 1:1 с
   *  onScroll → GUILD_SUBSCRIPTIONS_CHANNEL официального клиента: всегда
   *  [0,99] + сотни до видимой зоны + overscan (счёт диапазонов ведёт
   *  Rust в member_lists — replay после переподключения не теряет их). */
  subscribeMemberListRanges(ranges: [number, number][]) {
    const ch = this.activeChannelId;
    if (!ch || !this.activeGuildId || this.isBotAccount || !this.backend) return;
    void this.backend
      ?.subscribeChannel(this.activeGuildId, ch, ranges)
      .catch(() => undefined);
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
    // Хук плагинов: могут изменить текст или отменить отправку (вернуть false).
    const finalContent = await plugins.beforeSend(content);
    if (finalContent === false) {
      this.draft = '';
      return;
    }
    const text = (typeof finalContent === 'string' ? finalContent : content).trim();
    if (!text) return;
    this.draft = '';
    const replyTo = this.replyTo?.id ?? undefined;
    this.replyTo = null;
    try {
      const m = await this.backend.sendMessage(ch, text, replyTo);
      this.appendMessage(ch, m);
    } catch (e) {
      this.draft = text;
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

  /* ================= reply / attachments / pins / search (v0.6.0) ================= */

  /** Начать ответ на сообщение (баннер в инпуте, Esc — отмена). */
  startReply(m: Message) {
    this.editing = null;
    this.replyTo = m;
  }

  cancelReply() {
    this.replyTo = null;
  }

  /** Сменить свой статус (юзер-аккаунт; ботам недоступно). */
  async setStatus(status: 'online' | 'idle' | 'dnd' | 'invisible') {
    if (!this.backend || this.isBotAccount) return;
    const prev = this.myStatus;
    this.myStatus = status; // оптимистично — как оригинал
    try {
      await this.apiCall('PATCH', '/users/@me/settings', { status });
    } catch (e) {
      this.myStatus = prev;
      this.toast(`Статус не сменился: ${errMsg(e)}`, 'error');
    }
  }

  /** Закрепить/открепить сообщение. */
  async togglePin(m: Message) {
    try {
      if (m.pinned) await this.apiCall('DELETE', `/channels/${m.channel_id}/pins/${m.id}`);
      else await this.apiCall('PUT', `/channels/${m.channel_id}/pins/${m.id}`);
      m.pinned = !m.pinned;
      this.toast(m.pinned ? 'Сообщение закреплено' : 'Сообщение откреплено', 'info');
    } catch (e) {
      this.toast(`Не получилось: ${errMsg(e)}`, 'error');
    }
  }

  /** Список закреплённых (новые сверху). */
  async listPins(channelId: string): Promise<Message[]> {
    const list = (await this.apiCall('GET', `/channels/${channelId}/pins`)) as Message[];
    return list;
  }

  /** Поиск по каналу (юзер-аккаунт: тот же ендпоинт, что у оригинала). */
  async searchChannel(channelId: string, query: string): Promise<Message[]> {
    const raw = (await this.apiCall(
      'GET',
      `/channels/${channelId}/messages/search?query=${encodeURIComponent(query)}`,
    )) as { messages?: Message[][] };
    // Ответ — группы по 1 сообщению (аналогично оригиналу).
    return (raw.messages ?? []).map((gr) => gr[0]).filter(Boolean);
  }

  /** Расширенный профиль (баннер, био, общие сервера) — юзер-аккаунт. */
  async fetchProfile(userId: string): Promise<UserProfile | null> {
    if (this.profiles[userId]) return this.profiles[userId];
    if (this.isBotAccount) return null;
    try {
      const p = (await this.apiCall('GET', `/users/${userId}/profile`)) as UserProfile;
      this.profiles = { ...this.profiles, [userId]: p };
      return p;
    } catch {
      return null; // у ботов и приватных профилей — тихо
    }
  }

  /** Диалог выбора файлов → загрузка в активный канал (multipart). */
  async uploadAttachments() {
    const ch = this.activeChannelId;
    if (!ch || !this.backend) return;
    if (this.mode === 'mock') {
      this.toast('В демо-режиме загрузка файлов недоступна', 'error');
      return;
    }
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ multiple: true, title: 'Прикрепить файлы' });
      if (!selected || (Array.isArray(selected) && !selected.length)) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      const content = this.draft.trim() || undefined;
      this.draft = '';
      const m = await this.backend.uploadFiles(ch, paths, content);
      this.appendMessage(ch, m);
    } catch (e) {
      this.toast(`Загрузка не удалась: ${errMsg(e)}`, 'error');
    }
  }

  /** Открыть страницу релиза во внешнем браузере. */
  async openReleaseUrl() {
    if (!this.updateInfo) return;
    try {
      const { openUrl } = await import('@tauri-apps/plugin-opener');
      await openUrl(this.updateInfo.url);
    } catch {
      window.open(this.updateInfo.url, '_blank');
    }
  }

  /** Пометить «обновление видано» — плашку больше не показываем для этой версии. */
  markUpdateSeen() {
    if (this.updateInfo) localStorage.setItem('rustycord.seenUpdate', this.updateInfo.latest);
  }

  async deleteMessage(message: Message) {
    if (!this.backend) return;
    try {
      await this.backend.deleteMessage(message.channel_id, message.id);
    } catch (e) {
      this.toast(`Не удалось удалить: ${errMsg(e)}`, 'error');
    }
  }

  /** Недавние эмодзи для быстрого доступа в пикере (localStorage). */
  recentEmoji = $state<{ id: string | null; name: string; animated?: boolean }[]>([]);

  loadRecentEmoji() {
    try {
      const raw = localStorage.getItem(RECENT_EMOJI_KEY);
      if (raw) this.recentEmoji = JSON.parse(raw);
    } catch {
      /* пусто — ну и ладно */
    }
  }

  private pushRecentEmoji(e: { id: string | null; name: string; animated?: boolean }) {
    const list = [e, ...this.recentEmoji.filter((x) => emojiKey(x) !== emojiKey(e))].slice(
      0,
      RECENT_EMOJI_MAX,
    );
    this.recentEmoji = list;
    try {
      localStorage.setItem(RECENT_EMOJI_KEY, JSON.stringify(list));
    } catch {
      /* приватный режим — ну и ладно */
    }
  }

  /** Тогл реакции: стоит у нас — снимаем, не стоит — ставим. */
  async react(message: Message, emoji: { id: string | null; name: string | null; animated?: boolean }) {
    if (!this.backend || !emoji.name) return;
    const existing = (message.reactions ?? []).find(
      (r) => r.me && emojiKey(r.emoji) === emojiKey(emoji),
    );
    const api = emojiApi(emoji);
    try {
      if (existing) await this.backend.removeReaction(message.channel_id, message.id, api);
      else {
        await this.backend.addReaction(message.channel_id, message.id, api);
        this.pushRecentEmoji(emoji as { id: string | null; name: string; animated?: boolean });
      }
    } catch (e) {
      this.toast(`Реакция не удалась: ${errMsg(e)}`, 'error');
    }
  }

  async typingPing() {
    const ch = this.activeChannelId;
    if (!ch || !this.backend) return;
    // Плагины (например, Silent Typing) могут подавить индикатор.
    if (plugins.suppressions['typing']) return;
    const now = Date.now();
    if (now - (this.lastTypingSent[ch] ?? 0) < 7000) return;
    this.lastTypingSent[ch] = now;
    try {
      await this.backend.triggerTyping(ch);
    } catch {
      /* silent */
    }
  }

  /** Кто поставил реакцию: GET /channels/{cid}/messages/{mid}/reactions/{emoji}
   *  — список юзеров для поповера над чипом реакции. */
  async listReactionUsers(channelId: string, messageId: string, emoji: { id: string | null; name: string | null }): Promise<User[]> {
    try {
      const api = emojiApi(emoji);
      const res = await this.apiCall(
        'GET',
        `/channels/${channelId}/messages/${messageId}/reactions/${encodeURIComponent(api)}?limit=100`,
      );
      return Array.isArray(res) ? (res as User[]) : [];
    } catch {
      return [];
    }
  }

  /* ---------- треды, форумы, группы, серверы, профиль (v0.8.0) ---------- */

  /** Создать тред/форум-пост: POST /channels/{cid}/threads (+первое сообщение). */
  async createThread(channelId: string, name: string, content?: string): Promise<Channel | null> {
    try {
      const t = (await this.apiCall('POST', `/channels/${channelId}/threads`, {
        name,
        ...(content ? { message: { content } } : {}),
      })) as Channel;
      if (t?.id) this.selectChannel(t.id);
      return t;
    } catch (e) {
      this.toast(`Ветка не создана: ${errMsg(e)}`, 'error');
      return null;
    }
  }

  /** Ветка из сообщения (контекст-меню): POST …/messages/{mid}/threads. */
  async createThreadFromMessage(message: Message, name: string): Promise<void> {
    try {
      const t = (await this.apiCall(
        'POST',
        `/channels/${message.channel_id}/messages/${message.id}/threads`,
        { name },
      )) as Channel;
      if (t?.id) this.selectChannel(t.id);
    } catch (e) {
      this.toast(`Ветка не создана: ${errMsg(e)}`, 'error');
    }
  }

  /** Обновить список активных тредов гильдии (гидрация постов форума). */
  async refreshActiveThreads(guildId: string): Promise<void> {
    try {
      const res = (await this.apiCall('GET', `/guilds/${guildId}/threads/active`)) as {
        threads?: Channel[];
      };
      const g = this.guilds.find((x) => x.id === guildId);
      if (g && res?.threads) {
        const map = new Map((g.threads ?? []).map((t) => [t.id, t]));
        for (const t of res.threads) map.set(t.id, t);
        g.threads = [...map.values()];
      }
    } catch {
      /* silent — форум покажет то, что уже в кэше */
    }
  }

  /** Групповая ЛС (type 3): POST /users/@me/channels {recipients}. */
  async createGroupDm(recipientIds: string[]): Promise<Channel | null> {
    try {
      const c = (await this.apiCall('POST', '/users/@me/channels', {
        recipients: recipientIds,
      })) as Channel;
      if (c?.id) {
        this.dms = [...this.dms.filter((x) => x.id !== c.id), this.hydrateDm(c)];
        this.selectChannel(c.id);
      }
      return c;
    } catch (e) {
      this.toast(`Группа не создана: ${errMsg(e)}`, 'error');
      return null;
    }
  }

  /** Свой сервер: POST /guilds {name} — GUILD_CREATE доедет по gateway. */
  async createServer(name: string): Promise<void> {
    try {
      const g = (await this.apiCall('POST', '/guilds', { name })) as Guild;
      if (g?.id) this.selectGuild(g.id);
    } catch (e) {
      this.toast(`Сервер не создан: ${errMsg(e)}`, 'error');
    }
  }

  /** Профиль: аватар (data URI), био, display name — PATCH /users/@me. */
  async updateProfile(patch: { bio?: string; global_name?: string; avatar?: string | null }): Promise<boolean> {
    try {
      const u = (await this.apiCall('PATCH', '/users/@me', patch)) as User;
      if (u?.id && this.user) this.user = { ...this.user, ...u };
      this.toast('Профиль обновлён', 'info');
      return true;
    } catch (e) {
      this.toast(`Профиль не обновлён: ${errMsg(e)}`, 'error');
      return false;
    }
  }

  async toggleVoice(guildId: string | null, channelId: string) {
    if (!this.backend) return;
    // guildId = null → звонок в ЛС/группе (myVoice.guildId === '').
    const joined =
      this.myVoice?.channelId === channelId &&
      this.myVoice?.guildId === (guildId ?? '');
    try {
      await this.backend.joinVoice(
        guildId,
        joined ? null : channelId,
        this.myVoice?.selfMute ?? false,
        this.myVoice?.selfDeaf ?? false,
        this.myVoice?.selfVideo ?? false,
      );
      if (joined) await this.stopLocalPreview(); // выход из войса гасит превью
    } catch (e) {
      this.toast(`Ошибка войса: ${errMsg(e)}`, 'error');
    }
  }

  async leaveVoice() {
    if (!this.backend || !this.myVoice) return;
    try {
      await this.stopLocalPreview();
      await this.backend.joinVoice(
        this.myVoice.guildId || null,
        null,
        false,
        false,
        false,
      );
    } catch (e) {
      this.toast(`Ошибка выхода: ${errMsg(e)}`, 'error');
    }
  }

  async setSelfMute(mute: boolean) {
    if (!this.backend || !this.myVoice) return;
    try {
      await this.backend.joinVoice(
        this.myVoice.guildId || null,
        this.myVoice.channelId,
        mute,
        this.myVoice.selfDeaf,
        this.myVoice.selfVideo,
      );
    } catch {
      /* silent */
    }
  }

  async setSelfDeaf(deaf: boolean) {
    if (!this.backend || !this.myVoice) return;
    try {
      await this.backend.joinVoice(
        this.myVoice.guildId || null,
        this.myVoice.channelId,
        this.myVoice.selfMute,
        deaf,
        this.myVoice.selfVideo,
      );
    } catch {
      /* silent */
    }
  }

  /* ---------- трансляции: экран и камера (эксперимент) ---------- */

  /** Свои стримы — по владельцу ключа (guild:channel:user[:type]). */
  get myStreams(): StreamInfo[] {
    return Object.values(this.streams).filter(
      (s) => s.userId === this.user?.id || s.streamKey.includes(`:${this.user?.id}:`),
    );
  }

  /** Чужие трансляции в канале, где мы сейчас в войсе. */
  streamsInMyChannel(): StreamInfo[] {
    const mv = this.myVoice;
    if (!mv) return [];
    return Object.values(this.streams).filter(
      (s) => s.guildId === mv.guildId && s.channelId === mv.channelId,
    );
  }

  /** Включить/выключить показ экрана или камеры.
   *  Локальное превью — через media-захват webview (getDisplayMedia для
   *  экрана, getUserMedia для камеры); сигналинг — op 18/20 на gateway.
   *  Передача видео-потока другим участникам — в разработке (RTP-стек
   *  следующего релиза), пока это превью + стрим-индикатор для остальных. */
  async toggleStream(kind: 'screen' | 'camera') {
    if (!this.backend || !this.myVoice) {
      this.toast('Сначала войди в голосовой канал', 'error');
      return;
    }
    const own = this.myStreams.find((s) => (s.type ?? 'screen') === kind);
    try {
      if (own) {
        // Стоп: гасим стрим на сервере и локальный захват.
        await this.backend.stopStream(own.streamKey);
        await this.stopLocalPreview();
        this.toast(kind === 'screen' ? 'Показ экрана остановлен' : 'Камера выключена', 'info');
        return;
      }
      // Сначала локальный захват (юзер выбирает окно/экран или разрешает камеру)…
      const stream =
        kind === 'screen'
          ? await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false })
          : await navigator.mediaDevices.getUserMedia({ video: { width: 1280, height: 720 }, audio: false });
      // …затем заявляем стрим на сервер (op 18).
      await this.backend.startStream(kind, this.myVoice.guildId, this.myVoice.channelId);
      this.localPreview = stream;
      this.localPreviewKind = kind;
      // Юзер нажал «Стоп» в нативной полосе браузера — гасим и стрим.
      stream.getVideoTracks()[0]?.addEventListener('ended', () => {
        void this.stopStreamByKind(kind);
      });
      this.toast(
        kind === 'screen'
          ? 'Показ экрана включён (превью локальное; передача — в следующем обновлении)'
          : 'Камера включена (превью локальное; передача — в следующем обновлении)',
        'info',
      );
    } catch (e) {
      this.toast(
        `Не удалось включить ${kind === 'screen' ? 'показ экрана' : 'камеру'}: ${errMsg(e)}`,
        'error',
      );
    }
  }

  private async stopStreamByKind(kind: 'screen' | 'camera') {
    const own = this.myStreams.find((s) => (s.type ?? 'screen') === kind);
    if (own) {
      try {
        await this.backend?.stopStream(own.streamKey);
      } catch {
        /* silent */
      }
    }
    await this.stopLocalPreview();
  }

  /** Остановить локальный захват превью (треки обязаны быть остановлены,
   *  иначе индикатор камеры/захвата висит в системе). */
  async stopLocalPreview() {
    if (this.localPreview) {
      for (const t of this.localPreview.getTracks()) t.stop();
    }
    this.localPreview = null;
    this.localPreviewKind = null;
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

  /* ================= guild management (v0.3.0) ================= */

  /** Универсальный REST-вызов через Rust-слой (rate limits + заголовки сессии). */
  async apiCall(method: string, path: string, body?: unknown): Promise<any> {
    if (!this.backend) throw new Error('Нет подключения');
    return this.backend.apiRequest(method, path, body);
  }

  async listBans(guildId: string): Promise<BanEntry[]> {
    return this.apiCall('GET', `/guilds/${guildId}/bans`);
  }

  async listInvites(guildId: string): Promise<Invite[]> {
    return this.apiCall('GET', `/guilds/${guildId}/invites`);
  }

  /* ================= friends & DMs (v0.4.0) ================= */

  /** Статус друга/участника ЛС — сначала гильдейские статусы, затем друзья. */
  friendStatus(userId: string): string {
    return this.friendPresences[userId]?.status ?? 'offline';
  }

  /** Найти существующий DM с пользователем (по списку получателей). */
  dmWith(userId: string): Channel | null {
    return (
      this.dms.find(
        (c) =>
          (c.type === 1 || c.type === 3) &&
          (c.recipients?.some((r) => r.id === userId) || c.recipient_ids?.includes(userId)),
      ) ?? null
    );
  }

  /** Открыть личную беседу: существующий DM или создать через REST.
   *  Работает для юзер-аккаунта — писать первым можно друзьям и тем,
   *  с кем есть общие серверы. */
  async openDmWith(userId: string) {
    if (!this.backend) return;
    const existing = this.dmWith(userId);
    if (existing) {
      this.activeGuildId = null;
      this.selectChannel(existing.id);
      return;
    }
    try {
      const ch = await this.backend.createDm(userId);
      if (!this.dms.some((x) => x.id === ch.id)) this.dms.push(ch);
      this.activeGuildId = null;
      this.selectChannel(ch.id);
    } catch (e) {
      this.toast(`Не удалось открыть беседу: ${errMsg(e)}`, 'error');
    }
  }

  /** Отправить заявку в друзья по имени пользователя (без #). */
  async sendFriendRequest(handle: string): Promise<boolean> {
    const name = handle.trim().replace(/^@/, '');
    if (!name) {
      this.toast('Введи имя пользователя', 'error');
      return false;
    }
    try {
      // Новый формат имён: просто username. Старый «name#1234» приводим к нему.
      const [username, discriminator] = name.split('#');
      const body: Record<string, string> = { username };
      if (discriminator && discriminator !== '0') body.discriminator = discriminator;
      await this.apiCall('POST', '/users/@me/relationships', body);
      this.toast(`Заявка для @${username} отправлена`, 'info');
      void this.syncRelationships();
      return true;
    } catch (e) {
      this.toast(friendlyFriendError(errMsg(e)), 'error');
      return false;
    }
  }

  /** Принять входящую заявку (или добавить принятого в друзья). */
  async acceptFriend(userId: string) {
    try {
      await this.apiCall('PUT', `/users/@me/relationships/${userId}`, { type: 1 });
      this.toast('Заявка принята — теперь вы друзья', 'info');
    } catch (e) {
      this.toast(`Не получилось: ${errMsg(e)}`, 'error');
    }
  }

  /** Блокировать пользователя. */
  async blockUser(userId: string) {
    try {
      await this.apiCall('PUT', `/users/@me/relationships/${userId}`, { type: 2 });
      this.toast('Пользователь заблокирован', 'info');
    } catch (e) {
      this.toast(`Не получилось: ${errMsg(e)}`, 'error');
    }
  }

  /** Удалить из друзей / отменить заявку / отклонить входящую / разблокировать. */
  async removeRelationship(userId: string, what: string) {
    try {
      await this.apiCall('DELETE', `/users/@me/relationships/${userId}`);
      this.toast(what, 'info');
    } catch (e) {
      this.toast(`Не получилось: ${errMsg(e)}`, 'error');
    }
  }

  /** REST-синхронизация друзей — страховка от потерянного READY-массива. */
  async syncRelationships() {
    if (!this.backend || this.isBotAccount) return;
    try {
      const rels = await this.backend.listRelationships();
      const map: Record<string, Relationship> = {};
      for (const r of rels) map[r.user.id] = r;
      this.relationships = map;
    } catch {
      /* тихо: страницу друзей покажет и gateway-кэш */
    }
  }

  /** Присоединиться к серверу по ссылке-приглашению (юзер-аккаунт). */
  async joinServer(invite: string) {
    const code = invite
      .trim()
      .replace(/^https?:\/\/(www\.)?discord\.(gg|com\/invite)\//i, '')
      .replace(/^\/+/, '')
      .split(/[?#]/)[0]
      .trim();
    if (!code) {
      this.toast('Вставь ссылку-приглашение или код', 'error');
      return;
    }
    try {
      const res = (await this.apiCall('POST', `/invites/${encodeURIComponent(code)}`)) as {
        guild?: { name?: string };
      };
      // GUILD_CREATE придёт от gateway сам; тост — немедленный отклик.
      this.toast(`Присоединился к «${res.guild?.name ?? code}»`, 'info');
    } catch (e) {
      this.toast(`Не получилось: ${errMsg(e)}`, 'error');
    }
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
    if (this.usersById[userId]) return this.usersById[userId];
    for (const map of Object.values(this.membersByGuild)) {
      const m = map[userId];
      if (m?.user) return m.user;
    }
    for (const c of this.dms) {
      const u = c.recipients?.find((r) => r.id === userId);
      if (u) return u;
    }
    if (this.relationships[userId]) return this.relationships[userId].user;
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
    const guild = this.presencesByGuild[guildId]?.[userId];
    if (guild) return guild.status;
    // Вне гильдии (друзья, ЛС) — карта дружеских статусов.
    return this.friendPresences[userId]?.status ?? 'offline';
  }

  /** Статус собеседника ЛС/группы: друзья → любые общие серверы.
   *  Раньше брали только friendPresences — неличные партнёры по группе
   *  всегда выглядели «не в сети». */
  statusOfDm(userId: string): string {
    const friend = this.friendPresences[userId];
    if (friend) return friend.status;
    for (const map of Object.values(this.presencesByGuild)) {
      const p = map[userId];
      if (p) return p.status;
    }
    return 'offline';
  }

  /** Участники звонка в ЛС/группе (VOICE_STATE_UPDATE с guild_id = null). */
  dmCallUsers(channelId: string): VoiceState[] {
    return this.dmCalls[channelId] ?? [];
  }

  /** Своя карточка участника гильдии (роли — для расчёта прав). */
  myMemberOf(guildId: string): Member | null {
    if (!this.user) return null;
    return this.membersByGuild[guildId]?.[this.user.id] ?? null;
  }

  /** Базовые права в гильдии: объединение бит ролей (вкл. @everyone).
   *  null = данных нет (не пришла карточка/роли) — UI не фильтрует. */
  myBasePermissions(guildId: string): bigint | null {
    const guild = this.guilds.find((g) => g.id === guildId);
    const me = this.myMemberOf(guildId);
    if (!guild || !me || !guild.roles.length) return null;
    const roleIds = [...me.roles, guildId]; // @everyone = id гильдии
    return computePermissions(roleIds, guild.roles);
  }

  /** Виден ли канал текущему юзеру: VIEW_CHANNEL с учётом перезаписей
   *  (@everyone → роли → личные). Без данных о ролях не скрываем —
   *  лучше показать лишний канал, чем спрятать весь сервер. */
  canViewChannel(guildId: string, channel: Channel): boolean {
    if (this.isBotAccount || this.isDemo) return true;
    if (channel.type === 4) return true; // категории фильтруем по детям
    const base = this.myBasePermissions(guildId);
    if (base === null) return true;
    if (has(base, PERMISSIONS.ADMINISTRATOR)) return true;
    const overwrites = channel.permission_overwrites ?? [];
    if (!overwrites.length) return has(base, PERMISSIONS.VIEW_CHANNEL);
    const me = this.myMemberOf(guildId);
    const roleIds = [...(me?.roles ?? []), guildId];
    const bits = computeChannelPermissions(
      base,
      guildId,
      overwrites,
      roleIds,
      this.user?.id ?? '',
    );
    return has(bits, PERMISSIONS.VIEW_CHANNEL);
  }

  /** Каналы гильдии, видимые текущему юзеру (для сайдбара/выбора). */
  visibleChannels(guildId: string): Channel[] {
    const guild = this.guilds.find((g) => g.id === guildId);
    if (!guild) return [];
    return (guild.channels ?? []).filter((c) => this.canViewChannel(guildId, c));
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

/** Человеческие формулировки для частых ошибок заявок в друзья. */
function friendlyFriendError(raw: string): string {
  const m = raw.toLowerCase();
  if (m.includes('relationship already') || m.includes('уже друзья')) {
    return 'Вы уже друзья';
  }
  if (m.includes('friend request already') || m.includes('already sent')) {
    return 'Заявка уже отправлена — ждём ответа';
  }
  if (m.includes('maximum number of friends') || m.includes('max friends')) {
    return 'Достигнут лимит друзей — сначала удали кого-нибудь';
  }
  if (m.includes('too many requests') || m.includes('too many friend')) {
    return 'Слишком много заявок подряд — подожди немного';
  }
  if (m.includes('не хватает прав') || m.includes('403')) {
    return 'Не хватает прав для этого действия';
  }
  if (m.includes('unknown user') || m.includes('404')) {
    return 'Пользователь не найден — проверь имя (имя_пользователя без #)';
  }
  return `Заявка не отправилась: ${raw}`;
}

function detectTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export const app = new AppStore();
