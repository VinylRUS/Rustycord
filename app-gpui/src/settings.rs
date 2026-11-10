//! Экран настроек (полностраничный, как в официальном клиенте): разделы
//! «Голос и аудио», «Поведение», «Аккаунт», «О программе».
//!
//! Голосовые настройки (DSP, лупбек) пишутся прямо в состояние ядра —
//! применяются к живой сессии без перезахода в канал. «Поведение» —
//! нативные порты плагинов Silent Typing и Message Logger (без JS-движка:
//! флаги читают actions::trigger_typing и обработчик MESSAGE_DELETE).

use gpui::{prelude::*, px, rgb, IntoElement, MouseButton};
use rustycord_core::rest::models::User;
use rustycord_core::voice::dsp::{DspConfig, DspMetrics, NsStrength};

use crate::chat::{avatar_or_initial, avatar_url};
use crate::state::{AppModel, Screen, SettingsSection};
use crate::theme::{
    BG_BASE, BG_INPUT, BG_PANEL, BLURPLE, GREEN, RED, TEXT_FADED, TEXT_MUTED, TEXT_PRIMARY,
};

/* ------------------------------ Мелочи UI ------------------------------ */

/// Отображаемое имя для карточки «Аккаунт».
pub fn display_name_me(u: &User) -> String {
    u.global_name.clone().unwrap_or_else(|| u.username.clone())
}

/// Кликабельная кнопка.
fn action_btn(
    id: &str,
    label: &str,
    accent: u32,
    cx: &mut gpui::Context<AppModel>,
    on_click: impl Fn(&mut AppModel, &mut gpui::Context<AppModel>) + 'static,
) -> impl IntoElement {
    let label = label.to_string();
    gpui::div()
        .id(gpui::SharedString::from(id.to_string()))
        .px_3()
        .py_1()
        .rounded_md()
        .bg(rgb(accent))
        .text_color(gpui::white())
        .text_size(px(13.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x4752c4)))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |m, _, _, cx| on_click(m, cx)),
        )
}

/// Строка-переключатель (toggle).
fn toggle_row(
    id: &str,
    title: &str,
    hint: &str,
    on: bool,
    cx: &mut gpui::Context<AppModel>,
    on_flip: impl Fn(&mut AppModel, &mut gpui::Context<AppModel>) + 'static,
) -> impl IntoElement {
    gpui::div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .gap_3()
        .py_2()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(BG_INPUT)))
        .rounded_md()
        .child(
            gpui::div()
                .flex()
                .flex_col()
                .flex_1()
                .child(
                    gpui::div()
                        .text_size(px(14.))
                        .text_color(rgb(TEXT_PRIMARY))
                        .child(title.to_string()),
                )
                .child(
                    gpui::div()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT_MUTED))
                        .child(hint.to_string()),
                ),
        )
        .child(
            gpui::div()
                .flex()
                .items_center()
                .justify_end()
                .w(px(44.))
                .h(px(24.))
                .rounded_full()
                .map(|el| {
                    if on {
                        el.bg(rgb(GREEN))
                    } else {
                        el.bg(rgb(BG_INPUT))
                    }
                })
                .child(if on {
                    gpui::div()
                        .ml_2()
                        .size(px(18.))
                        .rounded_full()
                        .bg(gpui::white())
                } else {
                    gpui::div()
                        .ml_1()
                        .size(px(18.))
                        .rounded_full()
                        .bg(gpui::white())
                }),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |m, _, _, cx| on_flip(m, cx)),
        )
}

fn section_title(text: &str) -> impl IntoElement {
    gpui::div()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(TEXT_MUTED))
        .pb_2()
        .child(text.to_string())
}

fn label(text: &str) -> impl IntoElement {
    gpui::div()
        .text_size(px(13.))
        .text_color(rgb(TEXT_PRIMARY))
        .child(text.to_string())
}

fn muted(text: &str) -> impl IntoElement {
    gpui::div()
        .text_size(px(12.))
        .text_color(rgb(TEXT_FADED))
        .child(text.to_string())
}

/// Полоска-индикатор уровня (−100…0 dBFS → ширина).
fn level_bar(db: f32, width: f32) -> impl IntoElement {
    let norm = ((db + 100.0) / 100.0).clamp(0.0, 1.0);
    let w = (width * norm).max(2.0);
    let color = if db > -3.0 {
        RED
    } else if db > -20.0 {
        0xf0b232
    } else {
        GREEN
    };
    gpui::div()
        .w(px(width))
        .h(px(8.))
        .rounded_full()
        .bg(rgb(BG_INPUT))
        .child(gpui::div().w(px(w)).h(px(8.)).rounded_full().bg(rgb(color)))
}

/* ------------------------------ Открыть URL ------------------------------ */

