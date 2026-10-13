/**
 * Plugin system (v0.3.0) — Vencord-style, но проще.
 *
 * Плагин — обычный JS-файл, который вычисляется в объект-литерал:
 *
 *   ({
 *     name: 'My Plugin',
 *     version: '1.0.0',
 *     author: 'you',
 *     description: 'что делает',
 *     start(api) { ... },
 *     stop(api) { ... }
 *   })
 *
 * `api` — мост к клиенту: события, REST-прокси, CSS-инъекции, storage.
 * Код исполняется в том же webview, что и UI (как Vencord в Electron):
 * плагин может трогать DOM, но все сетевые запросы идут через Rust-слой
 * с общими rate limits и заголовками сессии.
 */
import type { Backend } from '../ipc';
import type { Message, PluginEventName, PluginInfo, PluginMeta } from '../types';

export interface PluginApi {
  /** Версия API плагинов. */
  readonly version: '1';
  /** Имя файла плагина. */
  readonly id: string;
  /** Подписка на события клиента. Авто-отписка при выключении плагина. */
  on(event: PluginEventName, handler: (payload: any) => void): void;
  /** REST-запрос к Discord API (относительный путь, напр. '/users/@me'). */
  rest(method: string, path: string, body?: unknown): Promise<unknown>;
  /** Инъекция CSS в документ (id изолирован по плагину). */
  addCss(css: string): void;
  /** Значок «печатает…» и прочие флаги поведения клиента. */
  suppress(flag: 'typing', value: boolean): void;
  /** Локальное хранилище плагина (localStorage с префиксом). */
  storage: {
    get<T>(key: string, fallback: T): T;
    set(key: string, value: unknown): void;
  };
  /** Тост-уведомление в углу UI. */
  toast(text: string): void;
  /** Текущий пользователь (null до ready). */
  me(): { id: string; username: string } | null;
}

interface RunningPlugin {
  file: string;
  meta: PluginMeta;
  instance: { start?: (api: PluginApi) => void | Promise<void>; stop?: (api: PluginApi) => void | Promise<void> } | null;
  api: PluginApi;
  handlers: Map<string, Set<(payload: unknown) => void>>;
  cssIds: string[];
}

type EventHandler = (payload: unknown) => void;
type SendHook = (content: string) => string | false | void | Promise<string | false | void>;

class PluginManager {
  private backend: Backend | null = null;
  private running = new Map<string, RunningPlugin>();
  /** Плагин → хук 'message:send' (может менять текст или отменять отправку). */
  private sendHooks = new Map<string, SendHook>();
  /** Флаги suppress (например, 'typing' у Silent Typing). */
  suppressions: Record<string, boolean> = {};
  /** Список с диска + runtime-состояние, для UI. */
  installed: PluginInfo[] = [];

  attach(backend: Backend) {
    this.backend = backend;
  }

  /** Загрузить список и запустить все включённые. Вызывать после логина. */
  async loadAll() {
    if (!this.backend) return;
    await this.stopAll();
    try {
      this.installed = await this.backend.listPlugins();
    } catch (e) {
      console.error('[plugins] listPlugins failed', e);
      return;
    }
    for (const info of this.installed) {
      if (info.enabled) await this.start(info.file);
    }
  }

  /** Список с актуальными runtime-флагами (запущен или нет). */
  isRunning(file: string): boolean {
    return this.running.has(file);
  }

  /** Свежий список с диска (без перезапуска плагинов) — для UI-вкладки. */
  async loadedList(): Promise<PluginInfo[]> {
    if (!this.backend) return this.installed;
    try {
      this.installed = await this.backend.listPlugins();
    } catch (e) {
      console.error('[plugins] listPlugins failed', e);
    }
    return this.installed;
  }

