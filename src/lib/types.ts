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

export type ChannelType = 0 | 1 | 2 | 3 | 4 | 5 | 13;

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
  // DM channels
  recipients?: User[];
  owner_id?: string;
  icon?: string | null;
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
  channels?: Channel[];
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
  reactions?: Reaction[] | null;
  pinned: boolean;
  type: number;
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

export type BackendEvent =
  | { type: 'status'; stage: ConnStage; detail?: string }
  | { type: 'ready'; user: User; session_id: string }
  | { type: 'dispatch'; t: string; d: unknown }
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
}
