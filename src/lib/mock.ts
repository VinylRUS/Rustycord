/**
 * MockBackend — a faithful simulation of the Discord Gateway + REST layer.
 *
 * Purpose:
 *  1. Run the UI in any browser (demo mode, no token needed).
 *  2. Act as executable documentation of the protocol: the event flow here
 *     (status → ready → GUILD_CREATE → GUILD_MEMBERS_CHUNK → live dispatches)
 *     is exactly what the Rust core receives from the real Gateway.
 */
import type { Backend, GetMessageOptions } from './ipc';
import type { BackendEvent, Channel, Guild, Member, Message, Presence, Role, SnowflakeInfo, StateSnapshot, User, VoiceState } from './types';
import { parseSnowflakeLocal } from './time';

/* ---------- helpers ---------- */

const EPOCH = 1420070400000n;
/** Build a plausible snowflake N ms ago with per-message increment. */
function sf(msAgo: number, seq: number): string {
  const ts = BigInt(Date.now() - msAgo);
  return (((ts - EPOCH) << 22n) | BigInt(seq)).toString();
}
const H = 3600_000;
const M = 60_000;

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

/* ---------- demo world ---------- */

const BOT: User = {
  id: '1302000000000000001',
  username: 'rustycord.bot',
  discriminator: '0',
  global_name: 'RustyCord',
  avatar: null,
  bot: true,
};

const GUILD_ID = '1301000000000000010';

const ROLES: Role[] = [
  { id: GUILD_ID, name: '@everyone', color: 0, hoist: false, position: 0, permissions: '104324673', managed: false },
  { id: '1301000000000000011', name: 'Админ', color: 0xed4245, hoist: true, position: 4, permissions: '8' },
  { id: '1301000000000000012', name: 'Модератор', color: 0x57f287, hoist: true, position: 3, permissions: '1099511697657' },
  { id: '1301000000000000013', name: 'Разработчик', color: 0x5865f2, hoist: true, position: 2, permissions: '1080106024961' },
  { id: '1301000000000000014', name: 'Бот', color: 0xeb459e, hoist: true, position: 1, permissions: '137438953473' },
];

function user(id: string, username: string, globalName: string, extra: Partial<User> = {}): User {
  return { id, username, discriminator: '0', global_name: globalName, avatar: null, ...extra };
}

const ALEXEY = user('1301000000000000201', 'alexey.dev', 'Алексей');
const MARINA = user('1301000000000000202', 'marina.k', 'Марина');
const DMITRY = user('1301000000000000203', 'dmitry404', 'Дмитрий');
const OLYA = user('1301000000000000204', 'olya.rust', 'Оля');
const PYOTR = user('1301000000000000205', 'pyotr', 'Пётр');
const IRINA = user('1301000000000000206', 'irina', 'Ирина');
const MELODY = user('1301000000000000207', 'melody', 'Melody', { bot: true });

const MEMBERS: Member[] = [
  { user: ALEXEY, nick: null, roles: ['1301000000000000011'], joined_at: '2025-11-02T10:00:00.000+00:00' },
  { user: MARINA, nick: null, roles: ['1301000000000000012'], joined_at: '2025-11-02T11:20:00.000+00:00' },
  { user: DMITRY, nick: 'дима', roles: ['1301000000000000013'], joined_at: '2025-11-04T09:05:00.000+00:00' },
  { user: OLYA, nick: null, roles: ['1301000000000000013'], joined_at: '2025-11-05T14:41:00.000+00:00' },
  { user: PYOTR, nick: null, roles: [], joined_at: '2025-12-01T18:00:00.000+00:00' },
  { user: IRINA, nick: null, roles: [], joined_at: '2025-12-03T12:30:00.000+00:00' },
  { user: MELODY, nick: null, roles: ['1301000000000000014'], joined_at: '2025-11-10T08:00:00.000+00:00' },
  { user: BOT, nick: null, roles: ['1301000000000000014'], joined_at: '2025-12-20T08:00:00.000+00:00' },
];