  /** Включить/выключить плагин (persist + start/stop). */
  async setEnabled(file: string, enabled: boolean) {
    if (!this.backend) return;
    await this.backend.setPluginEnabled(file, enabled);
    if (enabled) {
      await this.start(file);
    } else {
      await this.stop(file);
    }
    this.installed = this.installed.map((p) => (p.file === file ? { ...p, enabled } : p));
  }

  /** Установить новый плагин из кода. */
  async install(name: string, code: string): Promise<string> {
    if (!this.backend) throw new Error('Нет подключения');
    const file = name.endsWith('.js') ? name : `${name}.js`;
    await this.backend.savePlugin(file, code);
    const info = await this.readInfo(file, code);
    this.installed = [...this.installed.filter((p) => p.file !== file), info];
    return file;
  }

  /** Удалить плагин (и остановить, если запущен). */
  async remove(file: string) {
    if (!this.backend) return;
    await this.stop(file);
    await this.backend.deletePlugin(file);
    this.installed = this.installed.filter((p) => p.file !== file);
  }

  /** Прочитать код плагина (для просмотра/редактирования в UI). */
  async readCode(file: string): Promise<string> {
    if (!this.backend) return '';
    return this.backend.readPlugin(file);
  }

  /* ---------------- runtime ---------------- */

  private async readInfo(file: string, code: string): Promise<PluginInfo> {
    // Мягкий разбор манифеста без исполнения: тянем поля name/version/…
    const meta = this.parseMetaLoose(code);
    return { file, enabled: false, meta, size: code.length };
  }

  /** Regex-извлечение строк манифеста (без исполнения кода). */
  private parseMetaLoose(code: string): PluginMeta {
    const pick = (key: string): string | undefined => {
      const m = code.match(new RegExp(`${key}\\s*:\\s*['"\`]([^'"\`]*)['"\`]`));
      return m?.[1];
    };
    return {
      name: pick('name'),
      version: pick('version'),
      author: pick('author'),
      description: pick('description'),
    };
  }

  /** Исполнить код плагина и вызвать start(). */
  async start(file: string) {
    if (this.running.has(file) || !this.backend) return;
    let code: string;
    try {
      code = await this.backend.readPlugin(file);
    } catch (e) {
      console.error(`[plugins] не прочитать ${file}`, e);
      return;
    }
    try {
      // Плагин — выражение-объект: `({ ... });`. new Function НЕ возвращает
      // последнее выражение сам — оборачиваем в явный return(...), сняв
      // завершающие `;`. sourceURL даёт вменяемые стектрейсы.
      const cleaned = code.trim().replace(/;+\s*$/, '');
      const factory = new Function(
        `"use strict"; return (${cleaned});\n//# sourceURL=rustycord-plugin-${file}`,
      );
      const instance = factory();
      if (!instance || typeof instance !== 'object' || !instance.start) {
        throw new Error('плагин должен быть объектом с методом start(api)');
      }
      const handlers = new Map<string, Set<EventHandler>>();
      const cssIds: string[] = [];
      const api = this.makeApi(file, handlers, cssIds);
      await instance.start(api);
      this.running.set(file, {
        file,
        meta: {
          name: instance.name ?? file,
          version: instance.version ?? '?',
          author: instance.author,
          description: instance.description,
        },
        instance,
        api,
        handlers,
        cssIds,
      });
      // Сохраняем манифест на диск, чтобы список показывал карточки
      // без исполнения JS при следующем старте.
      try {
        await this.backend.setPluginMeta(file, this.running.get(file)!.meta);
      } catch {
        /* не критично */
      }
      console.info(`[plugins] ${instance.name ?? file} запущен`);
    } catch (e) {
      console.error(`[plugins] ошибка запуска ${file}:`, e);
      this.toastError(`Плагин ${file} упал при запуске — смотри консоль (F12)`);
    }
  }