/// Открыть ссылку в системном браузере (платформенная команда, без
/// плагинов — используется только из «О программе»).
pub fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let spawned = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let spawned = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    if let Err(e) = spawned {
        tracing::warn!("settings: не открыть {url}: {e}");
    }
}

/* ------------------------------ Навигация ------------------------------ */

fn nav_item(
    cx: &mut gpui::Context<AppModel>,
    cur: SettingsSection,
    want: SettingsSection,
    title: &str,
) -> impl IntoElement {
    let active = cur == want;
    gpui::div()
        .id(gpui::SharedString::from(format!("nav-{title}")))
        .px_3()
        .py_1()
        .rounded_md()
        .text_size(px(14.))
        .cursor_pointer()
        .map(|el| {
            if active {
                el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_PRIMARY))
            } else {
                el.text_color(rgb(TEXT_MUTED))
            }
        })
        .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
        .child(title.to_string())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |m, _, _, cx| {
                if let Screen::Main(st) = &mut m.screen {
                    st.settings_section = want;
                }
                cx.notify();
            }),
        )
}

/* -------------------------------- Рендер -------------------------------- */

pub fn render(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> gpui::AnyElement {
    let (section, me) = {
        let Screen::Main(m) = &model.screen else {
            return gpui::div().into_any_element();
        };
        let me = {
            let cache = model.core.cache.read().unwrap();
            cache.user.clone()
        };
        (m.settings_section, me)
    };

    // Левая навигация + заголовок с ✕.
    let mut sidebar = gpui::div()
        .flex()
        .flex_col()
        .w(px(220.))
        .p_3()
        .gap_1()
        .bg(rgb(BG_BASE))
        .child(
            gpui::div()
                .flex()
                .items_center()
                .pb_3()
                .child(
                    gpui::div()
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT_PRIMARY))
                        .child("Настройки"),
                )
                .child(
                    gpui::div()
                        .id("settings-close")
                        .ml_auto()
                        .size(px(26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(BG_INPUT))
                        .text_color(rgb(TEXT_MUTED))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(RED)))
                        .child("✕")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|m: &mut AppModel, _, _, cx| {
                                if let Screen::Main(st) = &mut m.screen {
                                    st.settings_open = false;
                                }
                                cx.notify();
                            }),
                        ),
                ),
        )
        .child(nav_item(
            cx,
            section,
            SettingsSection::Voice,
            "Голос и аудио",
        ))
        .child(nav_item(
            cx,
            section,
            SettingsSection::Behavior,
            "Поведение",
        ))
        .child(nav_item(cx, section, SettingsSection::Account, "Аккаунт"))
        .child(nav_item(cx, section, SettingsSection::About, "О программе"));

    if let Some(u) = &me {
        let avatar = u.avatar.as_deref().map(|h| avatar_url(&u.id, h));
        sidebar = sidebar.child(
            gpui::div()
                .mt_auto()
                .flex()
                .items_center()
                .gap_2()
                .pt_3()
                .child(avatar_or_initial(
                    &u.id,
                    avatar.as_deref(),
                    &display_name_me(u),
                    px(28.),
                ))
                .child(
                    gpui::div()
                        .flex()
                        .flex_col()
                        .child(
                            gpui::div()
                                .text_size(px(12.))
                                .text_color(rgb(TEXT_PRIMARY))
                                .child(display_name_me(u)),
                        )
                        .child(
                            gpui::div()
                                .text_size(px(10.))
                                .text_color(rgb(TEXT_FADED))
                                .child(format!("id {}", u.id)),
                        ),
                ),
        );
    }

    let content = match section {
        SettingsSection::Voice => voice_section(model, cx),
        SettingsSection::Behavior => behavior_section(model, cx),
        SettingsSection::Account => account_section(model, cx),
        SettingsSection::About => about_section(model, cx),
    };

    gpui::div()
        .flex()
        .flex_1()
        .min_h_0()
        .bg(rgb(BG_PANEL))
        .child(sidebar)
        .child(
            gpui::div()
                .id("settings-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .overflow_y_scroll()
                .px_8()
                .py_6()
                .child(content),
        )
        .into_any_element()
}

/* ------------------------------ Разделы ------------------------------ */

/// Снимок голосового состояния ядра для раздела «Голос».
fn voice_state(model: &AppModel) -> (DspConfig, DspMetrics, bool, bool) {
    let cfg = *model.core.voice_dsp_cfg.lock().unwrap();
    let metrics = *model.core.voice_dsp_metrics.lock().unwrap();
    let in_voice = model.core.voice_target.lock().unwrap().is_some();
    let loopback_on = matches!(&model.screen, Screen::Main(m) if m.loopback_on);
    (cfg, metrics, in_voice, loopback_on)
}