const PRESENCES: Presence[] = [
  { user: { id: ALEXEY.id }, status: 'online', activities: [{ name: 'Custom Status', type: 4, state: 'ревёрсю Gateway 🔧' }] },
  { user: { id: MARINA.id }, status: 'idle', activities: [{ name: 'Visual Studio Code', type: 0, state: 'gateway.py' }] },
  { user: { id: DMITRY.id }, status: 'dnd', activities: [] },
  { user: { id: OLYA.id }, status: 'online', activities: [{ name: 'Custom Status', type: 4, state: 'crab rave 🦀' }] },
  { user: { id: PYOTR.id }, status: 'offline', activities: [] },
  { user: { id: IRINA.id }, status: 'offline', activities: [] },
  { user: { id: MELODY.id }, status: 'online', activities: [] },
];

const CHANNELS: Channel[] = [
  { id: '1301000000000000301', type: 4, guild_id: GUILD_ID, name: 'Информация', position: 0, parent_id: null },
  { id: '1301000000000000302', type: 0, guild_id: GUILD_ID, name: 'правила', position: 0, parent_id: '1301000000000000301', topic: 'Читать до постинга' },
  { id: '1301000000000000303', type: 0, guild_id: GUILD_ID, name: 'анонсы', position: 1, parent_id: '1301000000000000301' },
  { id: '1301000000000000304', type: 4, guild_id: GUILD_ID, name: 'Общение', position: 1, parent_id: null },
  { id: '1301000000000000305', type: 0, guild_id: GUILD_ID, name: 'общий', position: 0, parent_id: '1301000000000000304', topic: 'Свободное общение о проекте' },
  { id: '1301000000000000306', type: 0, guild_id: GUILD_ID, name: 'разработка', position: 1, parent_id: '1301000000000000304', topic: 'Rust, Tauri, Svelte 5' },
  { id: '1301000000000000307', type: 0, guild_id: GUILD_ID, name: 'реверс-discord', position: 2, parent_id: '1301000000000000304', topic: 'Разбор протоколов Discord' },
  { id: '1301000000000000308', type: 0, guild_id: GUILD_ID, name: 'флудилка', position: 3, parent_id: '1301000000000000304' },
  { id: '1301000000000000309', type: 4, guild_id: GUILD_ID, name: 'Голосовые', position: 2, parent_id: null },
  { id: '1301000000000000310', type: 2, guild_id: GUILD_ID, name: 'Общий войс', position: 0, parent_id: '1301000000000000309' },
  { id: '1301000000000000311', type: 2, guild_id: GUILD_ID, name: 'Тихая комната', position: 1, parent_id: '1301000000000000309' },
];

const VOICE_STATES: VoiceState[] = [
  { guild_id: GUILD_ID, channel_id: '1301000000000000310', user_id: ALEXEY.id, session_id: 'demo-1', self_mute: false, self_deaf: false },
  { guild_id: GUILD_ID, channel_id: '1301000000000000310', user_id: MARINA.id, session_id: 'demo-2', self_mute: true, self_deaf: false },
];

const GUILD: Guild = {
  id: GUILD_ID,
  name: 'RustyCord HQ',
  icon: null,
  owner_id: ALEXEY.id,
  roles: ROLES,
  channels: CHANNELS,
  voice_states: VOICE_STATES,
  presences: PRESENCES,
  member_count: MEMBERS.length,
  large: false,
};

const DMS: Channel[] = [
  { id: '1301000000000000401', type: 1, name: null, recipients: [MARINA], last_message_id: sf(2 * H, 90) },
  { id: '1301000000000000402', type: 1, name: null, recipients: [DMITRY], last_message_id: sf(26 * H, 91) },
];

function msg(seq: number, agoMs: number, author: User, content: string, channelId: string, extra: Partial<Message> = {}): Message {
  return {
    id: sf(agoMs, seq),
    channel_id: channelId,
    guild_id: GUILD_ID,
    author,
    member: MEMBERS.find((m) => m.user?.id === author.id) ?? null,
    content,
    timestamp: new Date(Date.now() - agoMs).toISOString(),
    edited_timestamp: null,
    tts: false,
    mention_everyone: false,
    mentions: [],
    attachments: [],
    embeds: [],
    reactions: [],
    pinned: false,
    type: 0,
    ...extra,
  };
}

const CH_GENERAL = '1301000000000000305';
const CH_REVERSE = '1301000000000000307';
const CH_DEV = '1301000000000000306';
const CH_RULES = '1301000000000000302';

