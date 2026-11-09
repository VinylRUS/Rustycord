//! Экран логина: три способа — токен, email+пароль (+2FA), QR-код.
//! Капча (если риск-скоринг Discord её потребует) открывается отдельным
//! окном discord.com — реализация CaptchaSolver (wry) в captcha.rs.

use gpui::{
    prelude::*, px, rgb, AnyElement, IntoElement, MouseButton, ParentElement, Styled, Window,
};

use rustycord_core::VERSION;

use crate::state::{AppModel, LoginTab, QrStage, Screen};
use crate::text_input::{Submit, TextInput};
use crate::theme::*;

pub fn render(
    model: &mut AppModel,
    _window: &mut Window,
    cx: &mut gpui::Context<AppModel>,
) -> impl IntoElement {
    let tab = match &model.screen {
        Screen::Login(l) => l.tab,
        _ => LoginTab::Token,
    };
    let _busy = match &model.screen {
        Screen::Login(l) => l.busy,
        _ => false,
    };
    let error = match &model.screen {
        Screen::Login(l) => l.error.clone(),
        _ => None,
    };
    let mfa = match &model.screen {
        Screen::Login(l) => l.mfa_ticket.is_some(),
        _ => false,
    };

    let tabs = {
        let mut row = gpui::div().flex().gap_2();
        for (label, this) in [
            ("Токен", LoginTab::Token),
            ("Email", LoginTab::Email),
            ("QR-код", LoginTab::Qr),
        ] {
            let active = tab == this;
            row = row.child(
                gpui::div()
                    .id(gpui::SharedString::from(label.to_string()))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .text_size(px(13.))
                    .cursor_pointer()
                    .map(|el| {
                        if active {
                            el.bg(rgb(BLURPLE)).text_color(gpui::white())
                        } else {
                            el.bg(rgb(BG_INPUT))
                                .text_color(rgb(TEXT_MUTED))
                                .hover(|s| s.bg(rgb(BG_HOVER)).text_color(rgb(TEXT_PRIMARY)))
                        }
                    })
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |m: &mut AppModel, _, _, cx| {
                            if let Screen::Login(l) = &mut m.screen {
                                l.tab = this;
                                l.mfa_ticket = None;
                                l.error = None;
                            }
                            cx.notify();
                        }),
                    ),
            );
        }
        row
    };

    let card: AnyElement = if mfa {
        mfa_card(model, cx).into_any_element()
    } else {
        match tab {
            LoginTab::Token => token_card(model, cx).into_any_element(),
            LoginTab::Email => email_card(model, cx).into_any_element(),
            LoginTab::Qr => qr_card(model, cx).into_any_element(),
        }
    };

    gpui::div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .bg(rgb(BG_CHAT))
        .font_family(ui_font())
        .text_size(px(14.))
        .text_color(rgb(TEXT_PRIMARY))
        // Enter в любом поле = действие формы
        .on_action(cx.listener(|m: &mut AppModel, _: &Submit, _w, cx| {
            let (tab, mfa) = match &m.screen {
                Screen::Login(l) => (l.tab, l.mfa_ticket.is_some()),
                _ => return,
            };
            match (tab, mfa) {
                (LoginTab::Token, _) => {
                    let token = match &m.screen {
                        Screen::Login(l) => l.token_input.read(cx).text(),
                        _ => String::new(),
                    };
                    if !token.trim().is_empty() {
                        crate::actions::connect_token(m, token.trim().to_string(), cx);
                    }
                }
                (LoginTab::Email, true) => {
                    let code = match &m.screen {
                        Screen::Login(l) => l.mfa_input.read(cx).text(),
                        _ => String::new(),
                    };
                    crate::actions::login_mfa(m, code, cx);
                }
                (LoginTab::Email, false) => crate::actions::login_email(m, cx),
                (LoginTab::Qr, _) => crate::actions::qr_toggle(m, cx),
            }
        }))
        .child(
            gpui::div()
                .flex()
                .flex_col()
                .gap_4()
                .w(px(420.))
                .child(
                    gpui::div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            gpui::div()
                                .text_size(px(24.))
                                .text_color(rgb(TEXT_PRIMARY))
                                .child("RustyCord"),
                        )
                        .child(
                            gpui::div()
                                .text_size(px(13.))
                                .text_color(rgb(TEXT_MUTED))
                                .child(format!(
                                    "нативный клиент (GPUI) · протокол-ядро v{VERSION}"
                                )),
                        ),
                )
                .child(tabs)
                .child(card)
                .children(error.map(|e| {
                    gpui::div()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x4a_1d_24))
                        .text_color(rgb(0xf9_7c_81))
                        .text_size(px(13.))
                        .child(e)
                })),
        )
}