fn strength_str(s: NsStrength) -> &'static str {
    match s {
        NsStrength::Low => "low",
        NsStrength::Medium => "medium",
        NsStrength::High => "high",
    }
}

fn voice_section(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> gpui::AnyElement {
    let (cfg, metrics, in_voice, loopback_on) = voice_state(model);

    let mut col = gpui::div().flex().flex_col().gap_4().max_w(px(640.));

    col = col.child(section_title("ОБРАБОТКА МИКРОФОНА (DSP)"));

    col = col.child(toggle_row(
        "dsp-ns",
        "Шумоподавление",
        "Гасит фоновый шум: клавиатура, вентиляторы, улица",
        cfg.ns_enabled,
        cx,
        |m, cx| {
            let (c, _, _, _) = voice_state(m);
            crate::actions::dsp_set(m, !c.ns_enabled, strength_str(c.ns_strength), c.agc_enabled);
            cx.notify();
        },
    ));

    // Сила шумодава: три сегмента.
    let mut seg = gpui::div().flex().gap_1().pb_1();
    for (key, title) in [
        ("low", "Низкое"),
        ("medium", "Среднее"),
        ("high", "Сильное"),
    ] {
        let active = strength_str(cfg.ns_strength) == key;
        seg = seg.child(
            gpui::div()
                .id(gpui::SharedString::from(format!("dsp-ns-{key}")))
                .px_3()
                .py_1()
                .rounded_md()
                .text_size(px(12.))
                .cursor_pointer()
                .map(|el| {
                    if active {
                        el.bg(rgb(BLURPLE)).text_color(gpui::white())
                    } else {
                        el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_MUTED))
                    }
                })
                .hover(|s| s.text_color(gpui::white()))
                .child(title.to_string())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |m: &mut AppModel, _, _, cx| {
                        let (c, _, _, _) = voice_state(m);
                        crate::actions::dsp_set(m, c.ns_enabled, key, c.agc_enabled);
                        cx.notify();
                    }),
                ),
        );
    }
    col = col.child(seg);

    col = col.child(toggle_row(
        "dsp-agc",
        "Автоусиление (AGC)",
        "Выравнивает громкость микрофона — собеседники слышат тебя одинаково",
        cfg.agc_enabled,
        cx,
        |m, cx| {
            let (c, _, _, _) = voice_state(m);
            crate::actions::dsp_set(m, c.ns_enabled, strength_str(c.ns_strength), !c.agc_enabled);
            cx.notify();
        },
    ));

    col = col.child(gpui::div().h(px(6.)));

    col = col.child(section_title("ПРОВЕРКА МИКРОФОНА"));

    // Живой уровень: −100…0 dBFS + речевой детектор.
    let speech = if metrics.speech {
        "🗣 речь"
    } else {
        "тишина"
    };
    col = col
        .child(label(&format!(
            "Уровень входа: {:.0} dBFS ({speech})",
            metrics.input_db
        )))
        .child(gpui::div().pb_1().child(level_bar(metrics.input_db, 300.0)))
        .child(label(&format!(
            "После DSP: {:.0} dBFS · шумодав −{:.0} dB · AGC {:+.1} dB",
            metrics.output_db, metrics.ns_db, metrics.agc_db
        )));

    if in_voice {
        let (btn_id, btn_label, accent) = if loopback_on {
            ("loopback-off", "Выключить лупбек", RED)
        } else {
            ("loopback-on", "Проверить микрофон (лупбек)", BLURPLE)
        };
        let turn_on = !loopback_on;
        col = col.child(gpui::div().flex().gap_2().pt_1().child(action_btn(
            btn_id,
            btn_label,
            accent,
            cx,
            move |m, cx| {
                if let Screen::Main(st) = &mut m.screen {
                    st.loopback_on = turn_on;
                }
                crate::actions::loopback_set(m, turn_on);
                cx.notify();
            },
        )));
    } else {
        col = col.child(muted(
            "Войди в голосовой канал — здесь появится проверка микрофона и живой уровень",
        ));
    }

    col.into_any_element()
}

