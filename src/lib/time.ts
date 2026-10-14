/**
 * Snowflake utilities — the reverse-engineered heart of Discord IDs.
 *
 * Every Discord object ID is a 64-bit integer where:
 *   bits 63..22 | timestamp (ms since 2015-01-01T00:00:00Z, the "Discord epoch")
 *   bits 21..17 | internal worker id
 *   bits 16..12 | internal process id
 *   bits 11..0  | per-process increment
 *
 * The API never sends `timestamp` in MESSAGE_CREATE — clients derive it
 * from the message id. So we do exactly what the official client does.
 */
const DISCORD_EPOCH = 1420070400000; // 2015-01-01T00:00:00.000Z

export function snowflakeToMs(id: string): number {
  const n = BigInt(id);
  return Number((n >> 22n) + BigInt(DISCORD_EPOCH));
}

export function snowflakeDate(id: string): Date {
  return new Date(snowflakeToMs(id));
}

const ru = 'ru-RU';

/** "Сегодня в 14:32" / "Вчера в 09:01" / "12.06.2025" */
export function formatStamp(idOrTs: string | number): string {
  const ts = typeof idOrTs === 'string' ? snowflakeToMs(idOrTs) : idOrTs;
  const d = new Date(ts);
  const now = new Date();
  const time = d.toLocaleTimeString(ru, { hour: '2-digit', minute: '2-digit' });
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  if (sameDay(d, now)) return `Сегодня в ${time}`;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(d, yesterday)) return `Вчера в ${time}`;
  const date = d.toLocaleDateString(ru, { day: '2-digit', month: '2-digit', year: 'numeric' });
  return `${date} в ${time}`;
}

/** Short time for grouped messages: "14:32" */
export function formatTime(idOrTs: string | number): string {
  const ts = typeof idOrTs === 'string' ? snowflakeToMs(idOrTs) : idOrTs;
  return new Date(ts).toLocaleTimeString(ru, { hour: '2-digit', minute: '2-digit' });
}

/** Date divider label: "12 июня 2025 г." / "Сегодня" / "Вчера" */
export function formatDivider(idOrTs: string | number): string {
  const ts = typeof idOrTs === 'string' ? snowflakeToMs(idOrTs) : idOrTs;
  const d = new Date(ts);
  const now = new Date();
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  if (sameDay(d, now)) return 'Сегодня';
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(d, yesterday)) return 'Вчера';
  return d.toLocaleDateString(ru, { day: 'numeric', month: 'long', year: 'numeric' });
}

/** Client-side mirror of the Rust `parse_snowflake` command (used in demo mode). */
export function parseSnowflakeLocal(id: string) {
  const n = BigInt(id);
  const timestampMs = Number((n >> 22n) + BigInt(DISCORD_EPOCH));
  return {
    id,
    timestamp_ms: timestampMs,
    created_at: new Date(timestampMs).toISOString(),
    worker_id: Number((n >> 17n) & 0x1fn),
    process_id: Number((n >> 12n) & 0x1fn),
    increment: Number(n & 0xfffn),
    age_seconds: Math.max(0, Math.floor((Date.now() - timestampMs) / 1000)),
  };
}

export function formatDuration(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  if (d > 0) return `${d} д ${h} ч`;
  if (h > 0) return `${h} ч ${m} мин`;
  if (m > 0) return `${m} мин ${s} с`;
  return `${s} с`;
}