fn token_card(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> impl IntoElement {
    let login = match &model.screen {
        Screen::Login(l) => l,
        _ => unreachable!(),
    };
    gpui::div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(rgb(BG_PANEL))
        .child(
            gpui::div()
                .text_size(px(13.))
                .text_color(rgb(TEXT_MUTED))
                .child("Токен из веб-клиента (F12 → Network → Authorization) или бот-токен. Сессия живёт до сброса в Discord."),
        )
        .child(field(&login.token_input))
        .child(
            primary_button("Войти", login.busy).on_mouse_down(
                MouseButton::Left,
                cx.listener(|m: &mut AppModel, _, _, cx| {
                    let token = match &m.screen {
                        Screen::Login(l) => l.token_input.read(cx).text(),
                        _ => String::new(),
                    };
                    if !token.trim().is_empty() {
                        crate::actions::connect_token(m, token.trim().to_string(), cx);
                    }
                }),
            ),
        )
}

fn email_card(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> impl IntoElement {
    let login = match &model.screen {
        Screen::Login(l) => l,
        _ => unreachable!(),
    };
    gpui::div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(rgb(BG_PANEL))
        .child(field(&login.email_input))
        .child(field(&login.pass_input))
        .child(primary_button("Войти", login.busy).on_mouse_down(
            MouseButton::Left,
            cx.listener(|_m: &mut AppModel, _, _, cx| {
                crate::actions::login_email(_m, cx);
            }),
        ))
        .child(
            gpui::div()
                .text_size(px(12.))
                .text_color(rgb(TEXT_FADED))
                .child("При капче/2FA откроется окно подтверждения discord.com."),
        )
}

fn mfa_card(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> impl IntoElement {
    let login = match &model.screen {
        Screen::Login(l) => l,
        _ => unreachable!(),
    };
    gpui::div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(rgb(BG_PANEL))
        .child(
            gpui::div()
                .text_size(px(14.))
                .text_color(rgb(TEXT_PRIMARY))
                .child("Двухфакторная аутентификация"),
        )
        .child(field(&login.mfa_input))
        .child(primary_button("Подтвердить", login.busy).on_mouse_down(
            MouseButton::Left,
            cx.listener(|m: &mut AppModel, _, _, cx| {
                let code = match &m.screen {
                    Screen::Login(l) => l.mfa_input.read(cx).text(),
                    _ => String::new(),
                };
                crate::actions::login_mfa(m, code, cx);
            }),
        ))
}

fn qr_card(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> impl IntoElement {
    let stage = match &model.screen {
        Screen::Login(l) => &l.qr,
        _ => unreachable!(),
    };
    let body: AnyElement = match stage {
        QrStage::Idle => gpui::div()
            .flex()
            .flex_col()
            .gap_3()
            .items_center()
            .py_6()
            .child(
                gpui::div()
                    .text_size(px(13.))
                    .text_color(rgb(TEXT_MUTED))
                    .child("Отсканируй QR телефоном в приложении Discord:"),
            )
            .child(primary_button("Показать QR-код", false).on_mouse_down(
                MouseButton::Left,
                cx.listener(|m: &mut AppModel, _, _, cx| {
                    crate::actions::qr_toggle(m, cx);
                }),
            ))
            .into_any_element(),
        QrStage::Connecting | QrStage::Finishing => gpui::div()
            .flex()
            .py_10()
            .justify_center()
            .child(
                gpui::div()
                    .text_size(px(13.))
                    .text_color(rgb(TEXT_MUTED))
                    .child("Подключаемся к Discord…"),
            )
            .into_any_element(),
        QrStage::Matrix { modules, width, .. } => {
            // Сетка модулей: 8px на модуль, белая подложка = quiet zone.
            let cell = px(8.);
            let mut grid = gpui::div().flex().flex_col();
            for y in 0..*width {
                let mut row = gpui::div().flex();
                for x in 0..*width {
                    let dark = modules[y * width + x];
                    row = row.child(gpui::div().size(cell).map(|el| {
                        if dark {
                            el.bg(gpui::black())
                        } else {
                            el.bg(gpui::white())
                        }
                    }));
                }
                grid = grid.child(row);
            }
            gpui::div()
                .flex()
                .flex_col()
                .gap_3()
                .items_center()
                .py_2()
                .child(gpui::div().p_2().rounded_md().bg(gpui::white()).child(grid))
                .child(
                    gpui::div()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT_FADED))
                        .child("Настройки → Сканировать QR-код"),
                )
                .child(
                    gpui::div()
                        .id("qr-cancel")
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT_MUTED))
                        .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
                        .child("Отменить")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|m: &mut AppModel, _, _, cx| {
                                crate::actions::qr_toggle(m, cx);
                            }),
                        ),
                )
                .into_any_element()
        }
        QrStage::Scanned { username } => gpui::div()
            .flex()
            .flex_col()
            .gap_2()
            .items_center()
            .py_8()
            .child(
                gpui::div()
                    .text_size(px(15.))
                    .text_color(rgb(TEXT_PRIMARY))
                    .child(format!("Отсканировал: {username}")),
            )
            .child(
                gpui::div()
                    .text_size(px(13.))
                    .text_color(rgb(TEXT_MUTED))
                    .child("Подтверди вход на телефоне…"),
            )
            .into_any_element(),
        QrStage::Failed(msg) => gpui::div()
            .flex()
            .flex_col()
            .gap_3()
            .items_center()
            .py_6()
            .child(
                gpui::div()
                    .text_size(px(13.))
                    .text_color(rgb(0xf9_7c_81))
                    .child(msg.clone()),
            )
            .child(primary_button("Повторить", false).on_mouse_down(
                MouseButton::Left,
                cx.listener(|m: &mut AppModel, _, _, cx| {
                    crate::actions::qr_toggle(m, cx);
                }),
            ))
            .into_any_element(),
    };

    gpui::div()
        .p_4()
        .rounded_lg()
        .bg(rgb(BG_PANEL))
        .flex()
        .flex_col()
        .items_center()
        .child(body)
}

fn field(input: &gpui::Entity<TextInput>) -> impl IntoElement {
    gpui::div()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(BG_INPUT))
        .child(input.clone())
}

fn primary_button(label: &str, busy: bool) -> gpui::Stateful<gpui::Div> {
    gpui::div()
        .id(gpui::SharedString::from(label.to_string()))
        .flex()
        .h(px(36.))
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(rgb(BLURPLE))
        .text_color(gpui::white())
        .text_size(px(14.))
        .cursor_pointer()
        .map(|el| if busy { el.opacity(0.6) } else { el })
        .child(if busy {
            "Подождите…".to_string()
        } else {
            label.to_string()
        })
}