const HISTORY: Record<string, Message[]> = {
  [CH_GENERAL]: [
    msg(1, 40 * H, ALEXEY, 'Всем привет! 👋 Это **демонстрационный режим** RustyCord — реальный Discord не подключён, но весь протокол Gateway симулируется честно: события, статусы, typing, даже войс.', CH_GENERAL),
    msg(2, 39.5 * H, MARINA, 'Выглядит один в один как настоящий. Даже рейл серверов слева и зелёные точки онлайна на месте.', CH_GENERAL),
    msg(3, 39 * H, DMITRY, 'А размер-то какой? Официальный клиент на Electron весит ~200 МБ, а тут весь бинарь на Rust укладывается в ~10 МБ.', CH_GENERAL),
    msg(4, 38 * H, OLYA, 'Tauri использует системный WebView вместо того, чтобы тащить свой Chromium, — в этом вся магия 🦀', CH_GENERAL),
    msg(5, 37 * H, ALEXEY, 'Напоминаю правила: не ддосим Gateway, heartbeat шлём каждые `41 250 мс`, как завещал протокол.', CH_GENERAL, {
      reactions: [{ count: 3, me: false, emoji: { id: null, name: '👍' } }],
    }),
    msg(6, 30 * H, MARINA, 'Скинула дамп опкодов в <#1301000000000000307> — гляньте, всё сходится с discord.py.', CH_GENERAL),
    msg(7, 29 * H, DMITRY, 'RESUME работает? Если соединение порвётся, сессия подхватится без потери событий?', CH_GENERAL),
    msg(8, 28.9 * H, ALEXEY, 'Да: `session_id` + `sequence number`, и события за простой добираются сервером. Даже `resume_gateway_url` поддерживаем.', CH_GENERAL, {
      edited_timestamp: new Date(Date.now() - 28 * H).toISOString(),
    }),
    msg(9, 2 * H, OLYA, 'Кто-нибудь заходит в войс? Я в **Общем войсе**, обсудим шифрование `xsalsa20poly1305`.', CH_GENERAL),
    msg(10, 1 * H, MARINA, 'Я уже там, но на мьюте — сосед делает ремонт 🔨', CH_GENERAL),
    msg(11, 25 * M, ALEXEY, 'Всё, финальная сборка готова. Погнали тестировать! 🚀', CH_GENERAL, {
      reactions: [
        { count: 4, me: false, emoji: { id: null, name: '🚀' } },
        { count: 2, me: false, emoji: { id: null, name: '🦀' } },
      ],
    }),
  ],
  [CH_REVERSE]: [
    msg(20, 30 * H, MARINA, 'Итак, **Gateway WebSocket** — таблица опкодов:\n```json\n0  DISPATCH        события\n1  HEARTBEAT       пульс\n2  IDENTIFY        авторизация\n6  RESUME          восстановление\n7  RECONNECT       сервер просит переподключиться\n9  INVALID_SESSION сброс сессии\n10 HELLO           интервал пульса\n11 HEARTBEAT_ACK   подтверждение пульса\n```', CH_REVERSE),
    msg(21, 29.9 * H, MARINA, 'Интервал приходит в HELLO: `heartbeat_interval: 41250`. После каждого пульса ждём ACK, два подряд без ответа — соединение зомби, переподключаемся.', CH_REVERSE, {
      reactions: [{ count: 3, me: false, emoji: { id: null, name: '🧠' } }],
    }),
    msg(22, 24 * H, DMITRY, 'Snowflake — это 64 бита:\n```text\n[ timestamp: 42 бита ][ worker: 5 ][ process: 5 ][ increment: 12 ]\n```\nЭпоха Discord — `2015-01-01T00:00:00Z`. Формула времени: `(id >> 22) + 1420070400000`.', CH_REVERSE),
    msg(23, 23 * H, DMITRY, 'Поэтому API не шлёт поле `timestamp` в MESSAGE_CREATE — клиент сам достаёт его из id сообщения. Проверь в инспекторе снежинок в настройках 😉', CH_REVERSE),
    msg(24, 20 * H, ALEXEY, 'Не забываем про **rate limits**: заголовки `X-RateLimit-Bucket`, `X-RateLimit-Remaining`, `X-RateLimit-Reset-After`. Бакеты вешаются на маршрут, а не на эндпоинт.', CH_REVERSE),
    msg(25, 18 * H, OLYA, 'Войс — отдельная история: главный Gateway используется только для сигналинга (`op 4` → `VOICE_SERVER_UPDATE`), а сам звук идёт по UDP с SRTP-шифрованием.', CH_REVERSE),
    msg(26, 17 * H, ALEXEY, 'Именно поэтому в RustyCord отдельный voice-модуль: свой WebSocket + UDP-сокет + crypto. Полный разбор — в README проекта.', CH_REVERSE),
  ],
  [CH_DEV]: [
    msg(30, 26 * H, DMITRY, 'Стек: `Rust (tokio) + Tauri 2 + Svelte 5 (runes)`. IPC через `invoke`/`listen`, кэш в `RwLock`.', CH_DEV),
    msg(31, 25 * H, OLYA, 'Стор на рунах получился лаконичный — `$state`, `$derived`, `$effect`. Ни одного стора из svelte/store 🎉', CH_DEV),
    msg(32, 24 * H, DMITRY, 'Проверил: рест-клиент уважает rate limits, 429 больше не ловим.', CH_DEV),
  ],
  [CH_RULES]: [
    msg(40, 41 * H, ALEXEY, 'Правила простые:\n1. Уважаем rate limits\n2. Не спамим heartbeat\n3. Интенты включаем в Developer Portal\n4. 🦀 крабов не обижаем', CH_RULES, { pinned: true }),
  ],
  '1301000000000000401': [
    msg(50, 3 * H, MARINA, 'Привет! Скинь, пожалуйста, схему Snowflake, когда будет минутка 🙏', '1301000000000000401'),
    msg(51, 2.5 * H, MARINA, 'И да, войс тестируем завтра в 19:00, не забудь.', '1301000000000000401'),
  ],
  '1301000000000000402': [
    msg(60, 26 * H, DMITRY, 'Брокер сообщений не нужен — у нас event-driven архитектура через Tauri IPC.', '1301000000000000402'),
    msg(61, 25 * H, DMITRY, 'Кстати, добавь в README сравнение с Electron, будет эффектно.', '1301000000000000402'),
  ],
};

