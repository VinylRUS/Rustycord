/**
 * TypeScript mirrors of Discord API models (v10).
 * Field names intentionally stay snake_case to match the wire format 1:1 —
 * this is a reverse-engineering project, we keep the protocol shapes visible.
 */

export type PresenceStatus = 'online' | 'idle' | 'dnd' | 'offline' | 'invisible';

export interface User {
  id: string;
  username: string;
  discriminator: string; // '0' for the new username system
  global_name: string | null;
  avatar: string | null;
  banner?: string | null;
  accent_color?: number | null;
  bot?: boolean;
  /** BIO из профиля (USER_UPDATE / PATCH /users/@me). */
  bio?: string;
}

export interface Emoji {
  id: string | null; // custom emoji id, null for unicode
  name: string | null;
  animated?: boolean;
}

export interface Reaction {
  count: number;
  me: boolean;
  emoji: Emoji;
}

export interface Activity {
  name: string;
  type: number;
  state?: string | null;
  details?: string | null;
}

export interface Presence {
  user: { id: string; username?: string; discriminator?: string; global_name?: string | null; avatar?: string | null };
  status: PresenceStatus;
  activities?: Activity[];
}

export interface Member {
  user?: User | null;
  nick: string | null;
  avatar?: string | null;
  roles: string[]; // role ids
  joined_at?: string | null;
  premium_since?: string | null;
  deaf?: boolean;
  mute?: boolean;
  pending?: boolean;
  communication_disabled_until?: string | null;
}

export interface Role {
  id: string;
  name: string;
  color: number; // 0 = default
  hoist: boolean;
  icon?: string | null;
  position: number;
  permissions: string; // bitfield as string
  managed?: boolean;
  mentionable?: boolean;
}

export type ChannelType = 0 | 1 | 2 | 3 | 4 | 5 | 11 | 12 | 13 | 15;

/** Перезапись прав на канал: id — роль (type 0, для @everyone id = guild_id)
 * или участник (type 1); allow/deny — битовые маски строкой. */
export interface Overwrite {
  id: string;
  type: number;
  allow: string;
  deny: string;
}

/** Метаданные треда (type 11/12): архив, авто-архив, блокировка. */
export interface ThreadMetadata {
  archived: boolean;
  auto_archive_duration: number;
  archive_timestamp: string;
  locked?: boolean;
  create_timestamp?: string | null;
}

export interface Channel {
  id: string;
  type: ChannelType;
  guild_id?: string | null;
  name?: string | null;
  topic?: string | null;
  position?: number;
  nsfw?: boolean;
  last_message_id?: string | null;
  parent_id?: string | null;
  rate_limit_per_user?: number;
  // DM channels: полный список юзеров (REST) либо только айдишники
  // (юзер-READY шлёт recipient_ids — полные объекты в отдельном массиве users).
  recipients?: User[];
  recipient_ids?: string[];
  owner_id?: string;
  icon?: string | null;
  /** Перезаписи прав канала — по ним UI скрывает каналы, недоступные роли. */
  permission_overwrites?: Overwrite[];
  /** Треды (type 11/12): архив/авто-архив/блокировка. */
  thread_metadata?: ThreadMetadata;
}

export interface VoiceState {
  guild_id?: string | null;
  channel_id?: string | null;
  user_id: string;
  session_id?: string;
  deaf?: boolean;
  mute?: boolean;
  self_deaf?: boolean;
  self_mute?: boolean;
  self_video?: boolean;
  suppress?: boolean;
}

export interface Guild {
  id: string;
  name: string;
  icon: string | null;
  banner?: string | null;
  owner_id: string;
  roles: Role[];
  /** Кастомные эмодзи сервера (для пикера реакций). */
  emojis?: Emoji[];
  channels?: Channel[];
  /** Активные треды гильдии (type 11/12) — из GUILD_CREATE/THREAD_LIST_SYNC. */
  threads?: Channel[];
  members?: Member[];
  presences?: Presence[];
  voice_states?: VoiceState[];
  member_count?: number;
  large?: boolean;
  unavailable?: boolean;
}

