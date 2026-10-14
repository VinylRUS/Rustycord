/**
 * CDN URL builders. Discord serves all assets from cdn.discordapp.com —
 * reverse-engineered detail: avatars are content-hashed, so the same
 * hash always maps to the same immutable file (aggressive caching).
 */
import type { User, Channel, Guild } from './types';

const CDN = 'https://cdn.discordapp.com';

/** 5 default avatar colors (index derived from user id % 5). */
const DEFAULT_COLORS = ['#5865f2', '#757e8a', '#eb459e', '#57f287', '#fee75c'];

export function defaultAvatarColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return DEFAULT_COLORS[h % DEFAULT_COLORS.length];
}

export function userAvatarUrl(user: User, size = 64): string | null {
  if (user.avatar) {
    const ext = user.avatar.startsWith('a_') ? 'gif' : 'png';
    return `${CDN}/avatars/${user.id}/${user.avatar}.${ext}?size=${size}`;
  }
  return null;
}

export function memberAvatarUrl(guildId: string, userId: string, hash: string, size = 64): string | null {
  return `${CDN}/guilds/${guildId}/users/${userId}/avatars/${hash}.png?size=${size}`;
}

export function guildIconUrl(guild: Guild, size = 64): string | null {
  if (!guild.icon) return null;
  const ext = guild.icon.startsWith('a_') ? 'gif' : 'png';
  return `${CDN}/icons/${guild.id}/${guild.icon}.${ext}?size=${size}`;
}

export function guildBannerUrl(guild: Guild, size = 512): string | null {
  if (!guild.banner) return null;
  return `${CDN}/banners/${guild.id}/${guild.banner}.png?size=${size}`;
}

/** DM channels have no icon — we build a color chip from recipient ids. */
export function dmColor(channel: Channel): string {
  const ids = channel.recipients?.map((r) => Number(BigInt(r.id) % 5n)) ?? [0];
  return DEFAULT_COLORS[ids[0] ?? 0];
}

export function emojiUrl(emoji: {
  id: string | null;
  name: string | null;
  animated?: boolean;
}): string | null {
  if (!emoji.id) return null;
  // Анимированные кастомные эмодзи живут в .gif, статичные — в .png.
  return `${CDN}/emojis/${emoji.id}.${emoji.animated ? 'gif' : 'png'}`;
}

/** Display name: global_name → nick handled by caller → username */
export function displayName(user: User | null | undefined): string {
  if (!user) return 'Неизвестный';
  return user.global_name || user.username;
}

export function userTag(user: User): string {
  // New username system: discriminator '0' → plain @username
  return user.discriminator && user.discriminator !== '0'
    ? `${user.username}#${user.discriminator}`
    : `@${user.username}`;
}