const OLDER_PAGE: Message[] = [
  msg(70, 3 * 24 * H, PYOTR, 'А боты вообще видят содержимое сообщений?', CH_GENERAL),
  msg(71, 3 * 24 * H - 2 * M, MARINA, 'Только с включённым **Message Content Intent** — привилегированные интенты теперь включаются вручную в Developer Portal.', CH_GENERAL),
  msg(72, 3 * 24 * H - 5 * M, ALEXEY, 'Точно. Плюс **Presence Intent** для статусов и **Server Members Intent** для списка участников.', CH_GENERAL),
  msg(73, 3 * 24 * H - 8 * M, OLYA, 'В README всё расписано, раздел «Быстрый старт» 📖', CH_GENERAL),
];

/** Live-traffic script: [typing user, message author, text] */
const SCRIPT: Array<{ typing: User; author: User; text: string; channel: string }> = [
  { typing: MARINA, author: MARINA, text: 'Нашла классный мануал по Snowflake — залью в <#1301000000000000307> 📐', channel: CH_GENERAL },
  { typing: OLYA, author: OLYA, text: 'Тестировала сборку на Tauri 2 — бинарь вышел **9.8 МБ**. Electron плачет 😎', channel: CH_GENERAL },
  { typing: DMITRY, author: DMITRY, text: 'Кто-нибудь пробовал resume после сна ноутбука? Сессия восстанавливается, события не теряются.', channel: CH_REVERSE },
  { typing: ALEXEY, author: ALEXEY, text: 'Следующий шаг — приём opus-аудио. RTP-заголовок уже разобран: 12 байт + optional extension.', channel: CH_DEV },
  { typing: MARINA, author: MARINA, text: 'Схему Snowflake добавила в вики, забирайте!', channel: CH_REVERSE },
];

/* ---------- backend ---------- */

export class MockBackend implements Backend {
  readonly mode = 'mock' as const;

  private handlers = new Set<(ev: BackendEvent) => void>();
  private timers: ReturnType<typeof setTimeout>[] = [];
  private members = [...MEMBERS];
  private histories: Record<string, Message[]> = Object.fromEntries(
    Object.entries(HISTORY).map(([k, v]) => [k, v.map((m) => ({ ...m }))]),
  );
  private olderServed = new Set<string>();
  private seq = 100;
  private running = false;

  private emit(ev: BackendEvent) {
    for (const h of this.handlers) h(ev);
  }

  private nextId(): string {
    return sf(0, this.seq++);
  }