export interface Attachment {
  id: string;
  filename: string;
  size: number;
  url: string;
  content_type?: string;
  width?: number;
  height?: number;
}

export interface MessageEmbed {
  type?: string;
  title?: string;
  description?: string;
  url?: string;
  color?: number | null;
  author?: { name?: string; url?: string; icon_url?: string };
  image?: { url?: string; proxy_url?: string; width?: number; height?: number } | null;
  thumbnail?: { url?: string; proxy_url?: string; width?: number; height?: number } | null;
  video?: { url?: string; width?: number; height?: number } | null;
  footer?: { text?: string; icon_url?: string } | null;
  fields?: { name: string; value: string; inline?: boolean }[];
}

/** Стикер в сообщении (Lottie рендерим как заглушку, PNG/APNG — как img). */
export interface StickerItem {
  id: string;
  name: string;
  /** 1 = PNG, 2 = APNG, 3 = LOTTIE. */
  format_type?: number;
}

export interface Message {
  id: string;
  channel_id: string;
  guild_id?: string | null;
  author: User;
  member?: Member | null;
  content: string;
  timestamp: string; // NOTE: absent in gateway MESSAGE_CREATE in some cases — snowflake of `id` is the source of truth
  edited_timestamp: string | null;
  tts: boolean;
  mention_everyone: boolean;
  mentions: User[];
  mention_roles?: string[];
  attachments: Attachment[];
  embeds: MessageEmbed[];
  sticker_items?: StickerItem[];
  /** Ссылка на сообщение-родитель (ответ). */
  message_reference?: { message_id?: string; channel_id?: string; guild_id?: string } | null;
  /** Присутствует в REST-выдаче и в gateway-эхе своего ответа. */
  referenced_message?: Message | null;
  reactions?: Reaction[] | null;
  pinned: boolean;
  type: number;
}

/** Расширенный профиль (GET /users/{id}/profile — только юзер-токены). */
export interface UserProfile {
  bio?: string;
  banner?: string | null;
  accent_color?: number | null;
  premium_type?: number;
  premium_since?: string | null;
  mutual_guilds?: { id: string; nick?: string | null }[];
  mutual_friends_count?: number;
  user?: User;
}

/** Результат проверки обновлений (Rust check_update / plugin-updater). */
export interface UpdateInfo {
  current: string;
  latest: string;
  is_newer: boolean;
  url: string;
  notes: string;
  published_at: string;
  no_releases: boolean;
  /** Платформа умеет ставить обновление «на месте» (Windows/macOS/AppImage). */
  can_auto?: boolean;
  /** Откуда пришла информация: подписанный latest.json или GitHub API. */
  source?: 'updater' | 'github';
}

/* ---------- Our own IPC payload types ---------- */

export interface SnowflakeInfo {
  id: string;
  timestamp_ms: number;
  created_at: string; // RFC3339
  worker_id: number;
  process_id: number;
  increment: number;
  age_seconds: number;
}

export type ConnStage = 'connecting' | 'connected' | 'reconnecting' | 'disconnected' | 'shutdown';

/* ---------- User auth (v0.3.0) ---------- */

/** Ответ /auth/login: сразу токен или запрос 2FA-кода. */
export interface LoginResult {
  status: 'ok' | 'mfa';
  token?: string;
  ticket?: string;
  sms?: boolean;
}

export type QrStage = 'connecting' | 'fingerprint' | 'scanned' | 'done' | 'cancelled' | 'timeout' | 'error';

export interface QrEvent {
  stage: QrStage;
  svg?: string;
  fingerprint?: string;
  token?: string;
  /** pending_ticket: кто отсканировал QR (id:disc:avatar:username). */
  username?: string;
  userId?: string;
  message?: string;
}


/* ---------- Guild management ---------- */

export interface BanEntry {
  reason: string | null;
  user: User;
}

export interface Invite {
  code: string;
  channel: { id: string; name?: string } | null;
  inviter: User | null;
  uses: number;
  max_uses: number;
  expires_at: string | null;
  temporary: boolean;
}