fn behavior_section(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> gpui::AnyElement {
    let (suppress, log_deleted) = {
        let Screen::Main(m) = &model.screen else {
            return gpui::div().into_any_element();
        };
        (m.suppress_typing, m.log_deleted)
    };
    let dev_mode = model.config.dev_mode;

    gpui::div()
        .flex()
        .flex_col()
        .gap_4()
        .max_w(px(640.))
        .child(section_title("ПЛАГИНЫ (НАТИВНЫЕ ПОРТЫ)"))
        .child(toggle_row(
            "silent-typing",
            "Silent Typing",
            "Другие не видят индикатор «печатает…» (порт плагина silent-typing.js)",
            suppress,
            cx,
            |m, cx| {
                if let Screen::Main(st) = &mut m.screen {
                    st.suppress_typing = !st.suppress_typing;
                }
                cx.notify();
            },
        ))
        .child(toggle_row(
            "message-logger",
            "Message Logger",
            "Показывает удалённые сообщения в баннере над чатом (порт message-logger.js)",
            log_deleted,
            cx,
            |m, cx| {
                if let Screen::Main(st) = &mut m.screen {
                    st.log_deleted = !st.log_deleted;
                }
                cx.notify();
            },
        ))
        .child(gpui::div().h(px(6.)))
        .child(section_title("РЕЖИМ РАЗРАБОТЧИКА"))
        .child(toggle_row(
            "dev-mode",
            "Dev-режим",
            "Клиент перезапустится и откроет консоль с логами протокола (уровень debug)",
            dev_mode,
            cx,
            |m, cx| {
                m.config.dev_mode = !m.config.dev_mode;
                crate::config::save(&m.config);
                cx.notify();
                // Перезапуск: консоль подключается на старте нового процесса.
                cx.restart();
            },
        ))
        .into_any_element()
}

fn account_section(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> gpui::AnyElement {
    let me = {
        let cache = model.core.cache.read().unwrap();
        cache.user.clone()
    };
    let Some(u) = me else {
        return gpui::div().into_any_element();
    };
    let avatar = u.avatar.as_deref().map(|h| avatar_url(&u.id, h));

    gpui::div()
        .flex()
        .flex_col()
        .gap_4()
        .max_w(px(640.))
        .child(section_title("МОЙ АККАУНТ"))
        .child(
            gpui::div()
                .flex()
                .items_center()
                .gap_3()
                .p_4()
                .rounded_lg()
                .bg(rgb(BG_BASE))
                .child(avatar_or_initial(
                    &u.id,
                    avatar.as_deref(),
                    &display_name_me(&u),
                    px(64.),
                ))
                .child(
                    gpui::div()
                        .flex()
                        .flex_col()
                        .child(
                            gpui::div()
                                .text_size(px(16.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(TEXT_PRIMARY))
                                .child(display_name_me(&u)),
                        )
                        .child(
                            gpui::div()
                                .text_size(px(12.))
                                .text_color(rgb(TEXT_MUTED))
                                .child(format!("@{}", u.username)),
                        )
                        .child(
                            gpui::div()
                                .text_size(px(11.))
                                .text_color(rgb(TEXT_FADED))
                                .child(format!("id: {}", u.id)),
                        ),
                ),
        )
        .child(gpui::div().flex().gap_2().pt_2().child(action_btn(
            "settings-logout",
            "Выйти из аккаунта",
            RED,
            cx,
            |m, cx| {
                crate::actions::logout(m, cx);
            },
        )))
        .into_any_element()
}

fn about_section(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> gpui::AnyElement {
    let (checking, text, url, is_newer) = {
        let Screen::Main(m) = &model.screen else {
            return gpui::div().into_any_element();
        };
        let u = &m.update_status;
        (u.checking, u.text.clone(), u.url.clone(), u.is_newer)
    };

    let mut col = gpui::div()
        .flex()
        .flex_col()
        .gap_4()
        .max_w(px(640.))
        .child(section_title("О ПРОГРАММЕ"))
        .child(label(&format!(
            "RustyCord GPUI {} · протокол rustycord-core {}",
            env!("CARGO_PKG_VERSION"),
            rustycord_core::VERSION
        )))
        .child(label(
            "Нативный клиент Discord на GPUI (Zed). Неофициальный проект.",
        ));

    let mut actions = gpui::div().flex().items_center().gap_3().pt_2();
    if checking {
        actions = actions.child(muted("Проверяем…"));
    } else {
        actions = actions.child(action_btn(
            "check-updates",
            "Проверить обновления",
            BLURPLE,
            cx,
            |m, cx| {
                crate::actions::check_updates(m, cx);
            },
        ));
    }
    if is_newer {
        let release_url = url.clone();
        actions = actions.child(action_btn(
            "open-release",
            "Открыть страницу релиза",
            GREEN,
            cx,
            move |_m, _cx| {
                open_url(&release_url);
            },
        ));
    }
    col = col.child(actions);

    if !text.is_empty() {
        col = col.child(
            gpui::div()
                .text_size(px(13.))
                .text_color(if is_newer {
                    rgb(GREEN)
                } else {
                    rgb(TEXT_MUTED)
                })
                .child(text),
        );
    }

    col = col.child(gpui::div().h(px(6.))).child(muted(
        "Репозиторий и баг-трекер: github.com/VinylRUS/Rustycord",
    ));

    col.into_any_element()
}
