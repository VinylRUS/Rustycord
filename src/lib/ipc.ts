/**
 * IPC layer — the single seam between UI and the Rust core.
 *
 * Two implementations share this interface:
 *  - `TauriBackend`  → invoke()/listen() over Tauri IPC to the Rust process
 *  - `MockBackend`   → pure-TypeScript simulation of the Discord Gateway,
 *                      lets the UI run in any browser without a token
 *                      (and doubles as a protocol playground).
 *
 * This mirrors what the official client does with its "native modules" —
 * except our native layer is Rust, not a Chromium Embedded Framework.
 */
import type {
  BackendEvent,
  Channel,
  LoginResult,
  Message,
  PluginInfo,
  PluginMeta,
  Relationship,
  SnowflakeInfo,
  StateSnapshot,
  UpdateInfo,
  User,
} from './types';

export interface GetMessageOptions {
  limit?: number;
  before?: string;
}

export interface Backend {
  readonly mode: 'tauri' | 'mock';

  // lifecycle
  connect(token: string): Promise<User>;
  disconnect(): Promise<void>;
  getState(): Promise<StateSnapshot>;

  // user auth (v0.3.0)
  loginEmail(email: string, password: string): Promise<LoginResult>;
  submitMfa(code: string, ticket: string): Promise<LoginResult>;
  startQr(): Promise<void>;
  cancelQr(): Promise<void>;

  // REST-ish
  getMessages(channelId: string, opts?: GetMessageOptions): Promise<Message[]>;
  sendMessage(channelId: string, content: string, replyTo?: string): Promise<Message>;
  editMessage(channelId: string, messageId: string, content: string): Promise<Message>;
  deleteMessage(channelId: string, messageId: string): Promise<void>;
  addReaction(channelId: string, messageId: string, emoji: string): Promise<void>;
  removeReaction(channelId: string, messageId: string, emoji: string): Promise<void>;
  triggerTyping(channelId: string): Promise<void>;
  requestMembers(guildId: string): Promise<void>;
  /** Escape hatch: отправить сырой JSON-кадр в gateway (протокольные эксперименты). */
  gatewaySendRaw(frame: string): Promise<void>;
  kickMember(guildId: string, userId: string): Promise<void>;
  banMember(guildId: string, userId: string): Promise<void>;
  /** op 4 — войти/выйти из войс-канала; guildId = null → звонок в ЛС/группе;
   *  channelId = null → покинуть. selfVideo — флаг камеры. */
  joinVoice(guildId: string | null, channelId: string | null, selfMute: boolean, selfDeaf: boolean, selfVideo?: boolean): Promise<void>;
  /** op 18 — начать трансляцию экрана/камеры (экспериментально). */
  startStream(streamType: 'screen' | 'camera', guildId: string, channelId: string): Promise<void>;
  /** op 20 — прекратить трансляцию. */
  stopStream(streamKey: string): Promise<void>;
  /** op 22 — пауза трансляции. */
  pauseStream(streamKey: string, paused: boolean): Promise<void>;

  // v0.6.0: подписки, обновления, файлы
  /** Юзер-сессии: op 37 + op 14 на гильдию (вызывается при её открытии). */
  subscribeGuild(guildId: string): Promise<void>;
  /** Проверка релизов на GitHub — null, если проверить не удалось. */
  checkUpdate(): Promise<UpdateInfo | null>;
  /** Загрузка файлов в канал (multipart). paths — абсолютные пути из диалога. */
  uploadFiles(channelId: string, paths: string[], content?: string): Promise<Message>;
  /** Путь к файлу логов (для баг-репортов). */
  getLogPath(): Promise<string>;

  // друзья и личные беседы (v0.4.0)
  /** Открыть (или создать) DM с пользователем — POST /users/@me/channels. */
  createDm(recipientId: string): Promise<Channel>;
  /** REST-синхронизация друзей/заявок/блок-листа. */
  listRelationships(): Promise<Relationship[]>;