/* ---------- Friends / relationships (v0.4.0) ---------- */

/** Тип отношения из протокола: 1 друг · 2 в блок-листе · 3 входящая · 4 исходящая заявка. */
export type RelationshipType = 1 | 2 | 3 | 4;

export interface Relationship {
  id: string; // = user.id
  type: RelationshipType;
  user: User;
  nickname?: string | null;
}

/* ---------- Plugins (v0.3.0, Vencord-style) ---------- */

export interface PluginMeta {
  name?: string;
  version?: string;
  author?: string;
  description?: string;
}

export interface PluginInfo {
  file: string;
  enabled: boolean;
  meta: PluginMeta | null;
  size: number;
}

export type PluginEventName =
  | 'ready'
  | 'dispatch'
  | 'message:create'
  | 'message:update'
  | 'message:delete'
  | 'message:send' // хук перед отправкой: может вернуть string|false
  | 'guild:create'
  | 'guild:update'
  | 'guild:delete'
  | 'voice:state';

/** Папка серверов в рейле (user_settings: proto или legacy guild_folders). */
export interface GuildFolder {
  id: string | null;
  name: string | null;
  /** RGB-цвет папки (int) — null для дефолтного серого. */
  color: number | null;
  guild_ids: string[];
}

/** Элемент порядка рейла: отдельная гильдия либо папка (индекс в folders). */
export type RailOrderItem =
  | { type: 'guild'; id: string }
  | { type: 'folder'; index: number };

/** Строка ленивого списка участников (GUILD_MEMBER_LIST_UPDATE от Rust).
 *  Сервер присылает сайдбар уже собранным: заголовки групп и участников
 *  вперемешку в готовом порядке — 1:1 с официальным клиентом. */
export type MemberListRowData =
  | { kind: 'group'; id: string; count: number }
  | {
      kind: 'member';
      user_id: string;
      username: string;
      global_name?: string | null;
      nick?: string | null;
      discriminator?: string | null;
      avatar?: string | null;
      bot?: boolean;
      status: string;
      activities?: Array<{ type: number; name?: string; state?: string }>;
      roles: string[];
      hoisted_role?: string | null;
    };

/** Снимок ленивого списка участников гильдии (по текущему каналу). */
export interface MemberListData {
  channel_id: string;
  member_count: number;
  online_count: number;
  rows: MemberListRowData[];
}

export type BackendEvent =
  | { type: 'status'; stage: ConnStage; detail?: string }
  | { type: 'ready'; user: User; session_id: string }
  | { type: 'dispatch'; t: string; d: unknown }
  | {
      /** Снапшот ленивого списка участников (после op 37 с channels).
       *  rows — готовый сайдбар: группы + участники с пресенсами. */
      type: 'memberList';
      guild_id: string;
      channel_id: string;
      member_count: number;
      online_count: number;
      rows: MemberListRowData[];
    }
  | {
      /** Раскладка рейла: папки + порядок отдельных гильдий (Rust декодирует
       *  user_settings_proto/legacy и нормализует в единый формат).
       *  rail_order — готовый порядок рейла: отдельные гильдии и папки
       *  вперемешку (legacy хранит точный порядок массива). */
      type: 'guildLayout';
      folders: GuildFolder[];
      guild_positions: string[];
      rail_order?: RailOrderItem[];
    }
  | { type: 'qr'; stage: QrStage; svg?: string; fingerprint?: string; token?: string; username?: string; userId?: string; message?: string }
  | {
      type: 'voice';
      kind: 'connected' | 'disconnected' | 'speaking';
      data: {
        guild_id: string;
        user_id?: string;
        channel_id?: string | null;
        speaking?: boolean;
        mode?: string;
        ssrc?: number;
        endpoint?: string;
        ip?: string;
        port?: number;
      };
    }
  | { type: 'warning'; message: string };

export interface StateSnapshot {
  user: User | null;
  guilds: Guild[];
  dms: Channel[];
  relationships?: Relationship[];
  friend_presences?: Presence[];
}
