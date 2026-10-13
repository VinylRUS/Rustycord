/**
 * Discord permission bitfield — reverse-engineered straight from the API docs.
 * Permissions are stored as a 64-bit integer (sent as string in JSON)
 * inside each role; the effective permission set of a member is the
 * union of their roles' bits (plus @everyone bits), short-circuited by
 * the ADMINISTRATOR flag.
 */

export const PERMISSIONS = {
  CREATE_INSTANT_INVITE: 1n << 0n,
  KICK_MEMBERS: 1n << 1n,
  BAN_MEMBERS: 1n << 2n,
  ADMINISTRATOR: 1n << 3n,
  MANAGE_CHANNELS: 1n << 4n,
  MANAGE_GUILD: 1n << 5n,
  ADD_REACTIONS: 1n << 6n,
  VIEW_AUDIT_LOG: 1n << 7n,
  PRIORITY_SPEAKER: 1n << 8n,
  STREAM: 1n << 9n,
  VIEW_CHANNEL: 1n << 10n,
  SEND_MESSAGES: 1n << 11n,
  SEND_TTS_MESSAGES: 1n << 12n,
  MANAGE_MESSAGES: 1n << 13n,
  EMBED_LINKS: 1n << 14n,
  ATTACH_FILES: 1n << 15n,
  READ_MESSAGE_HISTORY: 1n << 16n,
  MENTION_EVERYONE: 1n << 17n,
  USE_EXTERNAL_EMOJIS: 1n << 18n,
  VIEW_GUILD_INSIGHTS: 1n << 19n,
  CONNECT: 1n << 20n,
  SPEAK: 1n << 21n,
  MUTE_MEMBERS: 1n << 22n,
  DEAFEN_MEMBERS: 1n << 23n,
  MOVE_MEMBERS: 1n << 24n,
  USE_VAD: 1n << 25n,
  CHANGE_NICKNAME: 1n << 26n,
  MANAGE_NICKNAMES: 1n << 27n,
  MANAGE_ROLES: 1n << 28n,
  MANAGE_WEBHOOKS: 1n << 29n,
  MANAGE_GUILD_EXPRESSIONS: 1n << 30n,
  USE_APPLICATION_COMMANDS: 1n << 31n,
  REQUEST_TO_SPEAK: 1n << 32n,
  MANAGE_EVENTS: 1n << 33n,
  MANAGE_THREADS: 1n << 34n,
  CREATE_PUBLIC_THREADS: 1n << 35n,
  CREATE_PRIVATE_THREADS: 1n << 36n,
  USE_EXTERNAL_STICKERS: 1n << 37n,
  SEND_MESSAGES_IN_THREADS: 1n << 38n,
  USE_EMBEDDED_ACTIVITIES: 1n << 39n,
  MODERATE_MEMBERS: 1n << 40n,
} as const;

export type PermissionKey = keyof typeof PERMISSIONS;

export const PERMISSION_LABELS_RU: Record<string, string> = {
  ADMINISTRATOR: 'Администратор',
  MANAGE_GUILD: 'Управление сервером',
  MANAGE_ROLES: 'Управление ролями',
  MANAGE_CHANNELS: 'Управление каналами',
  KICK_MEMBERS: 'Выгонять участников',
  BAN_MEMBERS: 'Банить участников',
  MODERATE_MEMBERS: 'Мьютить участников',
  MANAGE_MESSAGES: 'Управление сообщениями',
  VIEW_CHANNEL: 'Просмотр каналов',
  SEND_MESSAGES: 'Отправка сообщений',
  ADD_REACTIONS: 'Добавление реакций',
  READ_MESSAGE_HISTORY: 'История сообщений',
  MENTION_EVERYONE: 'Упоминание @everyone',
  CONNECT: 'Подключение к войсу',
  SPEAK: 'Говорить в войсе',
  MUTE_MEMBERS: 'Мьютить в войсе',
  MOVE_MEMBERS: 'Перемещать участников',
  MANAGE_WEBHOOKS: 'Управление вебхуками',
  CHANGE_NICKNAME: 'Смена ника',
};

/** Effective permission BigInt of a member in a guild. */
export function computePermissions(
  memberRoleIds: string[],
  roles: { id: string; permissions: string }[],
): bigint {
  let bits = 0n;
  const byId = new Map(roles.map((r) => [r.id, r.permissions]));
  for (const id of memberRoleIds) {
    const p = byId.get(id);
    if (p !== undefined) bits |= BigInt(p || '0');
  }
  // @everyone role has id == guild id; if not passed explicitly, treat as no base bits.
  return bits;
}

export function has(permBits: bigint, flag: bigint): boolean {
  if (permBits & PERMISSIONS.ADMINISTRATOR) return true;
  return (permBits & flag) === flag;
}

/** Human-readable summary for the profile card. */
export function summarizePermissions(permBits: bigint): string[] {
  const out: string[] = [];
  if (has(permBits, PERMISSIONS.ADMINISTRATOR)) out.push(PERMISSION_LABELS_RU.ADMINISTRATOR);
  for (const [key, label] of Object.entries(PERMISSION_LABELS_RU)) {
    if (key === 'ADMINISTRATOR') continue;
    if (has(permBits, PERMISSIONS[key as PermissionKey])) out.push(label);
  }
  return out.slice(0, 12);
}
