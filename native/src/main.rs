//! RustyCord Native — вертикальный срез UI на GPUI (спайк).
//!
//! Цель среза: доказать жизнеспособность стека ДО большой миграции.
//! Что проверяем в этой итерации:
//!   1. GPUI 0.2.2 собирается в CI на Linux (Vulkan/Wayland-стек).
//!   2. Discord-тёмная раскладка: рельса серверов / список каналов /
//!      лента сообщений / поле ввода — на `div()`-стилях.
//!   3. Кириллица шейпится (у сообщений русский текст).
//!   4. Интерактив: клик по серверу и каналу меняет состояние и
//!      перерисовывает окно (entity-модель GPUI).
//! Что приходит следующими итерациями (порядок — в PR-описании):
//!   текстовый ввод с IME (адаптация input-примера GPUI), виртуализиро-
//!   ванный List, картинки/аватары, подключение rustycord_lib через
//!   EventSink (после рефакторинга emit-транспорта на main).

use gpui::{
    div, prelude::*, px, relative, rgb, rgba, size, App, Application, Bounds, Context, FontWeight,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};

// ---- палитра Discord (тёмная тема оригинального клиента) ----
const RAIL_BG: u32 = 0x1e1f22; // рельса серверов
const SIDEBAR_BG: u32 = 0x2b2d31; // список каналов
const MAIN_BG: u32 = 0x313338; // лента сообщений
const INPUT_BG: u32 = 0x383a40; // поле ввода
const BORDER_DARK: u32 = 0x26282d; // разделитель шапки канала
const TEXT_PRIMARY: u32 = 0xdbdee1; // основной текст
const TEXT_SECONDARY: u32 = 0x949ba4; // названия каналов
const TEXT_MUTED: u32 = 0x6d6f78; // плейсхолдер ввода, время
const ACCENT: u32 = 0x5865f2; // blurple

/// Канал текстовый (войс-каналы и категории — следующая итерация).
struct ChannelData {
    name: &'static str,
}

struct ServerData {
    name: &'static str,
    color: u32,
    channels: Vec<ChannelData>,
}

struct MessageData {
    author: &'static str,
    color: u32,
    text: &'static str,
    time: &'static str,
}

struct RustyApp {
    servers: Vec<ServerData>,
    /// Индекс выбранного сервера на рельсе.
    selected_server: usize,
    /// Индекс выбранного канала внутри выбранного сервера.
    selected_channel: usize,
    /// Статичная лента (позже — события MESSAGE_CREATE из rustycord_lib).
    messages: Vec<MessageData>,
}

impl RustyApp {
    fn new() -> Self {
        Self {
            servers: vec![
                ServerData {
                    name: "RustyCord HQ",
                    color: 0x5865f2,
                    channels: vec![
                        ChannelData { name: "правила" },
                        ChannelData { name: "общий" },
                        ChannelData { name: "devlog" },
                        ChannelData { name: "мемы" },
                    ],
                },
                ServerData {
                    name: "Zig vs Rust",
                    color: 0x23a55a,
                    channels: vec![
                        ChannelData { name: "general" },
                        ChannelData { name: "бенчмарки" },
                    ],
                },
                ServerData {
                    name: "Phoenix Watch",
                    color: 0xeb459e,
                    channels: vec![
                        ChannelData { name: "новости" },
                        ChannelData { name: "слухи" },
                    ],
                },
            ],
            selected_server: 0,
            selected_channel: 1,
            messages: vec![
                MessageData {
                    author: "Винил",
                    color: 0x5865f2,
                    text: "Привет из GPUI! Это RustyCord Native — вертикальный срез нативного клиента.",
                    time: "сегодня, в 12:00",
                },
                MessageData {
                    author: "Стас",
                    color: 0x23a55a,
                    text: "Кириллица шейпится, окно живое. Electron за окном уже почти.",
                    time: "сегодня, в 12:01",
                },
                MessageData {
                    author: "Гость",
                    color: 0xeb459e,
                    text: "А стримы? — Бэкенд уже умеет op 18/19, ждёт разводки EventSink — и летим.",
                    time: "сегодня, в 12:03",
                },
            ],
        }
    }