  /** Универсальный REST-прокси (управление сервером + плагины). */
  apiRequest(method: string, path: string, body?: unknown): Promise<unknown>;

  // plugins (v0.3.0)
  listPlugins(): Promise<PluginInfo[]>;
  savePlugin(name: string, code: string): Promise<void>;
  deletePlugin(name: string): Promise<void>;
  setPluginEnabled(name: string, enabled: boolean): Promise<void>;
  setPluginMeta(name: string, meta: PluginMeta): Promise<void>;
  readPlugin(name: string): Promise<string>;

  // voice
  joinVoice(guildId: string | null, channelId: string | null, selfMute: boolean, selfDeaf: boolean): Promise<void>;

  // tools
  parseSnowflake(id: string): Promise<SnowflakeInfo>;

  // unified event stream (status / ready / dispatch / voice / warning / qr)
  onEvent(handler: (ev: BackendEvent) => void): () => void;
}

/* ------------------------------------------------------------------ */
/* Tauri implementation                                                */
/* ------------------------------------------------------------------ */

class TauriBackend implements Backend {
  readonly mode = 'tauri' as const;

  private async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }

  async connect(token: string): Promise<User> {
    return this.invoke<User>('connect', { token });
  }

  async loginEmail(email: string, password: string): Promise<LoginResult> {
    return this.invoke<LoginResult>('auth_login_email', { email, password });
  }

  async submitMfa(code: string, ticket: string): Promise<LoginResult> {
    return this.invoke<LoginResult>('auth_submit_mfa', { code, ticket });
  }

  async startQr(): Promise<void> {
    await this.invoke('auth_start_qr');
  }

  async cancelQr(): Promise<void> {
    await this.invoke('auth_cancel_qr');
  }

  async disconnect(): Promise<void> {
    await this.invoke('disconnect');
  }

  async getState(): Promise<StateSnapshot> {
    return this.invoke<StateSnapshot>('get_state');
  }

  async getMessages(channelId: string, opts?: GetMessageOptions): Promise<Message[]> {
    return this.invoke<Message[]>('get_messages', {
      channelId,
      limit: opts?.limit ?? 50,
      before: opts?.before ?? null,
    });
  }

  async sendMessage(channelId: string, content: string, replyTo?: string): Promise<Message> {
    return this.invoke<Message>('send_message', { channelId, content, replyTo: replyTo ?? null });
  }

  async editMessage(channelId: string, messageId: string, content: string): Promise<Message> {
    return this.invoke<Message>('edit_message', { channelId, messageId, content });
  }

  async deleteMessage(channelId: string, messageId: string): Promise<void> {
    await this.invoke('delete_message', { channelId, messageId });
  }

  async addReaction(channelId: string, messageId: string, emoji: string): Promise<void> {
    await this.invoke('add_reaction', { channelId, messageId, emoji });
  }

  async removeReaction(channelId: string, messageId: string, emoji: string): Promise<void> {
    await this.invoke('remove_reaction', { channelId, messageId, emoji });
  }

  async triggerTyping(channelId: string): Promise<void> {
    await this.invoke('trigger_typing', { channelId });
  }

  async requestMembers(guildId: string): Promise<void> {
    await this.invoke('request_members', { guildId });
  }

  async gatewaySendRaw(frame: string): Promise<void> {
    await this.invoke('gateway_send_raw', { frame });
  }

  async kickMember(guildId: string, userId: string): Promise<void> {
    await this.invoke('kick_member', { guildId, userId });
  }

  async banMember(guildId: string, userId: string): Promise<void> {
    await this.invoke('ban_member', { guildId, userId });
  }

  async createDm(recipientId: string): Promise<Channel> {
    return this.invoke<Channel>('create_dm', { recipientId });
  }

  async listRelationships(): Promise<Relationship[]> {
    return this.invoke<Relationship[]>('list_relationships');
  }

  async apiRequest(method: string, path: string, body?: unknown): Promise<unknown> {
    return this.invoke('api_request', { method, path, body: body ?? null });
  }

  async listPlugins(): Promise<PluginInfo[]> {
    return this.invoke<PluginInfo[]>('plugin_list');
  }

  async savePlugin(name: string, code: string): Promise<void> {
    await this.invoke('plugin_save', { name, code });
  }

  async deletePlugin(name: string): Promise<void> {
    await this.invoke('plugin_delete', { name });
  }

  async setPluginEnabled(name: string, enabled: boolean): Promise<void> {
    await this.invoke('plugin_set_enabled', { name, enabled });
  }

  async setPluginMeta(name: string, meta: PluginMeta): Promise<void> {
    await this.invoke('plugin_set_meta', { name, meta });
  }

  async readPlugin(name: string): Promise<string> {
    return this.invoke<string>('plugin_read', { name });
  }

  async joinVoice(guildId: string | null, channelId: string | null, selfMute: boolean, selfDeaf: boolean, selfVideo?: boolean): Promise<void> {
    await this.invoke('join_voice', { guildId, channelId, selfMute, selfDeaf, selfVideo });
  }

  async startStream(streamType: 'screen' | 'camera', guildId: string, channelId: string): Promise<void> {
    await this.invoke('start_stream', { streamType, guildId, channelId });
  }

  async stopStream(streamKey: string): Promise<void> {
    await this.invoke('stop_stream', { streamKey });
  }

  async pauseStream(streamKey: string, paused: boolean): Promise<void> {
    await this.invoke('pause_stream', { streamKey, paused });
  }

  async parseSnowflake(id: string): Promise<SnowflakeInfo> {
    return this.invoke<SnowflakeInfo>('parse_snowflake', { id });
  }

  async subscribeGuild(guildId: string): Promise<void> {
    await this.invoke('subscribe_guild', { guildId });
  }

  async checkUpdate(): Promise<UpdateInfo | null> {
    try {
      return await this.invoke<UpdateInfo>('check_update');
    } catch {
      return null; // без сети/на дев-сборке — молча
    }
  }

  async uploadFiles(channelId: string, paths: string[], content?: string): Promise<Message> {
    return this.invoke<Message>('upload_files', { channelId, paths, content: content ?? null });
  }

  async getLogPath(): Promise<string> {
    return this.invoke<string>('get_log_path');
  }

  onEvent(handler: (ev: BackendEvent) => void): () => void {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    const bind = async () => {
      const { listen } = await import('@tauri-apps/api/event');
      const subs: Array<[string, (payload: unknown) => void]> = [
        ['discord://status', (d) => handler({ type: 'status', ...(d as object) } as BackendEvent)],
        ['discord://ready', (d) => handler({ type: 'ready', ...(d as object) } as BackendEvent)],
        ['discord://dispatch', (d) => handler({ type: 'dispatch', ...(d as object) } as BackendEvent)],
        ['discord://voice', (d) => handler({ type: 'voice', ...(d as object) } as BackendEvent)],
        ['discord://warning', (d) => handler({ type: 'warning', ...(d as object) } as BackendEvent)],
        ['discord://qr', (d) => handler({ type: 'qr', ...(d as object) } as BackendEvent)],
      ];
      for (const [event, fn] of subs) {
        const un = await listen(event, (e) => fn(e.payload));
        if (disposed) {
          un();
        } else {
          unlisteners.push(un);
        }
      }
    };
    bind().catch((err) => console.error('[tauri] event bind failed', err));

    return () => {
      disposed = true;
      for (const un of unlisteners) un();
    };
  }
}

/* ------------------------------------------------------------------ */
/* Factory                                                             */
/* ------------------------------------------------------------------ */

export async function createBackend(mode: 'tauri' | 'mock'): Promise<Backend> {
  if (mode === 'tauri') return new TauriBackend();
  const { MockBackend } = await import('./mock');
  return new MockBackend();
}
