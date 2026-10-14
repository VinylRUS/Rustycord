<script lang="ts">
  /**
   * Модалка hCaptcha — Discord требует капчу на высокорисковых действиях
   * (вход email, обмен QR-тикета). Решение уходит в ждущий Rust-запрос
   * заголовком X-Captcha-Key (docs.discord.food/topics/captcha-handling).
   *
   * rqdata (hCaptcha Enterprise) ОБЯЗАН уйти в виджет — иначе Discord
   * отклонит решение. В демо-режиме (браузер) рисуем заглушку.
   */
  import { app } from '../stores/app.svelte';
  import Icon from './Icon.svelte';

  let container = $state<HTMLDivElement | null>(null);
  let widgetError = $state('');
  let loading = $state(true);

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  const CONTEXT_TITLES: Record<string, string> = {
    qr: 'Подтверди вход по QR-коду',
    login: 'Подтверди вход',
    mfa: 'Подтверди двухфакторный вход',
  };

  let hcaptchaId: number | null = null;
  let currentChallenge: string | null = null;

  /** Загрузить api.js один раз (глобальный hcaptcha). */
  function loadScript(): Promise<void> {
    const w = window as unknown as { hcaptcha?: unknown; __rcCaptchaOnload?: () => void };
    if (w.hcaptcha) return Promise.resolve();
    if (document.querySelector('script[data-hcaptcha]')) {
      // Скрипт грузится, но onload ещё не сработал — дождаться через поллинг.
      return new Promise((resolve) => {
        const t = setInterval(() => {
          if (w.hcaptcha) {
            clearInterval(t);
            resolve();
          }
        }, 100);
      });
    }
    return new Promise((resolve, reject) => {
      w.__rcCaptchaOnload = () => resolve();
      const s = document.createElement('script');
      s.src = 'https://js.hcaptcha.com/1/api.js?render=explicit&onload=__rcCaptchaOnload';
      s.async = true;
      s.defer = true;
      s.dataset.hcaptcha = '1';
      s.onerror = () => reject(new Error('Не удалось загрузить hCaptcha (сеть?)'));
      document.head.appendChild(s);
    });
  }

  /** Смонтировать виджет под текущий вызов (sitekey + rqdata). */
  $effect(() => {
    const challenge = app.captcha;
    if (!challenge || !isTauri) return;
    // Перерисовка при смене вызова (новый rqdata после отклонённого решения).
    const fingerprint = `${challenge.sitekey}:${challenge.rqdata ?? ''}:${challenge.sessionId ?? ''}`;
    if (fingerprint === currentChallenge) return;
    currentChallenge = fingerprint;
    widgetError = '';
    loading = true;

    loadScript()
      .then(() => {
        const hc = (window as unknown as {
          hcaptcha: {
            render: (
              el: HTMLElement,
              opts: Record<string, unknown>,
            ) => number;
            reset: (id?: number) => void;
          };
        }).hcaptcha;
        if (!container) return;
        container.innerHTML = '';
        const opts: Record<string, unknown> = {
          sitekey: challenge.sitekey,
          theme: 'dark',
          size: challenge.invisible ? 'invisible' : 'normal',
          'error-callback': (err: string) => {
            widgetError = `Ошибка капчи: ${err}. Нажми «Показать снова».`;
          },
          callback: (token: string) => {
            // Пользователь прошёл: токен → ждущий Rust-запрос.
            void app.submitCaptcha(token);
          },
        };
        // Enterprise: без rqdata решение будет отклонено.
        if (challenge.rqdata) opts['rqdata'] = challenge.rqdata;
        hcaptchaId = hc.render(container, opts);
        loading = false;
      })
      .catch((e) => {
        widgetError = e instanceof Error ? e.message : String(e);
        loading = false;
      });
    return () => {
      // Размонтирование/смена вызова: сбрасываем виджет.
      currentChallenge = null;
    };
  });

  function retry() {
    const hc = (window as unknown as { hcaptcha?: { reset: (id?: number) => void } }).hcaptcha;
    widgetError = '';
    if (hc && hcaptchaId !== null) hc.reset(hcaptchaId);
    else currentChallenge = null; // форсируем ремоунт через $effect
  }
</script>

{#if app.captcha}
  <div class="overlay">
    <div class="modal" role="dialog" aria-modal="true">
      <div class="head">
        <Icon name="logo" size={28} color="#5865f2" />
        <h2>{CONTEXT_TITLES[app.captcha.context] ?? 'Подтверждение безопасности'}</h2>
      </div>
      <p class="sub">
        Discord просит решить капчу, чтобы продолжить{app.captcha.context === 'qr' ? ' вход по QR-коду' : ''}.
        Это то же окно, что показывает официальный клиент.
      </p>

      {#if !isTauri}
        <!-- Демо-режим (браузер): настоящий виджет вёб-среды не привязан к сессии -->
        <div class="demo-box">
          <p>Демо-капча (в приложении здесь настоящий hCaptcha)</p>
          <button class="solve" onclick={() => void app.submitCaptcha('demo-captcha-token')}>
            Я не робот 🤖
          </button>
        </div>
      {:else if loading}
        <div class="loading">Загружаем капчу…</div>
      {:else}
        <div class="widget" bind:this={container}></div>
      {/if}

      {#if widgetError}
        <div class="error">{widgetError}</div>
        <button class="retry" onclick={retry}>Показать снова</button>
      {/if}

      <button class="cancel" onclick={() => void app.cancelCaptcha()}>
        Отменить{app.captcha.context === 'qr' ? ' (QR придётся сканировать заново)' : ''}
      </button>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
  }
  .modal {
    width: min(400px, calc(100vw - 32px));
    background: var(--bg-primary);
    border-radius: var(--radius-md);
    padding: 24px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  }
  .head {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  h2 {
    color: var(--header-primary);
    font-size: 20px;
    text-align: center;
  }
  .sub {
    color: var(--text-muted);
    font-size: 13.5px;
    text-align: center;
    line-height: 1.5;
  }
  .widget {
    min-height: 78px;
    display: flex;
    justify-content: center;
  }
  .loading {
    color: var(--text-muted);
    font-size: 14px;
    padding: 28px 0;
  }
  .demo-box {
    width: 100%;
    background: #1e1f22;
    border-radius: var(--radius-sm);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: center;
  }
  .demo-box p {
    color: var(--text-muted);
    font-size: 13px;
  }
  .solve {
    background: #5865f2;
    color: #fff;
    border: none;
    border-radius: var(--radius-sm);
    padding: 10px 18px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .error {
    color: #ff8a8d;
    font-size: 13px;
    text-align: center;
  }
  .retry {
    background: transparent;
    color: var(--text-link);
    border: none;
    font-size: 13px;
    cursor: pointer;
  }
  .cancel {
    background: transparent;
    color: var(--text-muted);
    border: none;
    font-size: 13px;
    cursor: pointer;
    padding: 4px;
  }
  .cancel:hover {
    text-decoration: underline;
  }
</style>