  onEvent(handler: (ev: BackendEvent) => void): () => void {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  async connect(token: string): Promise<User> {
    if (!token.trim()) throw new Error('Введи любой токен (это демо) или нажми «Демо-режим»');
    this.running = true;
    this.emit({ type: 'status', stage: 'connecting' });
    await delay(350);
    this.emit({ type: 'ready', user: BOT, session_id: 'demo-session-42' });
    await delay(250);
    // Full GUILD_CREATE, exactly like the real gateway sends it.
    this.emit({
      type: 'dispatch',
      t: 'GUILD_CREATE',
      d: { ...GUILD, members: [], presences: PRESENCES, voice_states: [...VOICE_STATES] },
    });
    await delay(300);
    this.emit({
      type: 'dispatch',
      t: 'GUILD_MEMBERS_CHUNK',
      d: { guild_id: GUILD_ID, members: this.members, chunk_index: 0, chunk_count: 1 },
    });
    this.emit({ type: 'dispatch', t: 'CHANNEL_CREATE', d: DMS[0] });
    this.emit({ type: 'dispatch', t: 'CHANNEL_CREATE', d: DMS[1] });
    this.emit({ type: 'status', stage: 'connected' });
    this.runSimulation();
    return BOT;
  }

  async disconnect(): Promise<void> {
    this.running = false;
    for (const t of this.timers) clearTimeout(t);
    this.timers = [];
    this.emit({ type: 'status', stage: 'disconnected' });
  }

  async getState(): Promise<StateSnapshot> {
    return {
      user: BOT,
      guilds: [{ ...GUILD, members: this.members, presences: PRESENCES, voice_states: VOICE_STATES }],
      dms: DMS,
    };
  }

  async getMessages(channelId: string, opts?: GetMessageOptions): Promise<Message[]> {
    await delay(120);
    if (opts?.before) {
      if (this.olderServed.has(channelId)) return [];
      this.olderServed.add(channelId);
      return OLDER_PAGE.map((m) => ({ ...m }));
    }
    return (this.histories[channelId] ?? []).map((m) => ({ ...m }));
  }

  async sendMessage(channelId: string, content: string): Promise<Message> {
    const message = msg(this.seq++, 0, BOT, content, channelId);
    message.id = this.nextId();
    (this.histories[channelId] ??= []).push(message);
    // A friendly member sometimes reacts to us.
    if (Math.random() < 0.5) {
      this.timers.push(
        setTimeout(() => {
          message.reactions = [{ count: 1, me: true, emoji: { id: null, name: '👍' } }];
          this.emit({
            type: 'dispatch',
            t: 'MESSAGE_REACTION_ADD',
            d: { channel_id: channelId, message_id: message.id, user_id: MARINA.id, emoji: { id: null, name: '👍' } },
          });
        }, 1800),
      );
    }
    return { ...message };
  }

  async editMessage(channelId: string, messageId: string, content: string): Promise<Message> {
    const list = this.histories[channelId] ?? [];
    const m = list.find((x) => x.id === messageId);
    if (!m) throw new Error('Сообщение не найдено');
    m.content = content;
    m.edited_timestamp = new Date().toISOString();
    this.emit({ type: 'dispatch', t: 'MESSAGE_UPDATE', d: { ...m } });
    return { ...m };
  }

  async deleteMessage(channelId: string, messageId: string): Promise<void> {
    this.histories[channelId] = (this.histories[channelId] ?? []).filter((m) => m.id !== messageId);
    this.emit({ type: 'dispatch', t: 'MESSAGE_DELETE', d: { channel_id: channelId, id: messageId } });
  }

  async addReaction(channelId: string, messageId: string, emoji: string): Promise<void> {
    this.emit({
      type: 'dispatch',
      t: 'MESSAGE_REACTION_ADD',
      d: { channel_id: channelId, message_id: messageId, user_id: BOT.id, emoji: { id: null, name: emoji } },
    });
  }

  async triggerTyping(_channelId: string): Promise<void> {
    /* the real endpoint returns 204; nothing to simulate locally */
  }

  async requestMembers(guildId: string): Promise<void> {
    this.emit({
      type: 'dispatch',
      t: 'GUILD_MEMBERS_CHUNK',
      d: { guild_id: guildId, members: this.members, chunk_index: 0, chunk_count: 1 },
    });
  }

  async gatewaySendRaw(frame: string): Promise<void> {
    // В демо-режиме настоящего websocket нет — валидируем JSON и «отправляем»
    // обратно как dispatch, чтобы UI-консоль протокола была живой.
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(frame) as Record<string, unknown>;
    } catch {
      throw new Error('Невалидный JSON: ' + frame.slice(0, 40));
    }
    this.emit({
      type: 'dispatch',
      t: 'RAW_FRAME_ECHO',
      d: { op: parsed.op ?? null, note: 'демо-режим: кадр распознан, но никуда не уйдёт' },
    });
  }