  /** Вызвать stop(), снять подписки/CSS/хуки. */
  async stop(file: string) {
    const p = this.running.get(file);
    if (!p) return;
    this.running.delete(file);
    this.sendHooks.delete(file);
    try {
      if (p.instance?.stop) await p.instance.stop(p.api);
    } catch (e) {
      console.error(`[plugins] ошибка остановки ${file}:`, e);
    }
    for (const id of p.cssIds) document.getElementById(id)?.remove();
    p.handlers.clear();
  }

  async stopAll() {
    for (const file of [...this.running.keys()]) {
      await this.stop(file);
    }
    this.suppressions = {};
  }

  private makeApi(file: string, handlers: Map<string, Set<EventHandler>>, cssIds: string[]): PluginApi {
    const self = this;
    let cssSeq = 0;
    const api: PluginApi = {
      version: '1',
      id: file,
      on(event, handler) {
        let set = handlers.get(event);
        if (!set) {
          set = new Set();
          handlers.set(event, set);
        }
        set.add(handler as EventHandler);
        // Хук message:send — специальный случай: ожидает возвращаемое значение.
        if (event === 'message:send') {
          self.sendHooks.set(file, handler as SendHook);
        }
      },
      rest(method, path, body) {
        if (!self.backend) return Promise.reject(new Error('Нет подключения'));
        return self.backend.apiRequest(method, path, body);
      },
      addCss(css) {
        const id = `rc-plugin-css-${file.replace(/[^a-z0-9_-]/gi, '')}-${cssSeq++}`;
        const el = document.createElement('style');
        el.id = id;
        el.textContent = css;
        document.head.appendChild(el);
        cssIds.push(id);
      },
      suppress(flag, value) {
        self.suppressions[flag] = value;
      },
      storage: {
        get<T>(key: string, fallback: T): T {
          try {
            const raw = localStorage.getItem(`rc-plugin:${file}:${key}`);
            return raw === null ? fallback : (JSON.parse(raw) as T);
          } catch {
            return fallback;
          }
        },
        set(key: string, value: unknown) {
          try {
            localStorage.setItem(`rc-plugin:${file}:${key}`, JSON.stringify(value));
          } catch {
            /* quota */
          }
        },
      },
      toast(text: string) {
        window.dispatchEvent(new CustomEvent('rc-plugin-toast', { detail: text }));
      },
      me() {
        const raw = localStorage.getItem('rustycord.me');
        return raw ? (JSON.parse(raw) as { id: string; username: string }) : null;
      },
    };
    return api;
  }

  /* ---------------- events from store ---------------- */

  /** Диспатч gateway-события всем подписанным плагинам. */
  emit(event: PluginEventName, payload: unknown) {
    for (const p of this.running.values()) {
      const set = p.handlers.get(event);
      if (!set) continue;
      for (const h of set) {
        try {
          h(payload);
        } catch (e) {
          console.error(`[plugins] ${p.meta.name}: ошибка в обработчике ${event}`, e);
        }
      }
    }
  }

  /**
   * Хук перед отправкой сообщения: плагин может вернуть
   *  string — заменить текст, false — отменить отправку.
   */
  async beforeSend(content: string): Promise<string | false> {
    let text = content;
    for (const [file, hook] of this.sendHooks) {
      if (!this.running.has(file)) continue;
      try {
        const r: string | false | void = await hook(text);
        if (r === false) return false;
        if (typeof r === 'string' && r.length > 0) text = r;
      } catch (e) {
        console.error(`[plugins] send-hook ${file} упал`, e);
      }
    }
    return text;
  }

  private toastError(text: string) {
    window.dispatchEvent(new CustomEvent('rc-plugin-toast', { detail: text }));
  }
}

export const plugins = new PluginManager();

/* Утилита для UI: подписка на тосты плагинов. */
export function onPluginToast(cb: (text: string) => void): () => void {
  const fn = (e: Event) => cb((e as CustomEvent<string>).detail);
  window.addEventListener('rc-plugin-toast', fn);
  return () => window.removeEventListener('rc-plugin-toast', fn);
}

/** Тип для message:create подписки (удобно авторам плагинов). */
export type { Message };