    /// Рельса серверов слева: кнопка ЛС + круглые чипы серверов.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chips = self.servers.iter().enumerate().map(|(i, server)| {
            let selected = i == self.selected_server;
            div()
                .id(("server", i))
                .on_click(cx.listener(move |this, _event, _window, _cx| {
                    if this.selected_server != i {
                        this.selected_server = i;
                        this.selected_channel = 0;
                    }
                }))
                .when(selected, |el| {
                    el.border_2().border_color(rgb(0xffffff)).size(px(46.))
                })
                .when(!selected, |el| el.size(px(48.)))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(server.color))
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xffffff))
                .child(initials_of(server.name))
        });

        div()
            .w(px(72.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .pt_3()
            .bg(rgb(RAIL_BG))
            .child(
                // Домашняя кнопка (ЛС) — акцентный круг с «R».
                div()
                    .id("home")
                    .size(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgb(ACCENT))
                    .text_lg()
                    .font_weight(FontWeight::BLACK)
                    .text_color(rgb(0xffffff))
                    .child("R"),
            )
            .children(chips)
    }

    /// Список каналов выбранного сервера.
    fn render_sidebar(&self, server: &ServerData, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = server.channels.iter().enumerate().map(|(i, ch)| {
            let selected = i == self.selected_channel;
            div()
                .id(("channel", i))
                .on_click(cx.listener(move |this, _event, _window, _cx| {
                    this.selected_channel = i;
                }))
                .when(selected, |el| el.bg(rgba(0xffffff0d)))
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .text_sm()
                .text_color(rgb(if selected {
                    TEXT_PRIMARY
                } else {
                    TEXT_SECONDARY
                }))
                .child("#")
                .child(ch.name)
        });

        div()
            .w(px(240.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(SIDEBAR_BG))
            .child(
                // Шапка с именем сервера (выпадающее меню — позже).
                div()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .px_4()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xf2f3f5))
                    .child(server.name),
            )
            .child(
                div()
                    .id("channels")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows),
            )
    }

    /// Одно сообщение: аватар-кружок + автор/время + текст.
    fn render_message(&self, m: &MessageData) -> impl IntoElement {
        div()
            .flex()
            .gap_3()
            .child(
                div()
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgb(m.color))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0xffffff))
                    .child(initials_of(m.author)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(m.color))
                                    .child(m.author),
                            )
                            .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child(m.time)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .line_height(relative(1.4))
                            .text_color(rgb(TEXT_PRIMARY))
                            .child(m.text),
                    ),
            )
    }

    /// Основная область: шапка канала, лента, поле ввода.
    fn render_main(&self, channel: &ChannelData) -> impl IntoElement {
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(MAIN_BG))
            .child(
                div()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .border_b_1()
                    .border_color(rgb(BORDER_DARK))
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xf2f3f5))
                    .child("#")
                    .child(channel.name),
            )
            .child(
                div()
                    .id("messages")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_4()
                    .py_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(self.messages.iter().map(|m| self.render_message(m))),
            )
            .child(
                // Поле ввода: пока визуал; интерактивный ввод с IME —
                // адаптация input-примера GPUI, следующая итерация.
                div().px_4().pb_4().child(
                    div()
                        .h(px(44.))
                        .flex()
                        .items_center()
                        .px_4()
                        .rounded(px(8.))
                        .bg(rgb(INPUT_BG))
                        .text_sm()
                        .text_color(rgb(TEXT_MUTED))
                        .child(format!("Написать на #{}", channel.name)),
                ),
            )
    }
}

impl Render for RustyApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let server = &self.servers[self.selected_server];
        let channel = &server.channels[self.selected_channel];
        div()
            .flex()
            .size_full()
            .bg(rgb(MAIN_BG))
            .text_color(rgb(TEXT_PRIMARY))
            .child(self.render_rail(cx))
            .child(self.render_sidebar(server, cx))
            .child(self.render_main(channel))
    }
}

/// До двух заглавных букв имени («RustyCord HQ» → «RH») — вместо иконок,
/// пока не подключён Img и CDN-аватары.
fn initials_of(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.len() {
        0 => "?".into(),
        1 => words[0]
            .chars()
            .take(2)
            .flat_map(|c| c.to_uppercase())
            .collect(),
        _ => {
            let first = words[0].chars().next().unwrap_or('?');
            let second = words[1].chars().next().unwrap_or('?');
            let mut s = String::new();
            s.extend(first.to_uppercase());
            s.extend(second.to_uppercase());
            s
        }
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("RustyCord Native".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| RustyApp::new()),
        )
        .unwrap();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_of_handles_cyrillic_and_latin() {
        assert_eq!(initials_of("RustyCord HQ"), "RH");
        assert_eq!(initials_of("Винил"), "ВИ");
        assert_eq!(initials_of(""), "?");
    }
}