  async kickMember(guildId: string, userId: string): Promise<void> {
    this.members = this.members.filter((m) => m.user?.id !== userId);
    this.emit({ type: 'dispatch', t: 'GUILD_MEMBER_REMOVE', d: { guild_id: guildId, user: { id: userId } } });
  }

  async banMember(guildId: string, userId: string): Promise<void> {
    this.members = this.members.filter((m) => m.user?.id !== userId);
    this.emit({ type: 'dispatch', t: 'GUILD_MEMBER_REMOVE', d: { guild_id: guildId, user: { id: userId } } });
  }

  async joinVoice(guildId: string, channelId: string | null, selfMute: boolean, selfDeaf: boolean): Promise<void> {
    this.emit({
      type: 'dispatch',
      t: 'VOICE_STATE_UPDATE',
      d: { guild_id: guildId, channel_id: channelId, user_id: BOT.id, self_mute: selfMute, self_deaf: selfDeaf, session_id: 'demo-voice-1' },
    });
    if (channelId) {
      await delay(500);
      this.emit({
        type: 'voice',
        kind: 'connected',
        data: { guild_id: guildId, channel_id: channelId, mode: 'xsalsa20_poly1305', ssrc: 424242, endpoint: 'guild-42.discord.media', ip: '127.0.0.1', port: 50001 },
      });
    } else {
      this.emit({ type: 'voice', kind: 'disconnected', data: { guild_id: guildId } });
    }
  }

  async parseSnowflake(id: string): Promise<SnowflakeInfo> {
    return parseSnowflakeLocal(id);
  }

  /** The "live gateway": looping script of realistic events. */
  private runSimulation() {
    if (!this.running) return;
    let step = 0;
    const runStep = () => {
      if (!this.running) return;
      const item = SCRIPT[step % SCRIPT.length];
      step++;
      // typing → message
      this.emit({
        type: 'dispatch',
        t: 'TYPING_START',
        d: { channel_id: item.channel, guild_id: GUILD_ID, user_id: item.typing.id, timestamp: Math.floor(Date.now() / 1000) },
      });
      this.timers.push(
        setTimeout(() => {
          if (!this.running) return;
          const message = msg(this.seq++, 0, item.author, item.text, item.channel);
          message.id = this.nextId();
          (this.histories[item.channel] ??= []).push(message);
          this.emit({ type: 'dispatch', t: 'MESSAGE_CREATE', d: message });
          // someone reacts
          this.timers.push(
            setTimeout(() => {
              if (!this.running) return;
              message.reactions = [{ count: 1, me: false, emoji: { id: null, name: '👍' } }];
              this.emit({
                type: 'dispatch',
                t: 'MESSAGE_REACTION_ADD',
                d: { channel_id: item.channel, message_id: message.id, user_id: ALEXEY.id, emoji: { id: null, name: '👍' } },
              });
            }, 2200),
          );
        }, 2600),
      );
      // presence flip mid-step
      this.timers.push(
        setTimeout(() => {
          if (!this.running) return;
          const statuses = ['online', 'dnd'] as const;
          const next = statuses[step % 2];
          this.emit({ type: 'dispatch', t: 'PRESENCE_UPDATE', d: { user: { id: DMITRY.id }, status: next, activities: [] } });
        }, 9000),
      );
      // voice speaking pulses (Алексей is in the voice channel)
      const speak = (on: boolean) => {
        if (!this.running) return;
        this.emit({ type: 'voice', kind: 'speaking', data: { guild_id: GUILD_ID, user_id: ALEXEY.id, speaking: on } });
      };
      this.timers.push(setTimeout(() => speak(true), 5000));
      this.timers.push(setTimeout(() => speak(false), 7800));
      this.timers.push(setTimeout(() => speak(true), 12000));
      this.timers.push(setTimeout(() => speak(false), 14500));
      // next step
      this.timers.push(setTimeout(runStep, 24000));
    };
    this.timers.push(setTimeout(runStep, 2500));
  }
}
