(function () {
  'use strict';
  // Скрипт исполняется в КАЖДОМ документе окна (до навигации это
  // about:blank, дальше — iframe hCaptcha): рабочий контекст — только
  // главное окно на discord.com.
  if (window.top !== window) return;
  var host = location.hostname;
  if (host !== 'discord.com' && host.slice(-13) !== '.discord.com') return;
  if (window.__rcAuthInstalled) return;
  window.__rcAuthInstalled = true;
  function cb(kind, data) {
    location.href = '/__rustycord_cb?s=' + encodeURIComponent(kind) + '&d=' + encodeURIComponent(data);
  }
  function ping() { cb('ready', '1'); }
  function onReady() {
    veil('Подключаемся к Discord…');
    ping();
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', onReady);
  else onReady();

  // Вуаль поверх чужой страницы логина: окно открывается скрытым и его
  // содержимое до капчи/закрытия не должно выглядеть интерактивным.
  function veil(text) {
    var v = document.getElementById('__rc_veil');
    if (!v) {
      v = document.createElement('div');
      v.id = '__rc_veil';
      v.style.cssText = 'position:fixed;inset:0;z-index:2147483646;background:#1e1f22;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:16px;font-family:"gg sans","Segoe UI",system-ui,sans-serif';
      var spin = document.createElement('div');
      spin.style.cssText = 'width:30px;height:30px;border:3px solid #5865f2;border-top-color:transparent;border-radius:50%;animation:__rc_spin .8s linear infinite';
      var logo = document.createElement('div');
      logo.textContent = 'RUSTYCORD';
      logo.style.cssText = 'font-size:12px;font-weight:700;letter-spacing:.14em;color:#f2f3f5';
      var label = document.createElement('div');
      label.id = '__rc_veil_text';
      label.style.cssText = 'font-size:14px;color:#b5bac1';
      v.appendChild(spin); v.appendChild(logo); v.appendChild(label);
      (document.body || document.documentElement).appendChild(v);
      var st = document.createElement('style');
      st.textContent = '@keyframes __rc_spin{to{transform:rotate(360deg)}}';
      (document.head || document.documentElement).appendChild(st);
    }
    var t = document.getElementById('__rc_veil_text');
    if (t && text) t.textContent = text;
    return v;
  }

  // X-Super-Properties из НАСТОЯЩЕГО navigator: браузерный контекст окна
  // консистентен сам с собой (UA страницы = UA запроса = UA в XSP).
  function xsp(cfg) {
    var build = cfg.build;
    var ua = navigator.userAgent || '';
    var os = 'Unknown', osv = '';
    var mnt = ua.match(/Windows NT ([0-9.]+)/);
    if (mnt) { os = 'Windows'; osv = mnt[1] === '10.0' ? '10' : mnt[1]; }
    else if (/Mac OS X/.test(ua)) { os = 'Mac OS X'; }
    else if (/Android/.test(ua)) { os = 'Android'; }
    else if (/(iPhone|iPad)/.test(ua)) { os = 'iOS'; }
    else if (/Linux/.test(ua)) { os = 'Linux'; }
    var browser = 'Chrome';
    if (/Firefox\//.test(ua)) browser = 'Firefox';
    else if (/Edg\//.test(ua)) browser = 'Edge';
    else if (/Chrome\//.test(ua)) browser = 'Chrome';
    else browser = 'Safari';
    var bv = (ua.match(new RegExp(browser + '/([0-9.]+)')) || [])[1] || '';
    var p = {
      os: os, browser: browser, device: '',
      system_locale: (navigator.language || 'en-US'),
      has_client_mods: false,
      browser_user_agent: ua, browser_version: bv, os_version: osv,
      referrer: '', referring_domain: '', referrer_current: '', referring_domain_current: '',
      release_channel: 'stable',
      client_build_number: build || 0,
      native_build_number: null,
      client_event_source: null,
      client_launch_id: (cfg.launchId || ''),
      launch_signature: (cfg.launchSignature || ''),
      client_app_state: 'focused'
    };
    try { return btoa(unescape(encodeURIComponent(JSON.stringify(p)))); } catch (e) { return ''; }
  }

  function headers(cfg) {
    var h = {
      'Content-Type': 'application/json',
      'X-Discord-Locale': cfg.locale || 'ru-RU',
      'X-Super-Properties': xsp(cfg)
    };
    if (cfg.contextProperties) h['X-Context-Properties'] = cfg.contextProperties;
    if (cfg.captchaKey) h['X-Captcha-Key'] = cfg.captchaKey;
    if (cfg.sessionId) h['X-Captcha-Session-Id'] = cfg.sessionId;
    if (cfg.rqtoken) h['X-Captcha-Rqtoken'] = cfg.rqtoken;
    return h;
  }

  function renderCard(cfg) {
    var old = document.getElementById('__rc_overlay');
    if (old) old.remove();
    var v = document.getElementById('__rc_veil');
    if (v) v.remove();
    var overlay = document.createElement('div');
    overlay.id = '__rc_overlay';
    overlay.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:rgba(0,0,0,.93);display:flex;align-items:center;justify-content:center;font-family:"gg sans","Segoe UI",system-ui,sans-serif';
    var card = document.createElement('div');
    card.style.cssText = 'background:#313338;border-radius:10px;padding:24px 28px;max-width:400px;text-align:center;box-shadow:0 10px 30px rgba(0,0,0,.6)';
    var title = document.createElement('div');
    title.style.cssText = 'font-size:16px;font-weight:700;color:#f2f3f5;margin-bottom:6px';
    title.textContent = cfg.title;
    var sub = document.createElement('div');
    sub.id = '__rc_sub';
    sub.style.cssText = 'font-size:13px;color:#b5bac1;margin-bottom:16px;line-height:1.5';
    sub.textContent = cfg.subtitle;
    var widget = document.createElement('div');
    widget.id = '__rc_widget';
    widget.style.cssText = 'min-height:78px;display:flex;justify-content:center';
    var err = document.createElement('div');
    err.id = '__rc_err';
    err.style.cssText = 'color:#f87d7d;font-size:12px;margin-top:10px;min-height:14px;white-space:pre-wrap';
    var cancel = document.createElement('div');
    cancel.style.cssText = 'margin-top:14px;font-size:12px;color:#949ba4;cursor:pointer;text-decoration:underline';
    cancel.textContent = 'Отменить вход';
    cancel.onclick = function () { cb('cancel', '1'); };
    card.appendChild(title); card.appendChild(sub); card.appendChild(widget);
    card.appendChild(err); card.appendChild(cancel);
    overlay.appendChild(card);
    document.body.appendChild(overlay);
    var opts = {
      sitekey: cfg.sitekey,
      theme: 'dark',
      callback: function (token) {
        var s = document.getElementById('__rc_sub');
        if (s) s.textContent = 'Проверяем решение…';
        var w = document.getElementById('__rc_widget');
        if (w) w.style.display = 'none';
        cb('captcha', token);
      },
      'error-callback': function (code) {
        var el = document.getElementById('__rc_err');
        if (el) el.textContent = 'Ошибка капчи (' + code + ') — нажми чекбокс ещё раз';
        cb('hcaptcha-err', String(code));
      },
      'expired-callback': function () {
        var el = document.getElementById('__rc_err');
        if (el) el.textContent = 'Проверка истекла — решите ещё раз';
        try { window.hcaptcha.reset(); } catch (e) {}
      }
    };
    if (cfg.invisible) opts.size = 'invisible';
    var id;
    try { id = window.hcaptcha.render(widget, opts); }
    catch (e) {
      err.textContent = 'Не удалось показать капчу: ' + e;
      return;
    }
    // Enterprise-rqdata НЕ является опцией render — api.js её молча
    // игнорирует («rqdata» в api.js читается только при старте челленджа,
    // куда данные приходят через setData). Официальный клиент (бандл
    // discord.com, чанк капча-модалки) рендерит виджет БЕЗ rqdata и
    // передаёт её отдельным вызовом setData: она уходит в iframe
    // («challenge-data») и включается в getcaptcha, привязывая решение
    // к вызову Discord. Без привязки токен «чужой» для сессии капчи —
    // siteverify отвечает invalid-response (живой лог v0.8.3).
    if (cfg.rqdata) {
      try { window.hcaptcha.setData(id, { rqdata: cfg.rqdata }); }
      catch (e) { cb('err', 'hcaptcha-setdata: ' + e); }
    }
    if (cfg.invisible) {
      try { window.hcaptcha.execute(id); } catch (e) {}
    }
  }

  window.__rcOnload = function () {
    window.__rcScriptLoading = false;
    var cfg = window.__rcCfg;
    window.__rcCfg = null;
    if (cfg) renderCard(cfg);
  };

  window.__rcAuth = {
    challenge: function (cfg) {
      if (window.hcaptcha && window.hcaptcha.render) { renderCard(cfg); return; }
      window.__rcCfg = cfg;
      if (window.__rcScriptLoading) return;
      window.__rcScriptLoading = true;
      var s = document.createElement('script');
      s.src = 'https://js.hcaptcha.com/1/api.js?render=explicit&onload=__rcOnload';
      s.async = true;
      s.onerror = function () { cb('err', 'hcaptcha-api-load'); };
      (document.head || document.documentElement).appendChild(s);
    },
    request: function (cfg) {
      // Карточка капчи уже висит (ретрай решения) — обновляем её подпись;
      // иначе показываем/обновляем вуаль.
      var card = document.getElementById('__rc_overlay');
      if (card) {
        var sub = document.getElementById('__rc_sub');
        if (sub && cfg.veil) sub.textContent = cfg.veil;
      } else {
        veil(cfg.veil || 'Проверяем…');
      }
      // Ретрай капчи: решение уходит ДВАЖДЫ — заголовками X-Captcha-*
      // (механизм веб-клиента) и полями JSON-тела captcha_key /
      // captcha_rqtoken / captcha_session_id (remote-auth/login — путь
      // десктопного QR-входа — читает тело). Сервер использует один
      // механизм, второй игнорирует.
      function payload() {
        if (!cfg.captchaKey) return JSON.stringify(cfg.body);
        var b = Object.assign({}, cfg.body);
        b.captcha_key = cfg.captchaKey;
        if (cfg.rqtoken) b.captcha_rqtoken = cfg.rqtoken;
        if (cfg.sessionId) b.captcha_session_id = cfg.sessionId;
        return JSON.stringify(b);
      }
      var i = 0;
      function go() {
        return fetch(cfg.urls[i], {
          method: 'POST',
          credentials: 'include',
          headers: headers(cfg),
          body: payload()
        }).then(function (r) {
          if (r.status === 404 && i + 1 < cfg.urls.length) { i++; return go(); }
          return r.text().then(function (t) { return { status: r.status, body: t }; });
        });
      }
      go().then(function (r) { cb('http', JSON.stringify(r)); })
        .catch(function (e) { cb('err', 'fetch: ' + e); });
    }
  };
})();
