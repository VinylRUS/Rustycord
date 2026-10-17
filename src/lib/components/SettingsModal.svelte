<script lang="ts">
  import { app } from '../stores/app.svelte';
  import { plugins } from '../plugins/manager';
  import type { SnowflakeInfo, VoiceDspStatus } from '../types';
  import { formatDuration } from '../time';
  import { APP_VERSION } from '../version';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  let tab = $state<'account' | 'voice' | 'plugins' | 'connection' | 'tools' | 'about'>('account');
  let snowflakeInput = $state('');
  let snowflakeResult = $state<SnowflakeInfo | null>(null);
  let snowflakeError = $state('');
  let rawFrame = $state('{\n  "op": 8,\n  "d": { "guild_id": "...", "query": "", "limit": 0 }\n}');
  let rawStatus = $state<{ kind: 'ok' | 'err'; text: string } | null>(null);
  let logPath = $state('');

  /* ---------- профиль (v0.8.0) ---------- */
  let bioDraft = $state('');
  let nameDraft = $state('');
  let profileBusy = $state(false);

  // Черновики — при каждом открытии вкладки из живого профиля.
  $effect(() => {
    if (tab === 'account' && app.user) {
      bioDraft = app.user.bio ?? '';
      nameDraft = app.user.global_name ?? '';
    }
  });

  /** Аватар: файл → data URI (PNG/JPEG ≤ 1 МБ — как лимит загрузки Discord). */
  async function pickAvatar(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    if (!/^image\/(png|jpeg)$/.test(file.type)) {
      app.toast('Аватар: нужен PNG или JPEG', 'error');
      return;
    }
    if (file.size > 1024 * 1024) {
      app.toast('Аватар: файл больше 1 МБ — сожми картинку', 'error');
      return;
    }
    const dataUri = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(new Error('read error'));
      reader.readAsDataURL(file);
    }).catch(() => '');
    if (!dataUri) return;
    profileBusy = true;
    await app.updateProfile({ avatar: dataUri });
    profileBusy = false;
  }

  async function clearAvatar() {
    profileBusy = true;
    await app.updateProfile({ avatar: null });
    profileBusy = false;
  }

  async function saveProfile() {
    profileBusy = true;
    const ok = await app.updateProfile({
      bio: bioDraft,
      global_name: nameDraft,
    });
    profileBusy = false;
    if (ok) bioDraft = bioDraft;
  }

  // Путь к файлу логов — тянем при открытии окна (для баг-репортов).
  $effect(() => {
    if (app.showSettings && !logPath) {
      void app.backend?.getLogPath().then((p) => (logPath = p));
    }
  });

  /* ---------- plugins ---------- */
  let pluginList = $state<import('../types').PluginInfo[]>([]);
  let newPluginName = $state('');
  let newPluginCode = $state('');
  let pluginError = $state('');
  let viewCode = $state('');
  let viewCodeName = $state('');
  let pluginsBusy = $state(false);

  async function refreshPlugins() {
    try {
      pluginList = await plugins.loadedList();
    } catch (e) {
      pluginError = e instanceof Error ? e.message : String(e);
    }
  }

  // Подтягиваем список при открытии вкладки.
  $effect(() => {
    if (tab === 'plugins' && app.user) void refreshPlugins();
  });

  async function installPlugin() {
    pluginError = '';
    if (!newPluginName.trim() || !newPluginCode.trim()) {
      pluginError = 'Укажи имя файла и код плагина';
      return;
    }
    pluginsBusy = true;
    try {
      await plugins.install(newPluginName.trim(), newPluginCode);
      newPluginName = '';
      newPluginCode = '';
      await refreshPlugins();
      app.toast('Плагин установлен (выключен) — включи его в списке', 'info');
    } catch (e) {
      pluginError = e instanceof Error ? e.message : String(e);
    } finally {
      pluginsBusy = false;
    }
  }

  async function togglePlugin(file: string, enabled: boolean) {
    try {
      await plugins.setEnabled(file, enabled);
      await refreshPlugins();
    } catch (e) {
      app.toast(`Не удалось: ${e instanceof Error ? e.message : String(e)}`, 'error');
    }
  }

  async function removePlugin(file: string) {
    if (!confirm(`Удалить плагин ${file}?`)) return;
    await plugins.remove(file);
    await refreshPlugins();
  }

  async function showCode(file: string) {
    if (viewCodeName === file) {
      viewCodeName = '';
      viewCode = '';
      return;
    }
    viewCodeName = file;
    viewCode = await plugins.readCode(file);
  }

  const PLUGIN_TEMPLATE = `({
  name: 'Мой плагин',
  version: '1.0.0',
  author: 'ты',
  description: 'Что делает плагин',
  start(api) {
    // api.on('message:create', (m) => console.log(m.author.username, m.content));
    // api.addCss('.message { letter-spacing: 0.3px; }');
    // api.suppress('typing', true);
  },
  stop() {}
});`;

  async function inspect() {
    snowflakeError = '';
    snowflakeResult = null;
    const id = snowflakeInput.trim();
    if (!id) return;
    try {
      snowflakeResult = await app.parseSnowflake(id);
    } catch (e) {
      snowflakeError = e instanceof Error ? e.message : String(e);
    }
  }

  async function sendRaw() {
    rawStatus = null;
    try {
      await app.gatewaySendRaw(rawFrame);
      rawStatus = { kind: 'ok', text: 'Кадр отправлен в gateway — ответ придёт потоком dispatch-событий (логи — в файле из вкладки «Подключение»).' };
    } catch (e) {
      rawStatus = { kind: 'err', text: e instanceof Error ? e.message : String(e) };
    }
  }

  /* ---------- голос: шумоподавление + AGC ---------- */

  /** Живая телеметрия DSP (опрос 4 раза в секунду, пока открыта вкладка). */
  let dspStatus = $state<VoiceDspStatus | null>(null);

  const NS_LEVELS = [
    { id: 'low', label: 'Слабое' },
    { id: 'medium', label: 'Среднее' },
    { id: 'high', label: 'Сильное' },
  ] as const;

  $effect(() => {
    if (!app.showSettings || tab !== 'voice' || !app.backend) return;
    let alive = true;
    const poll = async () => {
      try {
        const st = await app.backend!.getVoiceDspStatus();
        if (alive) dspStatus = st;
      } catch {
        /* сессия закрылась — покажем «неактивно» на следующем опросе */
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 250);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  });

  /** dBFS → проценты шкалы (−60 dBFS…0 → 0…100 %). */
  function dbPct(db: number): number {
    return Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  }

  function fmtDb(db: number): string {
    return db <= -95 ? '−∞' : db.toFixed(1);
  }
</script>

{#if app.showSettings}
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => (app.showSettings = false)} oncontextmenu={(e) => e.preventDefault()}></div>
  <div class="modal">
    <header>
      <h1>Настройки</h1>
      <button class="close" onclick={() => (app.showSettings = false)}>
        <Icon name="close" size={20} color="#b5bac1" />
      </button>
    </header>

    <div class="body">
      <nav class="nav">
        <div class="nav-group">Настройки пользователя</div>
        <button class:active={tab === 'account'} onclick={() => (tab = 'account')}>Моя учётная запись</button>
        <button class:active={tab === 'voice'} onclick={() => (tab = 'voice')}>Голос и видео</button>
        <button class:active={tab === 'plugins'} onclick={() => (tab = 'plugins')}>Плагины</button>
        <div class="nav-group">Расширенные</div>
        <button class:active={tab === 'connection'} onclick={() => (tab = 'connection')}>Подключение</button>
        <button class:active={tab === 'tools'} onclick={() => (tab = 'tools')}>Инструменты</button>
        <div class="nav-group">Прочее</div>
        <button class:active={tab === 'about'} onclick={() => (tab = 'about')}>О RustyCord</button>
        <div class="nav-spacer"></div>
        <button class="nav-logout" onclick={() => void app.logout()}>
          <Icon name="logout" size={16} color="#f23f43" /> Выйти из аккаунта
        </button>
      </nav>

    <div class="content scroller">
      {#if tab === 'account'}
        <section class="pane">
          <h3>Мой аккаунт</h3>
          <div class="profile-card">
            <Avatar user={app.user} size={72} ringColor="var(--bg-secondary)" />
            <div class="profile-fields">
              <div class="profile-name">{app.user?.global_name || app.user?.username}</div>
              <div class="profile-handle">@{app.user?.username}</div>
            </div>
            <div class="avatar-actions">
              <label class="btn-ghost file-btn">
                {profileBusy ? '…' : 'Сменить аватар'}
                <input type="file" accept="image/png,image/jpeg" onchange={(e) => void pickAvatar(e)} hidden />
              </label>
              {#if app.user?.avatar}
                <button class="btn-ghost" disabled={profileBusy} onclick={() => void clearAvatar()}>Убрать аватар</button>
              {/if}
            </div>
          </div>

          <h3>О себе</h3>
          <p class="hint">Био видно в профиле всем. Display name — отображаемое имя (не @username).</p>
          <div class="profile-form">
            <label class="field-label" for="global-name">DISPLAY NAME</label>
            <input id="global-name" bind:value={nameDraft} maxlength={32} placeholder={app.user?.username ?? ''} />
            <label class="field-label" for="bio">БИО</label>
            <textarea id="bio" rows={4} maxlength={190} bind:value={bioDraft} placeholder="Пара слов о себе…"></textarea>
            <button class="btn-blurple" onclick={() => void saveProfile()} disabled={profileBusy}>
              {profileBusy ? 'Сохраняем…' : 'Сохранить профиль'}
            </button>
          </div>
        </section>
      {:else if tab === 'connection'}
        <section class="pane">
          <h3>Подключение</h3>
          <p class="hint">Состояние соединения с Discord и служебная информация для баг-репортов.</p>
          <div class="user-line">
            <Avatar user={app.user} size={48} ringColor="var(--bg-secondary)" />
            <div>
              <div class="big">{app.user?.global_name || app.user?.username}</div>
              <div class="muted">{app.user ? '@' + app.user.username : ''}</div>
            </div>
          </div>
          <table>
            <tbody>
          <tr><td>Статус</td><td class="mono">{app.connStage}{app.connDetail ? ` (${app.connDetail})` : ''}</td></tr>
          <tr><td>Серверов</td><td class="mono">{app.guilds.length}</td></tr>
          <tr><td>Логи</td><td class="mono small" title={logPath}>{logPath || '…'}</td></tr>
          </tbody>
        </table>
        {#if logPath && logPath !== 'демо-режим: логи пишутся только в приложении'}
          <div class="row">
            <button class="btn-ghost" onclick={() => { void navigator.clipboard?.writeText(logPath); }}>
              Скопировать путь к логам (для баг-репортов)
            </button>
          </div>
        {/if}
        </section>
      {:else if tab === 'voice'}
        <section class="pane">
          <h3>Обработка микрофона</h3>
          <p class="hint">
            Шумоподавление и автогромкость работают на лету — настройки
            применяются сразу, без перезахода в канал.
          </p>

          <div class="dsp-row">
            <div class="dsp-main">
              <div class="dsp-name">Шумоподавление</div>
              <div class="dsp-desc">
                Давит стационарный шум (вентиляторы, клавиатура, улица) и паузы.
                Во время речи оценка шума замораживается — голос не трогается.
              </div>
            </div>
            <label class="switch">
              <input
                type="checkbox"
                checked={app.voiceDsp.ns_enabled}
                onchange={(e) => void app.setVoiceDsp({ ...app.voiceDsp, ns_enabled: (e.target as HTMLInputElement).checked })}
              />
              <span></span>
            </label>
          </div>
          {#if app.voiceDsp.ns_enabled}
            <div class="seg">
              {#each NS_LEVELS as lv (lv.id)}
                <button
                  class:on={app.voiceDsp.ns_strength === lv.id}
                  onclick={() => void app.setVoiceDsp({ ...app.voiceDsp, ns_strength: lv.id })}
                >{lv.label}</button>
              {/each}
            </div>
            <p class="hint small">Сильное — для шумных мест; если голос «тонет» или звучит роботизированно — уменьши силу.</p>
          {/if}

          <div class="dsp-row">
            <div class="dsp-main">
              <div class="dsp-name">Автоусиление (AGC)</div>
              <div class="dsp-desc">
                Доводит громкость речи до комфортного уровня: тихий микрофон
                подтягивается, громкий — приглушается. В паузах ничего не разгоняется.
              </div>
            </div>
            <label class="switch">
              <input
                type="checkbox"
                checked={app.voiceDsp.agc_enabled}
                onchange={(e) => void app.setVoiceDsp({ ...app.voiceDsp, agc_enabled: (e.target as HTMLInputElement).checked })}
              />
              <span></span>
            </label>
          </div>

          {#if app.myVoice}
            <div class="dsp-row">
              <div class="dsp-main">
                <div class="dsp-name">Проверка микрофона</div>
                <div class="dsp-desc">
                  Услышь свой голос в наушниках — ровно таким, каким его услышат
                  собеседники. Никуда не отправляется, только тебе.
                </div>
              </div>
              <button class="mini" class:on={app.micLoopback} onclick={() => void app.setVoiceLoopback(!app.micLoopback)}>
                {app.micLoopback ? 'Идёт проверка… стоп' : 'Слушать'}
              </button>
            </div>
          {/if}
        </section>

        <section class="pane">
          <h3>Живые уровни</h3>
          {#if dspStatus?.active}
            <div class="meter">
              <div class="meter-head">
                <span>Микрофон · до обработки</span>
                <span class="mono">{fmtDb(dspStatus.metrics.input_db)} dBFS</span>
              </div>
              <div class="meter-track">
                <div class="meter-fill" style:width="{dbPct(dspStatus.metrics.input_db)}%"></div>
              </div>
            </div>
            <div class="meter">
              <div class="meter-head">
                <span>Что услышат собеседники</span>
                <span class="mono">{fmtDb(dspStatus.metrics.output_db)} dBFS</span>
              </div>
              <div class="meter-track">
                <div class="meter-fill out" class:speaking={dspStatus.metrics.speech} style:width="{dbPct(dspStatus.metrics.output_db)}%"></div>
              </div>
            </div>
            <table>
              <tbody>
              <tr><td>Детектор</td><td>{#if dspStatus.metrics.speech}<span class="vad on">речь</span>{:else}<span class="vad">тишина</span>{/if}</td></tr>
              <tr><td>Усиление</td><td class="mono">{dspStatus.metrics.agc_db >= 0 ? '+' : ''}{fmtDb(dspStatus.metrics.agc_db)} dB</td></tr>
              <tr><td>Шумодав снял</td><td class="mono">{fmtDb(dspStatus.metrics.noise_db)} dB фона</td></tr>
              </tbody>
            </table>
            <p class="hint small">
              Если после включения шумодава слышно «дыхание» фона — прибавь силу
              или выключи автогромкость.
            </p>
          {:else}
            <p class="hint">Подключись к голосовому каналу — здесь появятся живые уровни микрофона.</p>
          {/if}
        </section>
      {:else if tab === 'tools'}
        <section class="pane">
          <h3>Инспектор ID</h3>
          <p class="hint">
            Вставь любой ID Discord (сообщение, канал, пользователь) — покажем,
            когда он создан и сколько ему дней.
          </p>
          <div class="sf-form">
            <input placeholder="Например: 1301000000000000305" bind:value={snowflakeInput} />
            <button class="btn-blurple" onclick={() => void inspect()}>Разобрать</button>
          </div>
          {#if snowflakeError}
            <div class="err">{snowflakeError}</div>
          {/if}
          {#if snowflakeResult}
            <table>
              <tbody>
              <tr><td>ID</td><td class="mono small">{snowflakeResult.id}</td></tr>
              <tr><td>Создан</td><td class="mono">{new Date(snowflakeResult.timestamp_ms).toLocaleString('ru-RU')}</td></tr>
              <tr><td>Возраст</td><td class="mono">{formatDuration(snowflakeResult.age_seconds)} назад</td></tr>
              </tbody>
            </table>
          {/if}
        </section>

        <section class="pane">
          <h3>Консоль протокола</h3>
          <p class="hint">
            Отправка JSON-кадра напрямую в шлюз Discord — для отладки
            и экспериментов. Ответ приходит потоком событий и пишется в логи
            приложения (путь — во вкладке «Подключение»).
          </p>
          <div class="sf-form">
            <textarea
              rows="7"
              class="mono"
              spellcheck="false"
              bind:value={rawFrame}
              placeholder={'{ "op": 1, "d": null }'}
            ></textarea>
          </div>
          <div class="row">
            <button class="btn-blurple" onclick={() => void sendRaw()}>Отправить кадр</button>
          </div>
          {#if rawStatus}
            <div class={rawStatus.kind === 'ok' ? 'ok-msg' : 'err'}>{rawStatus.text}</div>
          {/if}
        </section>
      {:else if tab === 'plugins'}
        <p class="hint">
          <b>Плагины</b> — как у Vencord: JS-файлы в папке приложения.
        </p>

        {#if pluginError}
          <div class="err">{pluginError}</div>
        {/if}

        {#if pluginList.length === 0}
          <p class="hint">Установленных плагинов нет. Вставь код ниже или положи .js в папку плагинов.</p>
        {:else}
          {#each pluginList as p (p.file)}
            <div class="plugin-card">
              <div class="plugin-head">
                <div class="plugin-title">
                  <b>{p.meta?.name ?? p.file}</b>
                  {#if p.meta?.version}<span class="muted">v{p.meta.version}</span>{/if}
                  {#if p.meta?.author}<span class="muted">· {p.meta.author}</span>{/if}
                  {#if plugins.isRunning(p.file)}<span class="ok-tag">запущен</span>{/if}
                </div>
                <label class="switch">
                  <input type="checkbox" checked={p.enabled} onchange={(e) => void togglePlugin(p.file, (e.target as HTMLInputElement).checked)} />
                  <span></span>
                </label>
              </div>
              {#if p.meta?.description}
                <p class="plugin-desc">{p.meta.description}</p>
              {/if}
              <div class="plugin-actions">
                <button class="mini" onclick={() => void showCode(p.file)}>{viewCodeName === p.file ? 'Скрыть код' : 'Код'}</button>
                <button class="mini danger" onclick={() => void removePlugin(p.file)}>Удалить</button>
                <span class="muted mono-s">{p.file} · {Math.max(1, Math.round(p.size / 1024))} КБ</span>
              </div>
              {#if viewCodeName === p.file}
                <pre class="plugin-code">{viewCode}</pre>
              {/if}
            </div>
          {/each}
        {/if}

        <h3 class="sub">Установить новый плагин</h3>
        <div class="sf-form">
          <input placeholder="имя-файла (латиница, напр. my-plugin)" bind:value={newPluginName} />
          <textarea
            rows="10"
            class="mono"
            spellcheck="false"
            bind:value={newPluginCode}
            placeholder={PLUGIN_TEMPLATE}
          ></textarea>
          <div class="row">
            <button class="btn-blurple" onclick={() => void installPlugin()} disabled={pluginsBusy}>
              {pluginsBusy ? 'Устанавливаем…' : 'Установить'}
            </button>
            <button class="btn-ghost" onclick={() => (newPluginCode = PLUGIN_TEMPLATE)}>Вставить шаблон</button>
          </div>
        </div>
        <p class="hint small">
          API: api.on('message:create'|'message:send'|'dispatch'|'ready', fn) · api.rest(method, path, body) ·
          api.addCss(css) · api.suppress('typing', true) · api.storage · api.toast(text).
          Хук 'message:send' может вернуть изменённый текст или false для отмены.
        </p>
      {:else if tab === 'about'}
        <section class="pane">
          <h3>О RustyCord</h3>
          <p class="hint">
            Лёгкий неофициальный клиент Discord. Открытый код, лицензия MIT:
            <a href="https://github.com/VinylRUS/Rustycord" target="_blank" rel="noreferrer">github.com/VinylRUS/Rustycord</a>.
          </p>
          <table>
            <tbody>
            <tr><td>Версия</td><td class="mono">{APP_VERSION}</td></tr>
            <tr><td>Лицензия</td><td class="mono">MIT</td></tr>
            </tbody>
          </table>
          <p class="hint small">
            Не аффилирован с Discord Inc. Клиент работает на пользовательском токене
            (как Vencord/Vesktop) — это серая зона ToS: спокойное использование
            терпимо, спам и автоматизация = бан аккаунта. Используй разумно и на свой риск.
          </p>
        </section>
      {/if}
    </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    z-index: 100;
  }
  .modal {
    position: fixed;
    inset: 0;
    margin: auto;
    width: min(940px, calc(100vw - 48px));
    height: min(600px, calc(100vh - 48px));
    background: var(--bg-primary);
    border-radius: var(--radius-md);
    z-index: 101;
    display: flex;
    flex-direction: column;
    animation: popIn 0.15s ease;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px 8px;
    flex-shrink: 0;
  }
  h1 { font-size: 20px; color: var(--header-primary); }
  .close {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .close:hover { background: var(--bg-modifier-hover); }

  /* ---- Discord-style: левая навигация + контент ---- */
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .nav {
    width: 218px;
    flex-shrink: 0;
    background: var(--bg-secondary);
    padding: 8px 8px 12px;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    user-select: none;
  }
  .nav-group {
    padding: 16px 10px 4px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    color: var(--text-muted);
  }
  .nav button {
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: left;
    padding: 6px 10px;
    margin: 1px 0;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-size: 14px;
    font-weight: 500;
  }
  .nav button:hover { background: var(--bg-modifier-hover); color: var(--text-normal); }
  .nav button.active {
    background: var(--bg-modifier-active);
    color: #fff;
  }
  .nav-spacer { flex: 1; }
  .nav-logout { color: var(--red) !important; font-weight: 600; }
  .nav-logout:hover { background: #f23f4322 !important; }
  .content { padding: 4px 24px 24px; }
  .user-line { display: flex; align-items: center; gap: 14px; margin-bottom: 18px; }
  .big { font-size: 18px; font-weight: 700; color: var(--header-primary); }
  table { width: 100%; border-collapse: collapse; margin-bottom: 18px; }
  td { padding: 7px 10px; border-bottom: 1px solid #26282c66; font-size: 14px; }
  td:first-child { color: var(--text-muted); width: 160px; }
  .mono { font-family: var(--font-code); }
  .small { font-size: 12px; word-break: break-all; }
  .row { display: flex; gap: 8px; margin-bottom: 14px; }
  .hint { color: var(--text-muted); font-size: 13px; line-height: 1.6; margin-bottom: 14px; }
  .sf-form { display: flex; gap: 8px; margin-bottom: 14px; }
  .sf-form input {
    flex: 1;
    background: var(--bg-base);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    font-family: var(--font-code);
    font-size: 13px;
  }
  .sf-form textarea {
    flex: 1;
    background: var(--bg-base);
    border: none;
    outline: none;
    resize: vertical;
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    font-family: var(--font-code);
    font-size: 13px;
    color: var(--text-normal);
    line-height: 1.5;
  }
  .err { color: var(--red); font-size: 13px; margin-bottom: 12px; }
  .ok-msg { color: var(--green, #23a55a); font-size: 13px; line-height: 1.5; margin-bottom: 12px; }

  .sub {
    font-size: 13px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.02em;
    margin: 18px 0 8px;
  }

  .plugin-card {
    background: #2b2d31;
    border-radius: 8px;
    padding: 12px 14px;
    margin-bottom: 10px;
  }
  .plugin-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .plugin-title {
    display: flex;
    align-items: baseline;
    gap: 6px;
    color: var(--header-primary);
    flex-wrap: wrap;
  }
  .ok-tag {
    color: #23a55a;
    font-size: 11px;
    border: 1px solid #23a55a66;
    border-radius: 4px;
    padding: 0 5px;
  }
  .plugin-desc {
    color: var(--text-muted);
    font-size: 13px;
    margin: 6px 0;
    line-height: 1.5;
  }
  .plugin-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .plugin-actions .muted { margin-left: auto; }
  .mono-s { font-family: var(--font-code); font-size: 11px; }
  .mini {
    background: #4e5058;
    color: #dbdee1;
    border: none;
    border-radius: 4px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .mini:hover { background: #6d6f78; }
  .mini.danger { background: transparent; color: #f23f43; }
  .mini.danger:hover { background: #f23f4322; }
  .mini.on { background: var(--green); color: #fff; }
  .plugin-code {
    background: #1e1f22;
    border-radius: 6px;
    padding: 10px;
    margin-top: 10px;
    font-family: var(--font-code);
    font-size: 12px;
    line-height: 1.55;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 260px;
    overflow-y: auto;
    color: #b5bac1;
  }

  .switch {
    position: relative;
    display: inline-block;
    width: 40px;
    height: 22px;
    flex-shrink: 0;
    cursor: pointer;
  }
  .switch input { opacity: 0; width: 0; height: 0; }
  .switch span {
    position: absolute;
    inset: 0;
    background: #80848e;
    border-radius: 11px;
    transition: 0.2s;
  }
  .switch span::before {
    content: '';
    position: absolute;
    width: 16px;
    height: 16px;
    left: 3px;
    top: 3px;
    background: #fff;
    border-radius: 50%;
    transition: 0.2s;
  }
  .switch input:checked + span { background: #23a55a; }
  .switch input:checked + span::before { transform: translateX(18px); }

  /* ---- профиль (v0.8.0) ---- */
  .profile-card {
    display: flex;
    align-items: center;
    gap: 16px;
    background: var(--bg-secondary);
    border-radius: var(--radius-md);
    padding: 16px;
  }
  .profile-name { color: var(--header-primary); font-size: 17px; font-weight: 700; }
  .profile-handle { color: var(--text-muted); font-size: 13px; }
  .avatar-actions { margin-left: auto; display: flex; flex-direction: column; gap: 8px; align-items: flex-end; }
  .file-btn { cursor: pointer; }
  .profile-form { display: flex; flex-direction: column; gap: 4px; margin-top: 8px; }
  .profile-form textarea {
    background: #1e1f22;
    border-radius: var(--radius-sm);
    padding: 10px;
    font-size: 14px;
    color: var(--text-normal);
    resize: vertical;
    font-family: inherit;
  }
  .profile-form .btn-blurple { margin-top: 10px; align-self: flex-start; }
  .field-label {
    display: block;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.02em;
    margin: 10px 0 6px;
  }
  .profile-form input {
    background: #1e1f22;
    border-radius: var(--radius-sm);
    padding: 10px;
    font-size: 14px;
    color: var(--text-normal);
  }

  /* ---- голос: шумоподавление + AGC ---- */
  .pane { margin-bottom: 22px; }
  .pane h3 { color: var(--header-primary); font-size: 15px; margin-bottom: 10px; }
  .dsp-row {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 12px 0;
  }
  .dsp-main { flex: 1; min-width: 0; }
  .dsp-name { color: var(--header-primary); font-size: 15px; font-weight: 600; margin-bottom: 4px; }
  .dsp-desc { color: var(--text-muted); font-size: 13px; line-height: 1.5; }
  .seg {
    display: inline-flex;
    background: #1e1f22;
    border-radius: var(--radius-sm);
    padding: 3px;
    gap: 2px;
    margin-bottom: 8px;
  }
  .seg button {
    padding: 6px 14px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-muted);
    border-radius: calc(var(--radius-sm) - 2px);
  }
  .seg button:hover { color: var(--text-normal); }
  .seg button.on { background: #5865f2; color: #fff; }
  .hint.small { font-size: 12px; }

  .meter { margin-bottom: 12px; }
  .meter-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: 12px;
    color: var(--text-muted);
    margin-bottom: 5px;
  }
  .meter-head .mono { font-size: 12px; color: var(--text-normal); }
  .meter-track {
    height: 8px;
    background: #1e1f22;
    border-radius: 4px;
    overflow: hidden;
  }
  .meter-fill {
    height: 100%;
    border-radius: 4px;
    background: linear-gradient(90deg, #23a55a 0%, #23a55a 62%, #f0b232 85%, #f23f43 100%);
    transition: width 0.15s linear;
  }
  .meter-fill.out { background: #5865f2; }
  .meter-fill.out.speaking { background: #23a55a; }
  .vad {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
  }
  .vad.on { color: #23a55a; }

</style>
