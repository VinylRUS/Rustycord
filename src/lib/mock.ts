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
import type { BackendEvent, Channel, Guild, LoginResult, Member, Message, PluginInfo, PluginMeta, Presence, Relationship, Role, SnowflakeInfo, StateSnapshot, UpdateInfo, User, UserProfile, VoiceState } from './types';
import { parseSnowflakeLocal } from './time';

/* ---------- helpers ---------- */

const EPOCH = 1420070400000n;
/** Build a plausible snowflake N ms ago with per-message increment. */
function sf(msAgo: number, seq: number): string {
  const ts = BigInt(Date.now() - msAgo);
  return (((ts - EPOCH) << 22n) | BigInt(seq)).toString();
}
const H = 3600_000;

/** «name:id» → кастомный эмодзи, иначе unicode. Как реальный REST-путь. */
function parseApiEmoji(api: string): { id: string | null; name: string } {
  const parts = api.split(':');
  if (parts.length === 2 && /^\d+$/.test(parts[1])) {
    return { id: parts[1], name: parts[0] };
  }
  return { id: null, name: api };
}
const M = 60_000;

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

/* ---------- demo world ---------- */

/** Демо-аккаунт — пользовательский (как настоящий вход по QR/email). */
const ME: User = {
  id: '1302000000000000001',
  username: 'peregovich',
  discriminator: '0',
  global_name: 'Peregovich',
  avatar: null,
  bot: false,
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
  { user: ME, nick: null, roles: ['1301000000000000014'], joined_at: '2025-12-20T08:00:00.000+00:00' },
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
  // «Кастомные эмодзи сервера» — на самом деле публичные эмодзи Discord CDN
  // (id настоящих глобальных эмодзи), чтобы пикер и реакции выглядели живьём.
  emojis: [
    { id: '1000116642218909690', name: 'rustycord', animated: false },
    { id: '1162243827154350132', name: 'rustlove', animated: false },
    { id: '1117802331364020245', name: 'rustcrab', animated: true },
    { id: '1094242520372236378', name: 'tableflip', animated: false },
  ],
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

/** Отношения демо-аккаунта: друзья + входящая/исходящая заявка. */
const RELATIONSHIPS: Relationship[] = [
  { id: MARINA.id, type: 1, user: MARINA, nickname: null },
  { id: OLYA.id, type: 1, user: OLYA, nickname: null },
  { id: PYOTR.id, type: 1, user: PYOTR, nickname: null },
  { id: DMITRY.id, type: 3, user: DMITRY, nickname: null }, // входящая — можно принять
  { id: IRINA.id, type: 4, user: IRINA, nickname: null }, // исходящая — ждём ответа
];

/** Статусы друзей (как READY.presences юзер-сессии — без guild_id). */
const FRIEND_PRESENCES: Presence[] = [
  { user: { id: MARINA.id }, status: 'idle', activities: [{ name: 'Visual Studio Code', type: 0, state: 'gateway.py' }] },
  { user: { id: OLYA.id }, status: 'online', activities: [{ name: 'Custom Status', type: 4, state: 'crab rave 🦀' }] },
  { user: { id: PYOTR.id }, status: 'offline', activities: [] },
  { user: { id: DMITRY.id }, status: 'dnd', activities: [] },
  { user: { id: IRINA.id }, status: 'offline', activities: [] },
];

/** Все известные демо-пользователи — для заявок и создания ЛС. */
const KNOWN_USERS: User[] = [ALEXEY, MARINA, DMITRY, OLYA, PYOTR, IRINA, MELODY];

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
      reactions: [
        { count: 3, me: false, emoji: { id: null, name: '👍' } },
        { count: 1, me: false, emoji: { id: null, name: '😭' } },
      ],
    }),
    msg(6, 30 * H, MARINA, 'Скинула дамп опкодов в <#1301000000000000307> — гляньте, всё сходится с discord.py.', CH_GENERAL),
    msg(7, 28 * H, OLYA, 'Свежий скрин архитектуры:', CH_GENERAL, {
      attachments: [
        {
          id: '1301000000000000991',
          filename: 'rustycord-arch.png',
          size: 204800,
          url: 'https://picsum.photos/seed/rustycord/640/360',
          content_type: 'image/png',
          width: 640,
          height: 360,
        },
      ],
    }),
    msg(8, 27 * H, ALEXEY, '', CH_GENERAL, {
      embeds: [
        {
          type: 'link',
          title: 'RustyCord — лёгкий Discord-клиент на Rust',
          description: 'Tauri + Svelte 5 вместо Electron: **~10 МБ** против ~200 МБ, память ниже на порядок.',
          url: 'https://github.com/VinylRUS/Rustycord',
          color: 0x5865f2,
          footer: { text: 'github.com · звёзд приветствуются' },
        },
      ],
    }),
    msg(9, 20 * H, DMITRY, 'Голос уже проверил? Songbird тянет и микрофон, и приём — звук чистый.', CH_GENERAL, {
      sticker_items: [{ id: '1301000000000000993', name: 'Crab Rave', format_type: 3 }],
    }),
    msg(10, 12 * H, MARINA, 'Кстати, ответы теперь работают — попробуй ответить на это сообщение (правый клик → Ответить).', CH_GENERAL, {
      pinned: true,
    }),
    msg(11, 11.8 * H, ALEXEY, 'И закреплённые тоже — жми 📌 в шапке канала.', CH_GENERAL, {
      message_reference: { message_id: sf(12 * H, 10), channel_id: CH_GENERAL },
    }),
    msg(7, 29 * H, DMITRY, 'RESUME работает? Если соединение порвётся, сессия подхватится без потери событий?', CH_GENERAL),
    msg(8, 28.9 * H, ALEXEY, 'Да: `session_id` + `sequence number`, и события за простой добираются сервером. Даже `resume_gateway_url` поддерживаем.', CH_GENERAL, {
      edited_timestamp: new Date(Date.now() - 28 * H).toISOString(),
    }),
    msg(9, 2 * H, OLYA, 'Кто-нибудь заходит в войс? Я в **Общем войсе**, обсудим шифрование `xsalsa20poly1305`.', CH_GENERAL),
    msg(10, 1 * H, MARINA, 'Я уже там, но на мьюте — сосед делает ремонт 🔨', CH_GENERAL),
    msg(11, 25 * M, ALEXEY, 'Всё, финальная сборка готова. Погнали тестировать! 🚀', CH_GENERAL, {
      reactions: [
        { count: 4, me: true, emoji: { id: null, name: '🚀' } },
        { count: 2, me: false, emoji: { id: null, name: '🦀' } },
        { count: 1, me: false, emoji: { id: null, name: '🔥' } },
        { count: 3, me: false, emoji: { id: '1000116642218909690', name: 'rustycord' } },
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

/** Схематичный QR для демо-режима (без зависимостей). */
function demoQrSvg(): string {
  const cells: Array<[number, number]> = [];
  // finder-паттерны + псевдослучайная «пыль»
  for (let i = 0; i < 7; i++) {
    for (let j = 0; j < 7; j++) {
      const edge = i === 0 || i === 6 || j === 0 || j === 6;
      const core = i >= 2 && i <= 4 && j >= 2 && j <= 4;
      if (edge || core) cells.push([i, j], [i + 17, j], [i, j + 17]);
    }
  }
  for (let i = 8; i < 16; i++) {
    for (let j = 8; j < 16; j++) {
      if ((i * 7 + j * 13) % 5 < 2) cells.push([i, j]);
    }
  }
  const path = cells.map(([x, y]) => `M${x + 2},${y + 2}h1v1h-1z`).join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 25 25" shape-rendering="crispEdges"><rect width="100%" height="100%" fill="#ffffff"/><path d="${path}" fill="#060607"/></svg>`;
}

/** Демо-плагин: показывает формат плагинов RustyCord. */
const DEMO_PLUGIN_SILENT_TYPING = `/* RustyCord plugin — объект-литерал, как у Vencord, только проще */
({
  name: 'Silent Typing',
  version: '1.0.0',
  author: 'RustyCord',
  description: 'Не отправлять индикатор «печатает…»',
  start(api) {
    api.suppress('typing', true);
  },
  stop(api) {
    api.suppress('typing', false);
  }
});
`;

export class MockBackend implements Backend {
  readonly mode = 'mock' as const;

  private handlers = new Set<(ev: BackendEvent) => void>();
  private timers: ReturnType<typeof setTimeout>[] = [];
  private members = [...MEMBERS];
  private relationships: Relationship[] = RELATIONSHIPS.map((r) => ({ ...r, user: { ...r.user } }));
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
    this.emit({ type: 'ready', user: ME, session_id: 'demo-session-42' });
    // Полный пользовательский READY — как его теперь пробрасывает Rust-ядро:
    // relationships, private_channels и presences друзей приезжают сразу.
    this.emit({
      type: 'dispatch',
      t: 'READY',
      d: {
        user: ME,
        session_id: 'demo-session-42',
        // users — справочник участников ЛС (как в реальном юзер-READY).
        users: KNOWN_USERS.map((u) => ({ ...u })),
        relationships: this.relationships.map((r) => ({ ...r })),
        private_channels: DMS.map((c) => ({ ...c })),
        presences: FRIEND_PRESENCES.map((p) => ({ ...p })),
      },
    });
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
    this.emit({ type: 'status', stage: 'connected' });
    this.runSimulation();
    return ME;
  }

  async disconnect(): Promise<void> {
    this.running = false;
    for (const t of this.timers) clearTimeout(t);
    this.timers = [];
    this.emit({ type: 'status', stage: 'disconnected' });
  }

  async getState(): Promise<StateSnapshot> {
    return {
      user: ME,
      guilds: [{ ...GUILD, members: this.members, presences: PRESENCES, voice_states: VOICE_STATES }],
      dms: DMS,
      relationships: this.relationships.map((r) => ({ ...r })),
      friend_presences: FRIEND_PRESENCES,
    };
  }

  /* ---------- друзья и личные беседы (v0.4.0) ---------- */

  async createDm(recipientId: string): Promise<Channel> {
    const u = KNOWN_USERS.find((x) => x.id === recipientId);
    if (!u) throw new Error('Пользователь не найден в демо-мире');
    const existing = DMS.find((c) => c.recipients?.some((r) => r.id === u.id));
    if (existing) return { ...existing };
    const channel: Channel = {
      id: this.nextId(),
      type: 1,
      name: null,
      recipients: [u],
      last_message_id: null,
    };
    DMS.push(channel);
    this.emit({ type: 'dispatch', t: 'CHANNEL_CREATE', d: { ...channel } });
    return { ...channel };
  }

  async listRelationships(): Promise<Relationship[]> {
    return this.relationships.map((r) => ({ ...r }));
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

  async sendMessage(channelId: string, content: string, replyTo?: string): Promise<Message> {
    const message = msg(this.seq++, 0, ME, content, channelId);
    message.id = this.nextId();
    // Ответ: проставляем ссылку и вложенное сообщение — как реальный gateway.
    if (replyTo) {
      const parent = (this.histories[channelId] ?? []).find((m) => m.id === replyTo) ?? null;
      message.message_reference = { message_id: replyTo, channel_id: channelId };
      message.referenced_message = parent ? { ...parent } : null;
    }
    (this.histories[channelId] ??= []).push(message);
    // A friendly member sometimes reacts to us.
    if (Math.random() < 0.5) {
      this.timers.push(
        setTimeout(() => {
          message.reactions = [{ count: 1, me: true, emoji: { id: null, name: '🦀' } }];
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
      d: {
        channel_id: channelId,
        message_id: messageId,
        user_id: ME.id,
        // Кастомный эмодзи приходит как «name:id» — разбираем обратно.
        emoji: parseApiEmoji(emoji),
      },
    });
  }

  async removeReaction(channelId: string, messageId: string, emoji: string): Promise<void> {
    this.emit({
      type: 'dispatch',
      t: 'MESSAGE_REACTION_REMOVE',
      d: {
        channel_id: channelId,
        message_id: messageId,
        user_id: ME.id,
        emoji: parseApiEmoji(emoji),
      },
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

  /* ---------- v0.6.0: подписки, обновления, файлы ---------- */

  async subscribeGuild(_guildId: string): Promise<void> {
    /* демо-режим: все события и так приходят из локальной симуляции */
  }

  async checkUpdate(): Promise<UpdateInfo | null> {
    // В демо обновлений нет — как в самой свежей сборке ;)
    return {
      current: '0.6.0',
      latest: '0.6.0',
      is_newer: false,
      url: 'https://github.com/VinylRUS/Rustycord/releases/latest',
      notes: '',
      published_at: '',
      no_releases: false,
    };
  }

  async uploadFiles(_channelId: string, _paths: string[], _content?: string): Promise<Message> {
    throw new Error('В демо-режиме загрузка файлов недоступна — запусти приложение');
  }

  async getLogPath(): Promise<string> {
    return 'демо-режим: логи пишутся только в приложении';
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
      d: { guild_id: guildId, channel_id: channelId, user_id: ME.id, self_mute: selfMute, self_deaf: selfDeaf, session_id: 'demo-voice-1' },
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

  /* ---------- user auth (демо: недоступно, только в Tauri-режиме) ---------- */

  async loginEmail(): Promise<LoginResult> {
    throw new Error('Демо-режим: вход по email доступен только в приложении (Tauri)');
  }

  async submitMfa(): Promise<LoginResult> {
    throw new Error('Демо-режим: 2FA доступна только в приложении (Tauri)');
  }

  async startQr(): Promise<void> {
    // Симулируем QR-флоу v2: подключение → QR → отсканировано (с профилем) → токен.
    this.emit({ type: 'qr', stage: 'connecting', message: 'Подключаемся к Discord…' });
    await delay(500);
    this.emit({ type: 'qr', stage: 'fingerprint', svg: demoQrSvg(), fingerprint: 'demo-fingerprint' });
    await delay(1200);
    this.emit({ type: 'qr', stage: 'scanned', username: 'alexey.dev', userId: '1301000000000000201' });
    await delay(800);
    this.emit({ type: 'qr', stage: 'done', token: 'demo-token' });
  }

  async cancelQr(): Promise<void> {
    /* no-op */
  }

  /* ---------- generic REST (мини-эмуляция для настроек сервера) ---------- */

  async apiRequest(method: string, path: string, body?: unknown): Promise<unknown> {
    const m = method.toUpperCase();
    // ---- друзья (v0.4.0): зеркалим семантику /users/@me/relationships ----
    if (path === '/users/@me/relationships') {
      if (m === 'POST') {
        const username = String((body as { username?: string } | undefined)?.username ?? '').toLowerCase();
        const target = KNOWN_USERS.find((u) => u.username.toLowerCase() === username);
        if (!target) {
          throw new Error('Unknown User: в демо известны только участники сервера — попробуй alexey.dev или melody');
        }
        if (this.relationships.some((r) => r.user.id === target.id)) {
          throw new Error('Relationship already exists: вы уже друзья или заявка уже отправлена');
        }
        const rel: Relationship = { id: target.id, type: 4, user: target, nickname: null };
        this.relationships.push(rel);
        this.emit({ type: 'dispatch', t: 'RELATIONSHIP_ADD', d: { ...rel } });
        // Демо-жизнь: через пару секунд собеседник принимает заявку.
        this.timers.push(
          setTimeout(() => {
            if (!this.running) return;
            this.relationships = this.relationships.filter((r) => r.user.id !== target.id);
            this.emit({ type: 'dispatch', t: 'RELATIONSHIP_REMOVE', d: { id: target.id, type: 4 } });
            const friend: Relationship = { id: target.id, type: 1, user: target, nickname: null };
            this.relationships.push(friend);
            this.emit({ type: 'dispatch', t: 'RELATIONSHIP_ADD', d: { ...friend } });
          }, 3000),
        );
        return null;
      }
      throw new Error(`Демо-режим: ${method} ${path} не поддерживается`);
    }
    if (/^\/users\/@me\/relationships\/.+$/.test(path)) {
      const userId = path.split('/').pop() ?? '';
      const rel = this.relationships.find((r) => r.user.id === userId);
      if (m === 'PUT') {
        const type = Number((body as { type?: number } | undefined)?.type ?? 1) as Relationship['type'];
        if (!rel) throw new Error('Relationship not found');
        rel.type = type;
        this.emit({ type: 'dispatch', t: 'RELATIONSHIP_ADD', d: { ...rel } });
        return null;
      }
      if (m === 'DELETE') {
        this.relationships = this.relationships.filter((r) => r.user.id !== userId);
        this.emit({ type: 'dispatch', t: 'RELATIONSHIP_REMOVE', d: { id: userId } });
        return null;
      }
      throw new Error(`Демо-режим: ${method} ${path} не поддерживается`);
    }
    if (/\/bans$/.test(path) && m === 'GET') return [];
    // ---- v0.6.0: пины, поиск, профили, статус ----
    if (m === 'GET' && /\/channels\/[^/]+\/pins$/.test(path)) {
      const id = path.split('/')[2];
      return Object.values(this.histories)
        .flat()
        .filter((x) => x.channel_id === id && x.pinned)
        .map((x) => ({ ...x }));
    }
    if ((m === 'PUT' || m === 'DELETE') && /\/channels\/[^/]+\/pins\/.+$/.test(path)) {
      const id = path.split('/')[2];
      const mid = path.split('/').pop() ?? '';
      const target = (this.histories[id] ?? []).find((x) => x.id === mid);
      if (target) target.pinned = m === 'PUT';
      return null;
    }
    if (m === 'GET' && /\/channels\/[^/]+\/messages\/search/.test(path)) {
      const id = path.split('/')[2];
      const q = decodeURIComponent(path.split('query=')[1] ?? '').toLowerCase();
      const hits = (this.histories[id] ?? []).filter((x) => x.content.toLowerCase().includes(q));
      return { messages: hits.map((x) => [{ ...x }]), total_results: hits.length };
    }
    if (m === 'GET' && /^\/users\/[^/]+\/profile$/.test(path)) {
      const uid = path.split('/')[2];
      const u = KNOWN_USERS.find((x) => x.id === uid) ?? ME;
      const profile: UserProfile & { user: User } = {
        user: u,
        bio: 'Демо-профиль: био, баннер и общие сервера приходят из /users/{id}/profile.',
        banner: null,
        premium_type: u.id === MARINA.id ? 2 : 0,
        mutual_guilds: [{ id: GUILD_ID, nick: null }],
        mutual_friends_count: 3,
      };
      return profile;
    }
    if (m === 'PATCH' && path === '/users/@me/settings') {
      return { status: (body as { status?: string } | undefined)?.status ?? 'online' };
    }
    if (m === 'POST' && /^\/invites\/.+/.test(path)) {
      // Демо: вступление по приглашению — GUILD_CREATE приезжает как в реальном клиенте.
      const newGuild: Guild = {
        id: '1303000000000000001',
        name: 'Discord API',
        icon: null,
        owner_id: ALEXEY.id,
        roles: [],
        channels: [
          { id: '1303000000000000011', type: 0, guild_id: '1303000000000000001', name: 'общий', position: 0, parent_id: null, topic: 'Новый сервер через демо-приглашение' },
        ],
        voice_states: [],
        presences: [],
        member_count: 1,
        large: false,
      };
      this.emit({ type: 'dispatch', t: 'GUILD_CREATE', d: newGuild });
      return { guild: { name: newGuild.name } };
    }
    if (/\/invites$/.test(path) && m === 'GET') {
      return [{ code: 'demo24h', channel: { id: CH_GENERAL, name: 'основной' }, inviter: ALEXEY, uses: 3, max_uses: 0, expires_at: null, temporary: false }];
    }
    if (m === 'PATCH' && /^\/channels\//.test(path)) {
      const id = path.split('/')[2];
      const name = (body as { name?: string } | undefined)?.name;
      const ch = GUILD.channels?.find((c) => c.id === id);
      if (ch && name) {
        ch.name = name;
        this.emit({ type: 'dispatch', t: 'CHANNEL_UPDATE', d: { ...ch } });
        return ch;
      }
      return {};
    }
    if (m === 'POST' && /^\/guilds\/.+\/channels$/.test(path)) {
      const b = (body ?? {}) as { name?: string; type?: number; parent_id?: string | null };
      const channel: Channel = {
        id: this.nextId(),
        type: (b.type ?? 0) as Channel['type'],
        guild_id: GUILD_ID,
        name: b.name ?? 'новый-канал',
        position: 99,
        parent_id: b.parent_id ?? null,
      };
      GUILD.channels?.push(channel);
      this.emit({ type: 'dispatch', t: 'CHANNEL_CREATE', d: channel });
      return channel;
    }
    if (m === 'DELETE' && /^\/channels\//.test(path)) {
      const id = path.split('/')[2];
      const ch = GUILD.channels?.find((c) => c.id === id);
      if (GUILD.channels) GUILD.channels = GUILD.channels.filter((c) => c.id !== id);
      if (ch) this.emit({ type: 'dispatch', t: 'CHANNEL_DELETE', d: ch });
      return {};
    }
    if (m === 'PATCH' && /^\/guilds\/[^/]+$/.test(path)) {
      const name = (body as { name?: string } | undefined)?.name;
      if (name) this.emit({ type: 'dispatch', t: 'GUILD_UPDATE', d: { ...GUILD, name } });
      return { ok: true };
    }
    throw new Error(`Демо-режим: ${method} ${path} не поддерживается (нужен Tauri + реальный аккаунт)`);
  }

  /* ---------- plugins (in-memory, чтобы UI можно было потрогать) ---------- */

  private pluginStore: Map<string, { code: string; enabled: boolean; meta: PluginMeta | null }> = new Map([
    ['silent-typing.js', { code: DEMO_PLUGIN_SILENT_TYPING, enabled: true, meta: { name: 'Silent Typing', version: '1.0.0', author: 'RustyCord', description: 'Не отправлять индикатор «печатает…»' } }],
  ]);

  async listPlugins(): Promise<PluginInfo[]> {
    return [...this.pluginStore.entries()].map(([file, v]) => ({ file, enabled: v.enabled, meta: v.meta, size: v.code.length }));
  }

  async savePlugin(name: string, code: string): Promise<void> {
    this.pluginStore.set(name, { code, enabled: false, meta: null });
  }

  async deletePlugin(name: string): Promise<void> {
    this.pluginStore.delete(name);
  }

  async setPluginEnabled(name: string, enabled: boolean): Promise<void> {
    const v = this.pluginStore.get(name);
    if (v) v.enabled = enabled;
  }

  async setPluginMeta(name: string, meta: PluginMeta): Promise<void> {
    const v = this.pluginStore.get(name);
    if (v) v.meta = meta;
  }

  async readPlugin(name: string): Promise<string> {
    return this.pluginStore.get(name)?.code ?? '';
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
          // someone reacts — каждый раз разными эмодзи, как живой чат
          this.timers.push(
            setTimeout(() => {
              if (!this.running) return;
              const pool = ['👍', '🔥', '😂', '🦀', '💯', '🎉', '👀'];
              const emoji = { id: null, name: pool[Math.floor(Math.random() * pool.length)] };
              message.reactions = [{ count: 1, me: false, emoji }];
              this.emit({
                type: 'dispatch',
                t: 'MESSAGE_REACTION_ADD',
                d: { channel_id: item.channel, message_id: message.id, user_id: ALEXEY.id, emoji },
              });
            }, 2200),
          );
        }, 2600),
      );
      // presence flip mid-step: гильдейский (Дмитрий) и дружеский (Марина)
      this.timers.push(
        setTimeout(() => {
          if (!this.running) return;
          const statuses = ['online', 'dnd'] as const;
          const next = statuses[step % 2];
          this.emit({ type: 'dispatch', t: 'PRESENCE_UPDATE', d: { user: { id: DMITRY.id }, status: next, activities: [] } });
          const marinaNext = step % 2 ? 'idle' : 'online';
          this.emit({
            type: 'dispatch',
            t: 'PRESENCE_UPDATE',
            d: { user: { id: MARINA.id }, status: marinaNext, activities: [] },
          });
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
