//! Главный экран: рейл серверов | каналы | чат | участники.
//! Канонические данные (гильдии/каналы/участники/войс) читаются напрямую
//! из кэша ядра снапшотами (RwLock не удерживается через слушатели).

use chrono::DateTime;
use gpui::{
    prelude::*, px, rgb, AnyElement, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    Window,
};
use rustycord_core::rest::models::{Channel, Message, VoiceState};
use rustycord_core::state::{AppState, MemberListRow};
use std::collections::HashMap;

use crate::state::{AppModel, Screen};
use crate::text_input::Submit;
use crate::theme::*;

/* ---------------------------- Снапшоты ---------------------------- */

#[allow(dead_code)]
struct UiGuild {
    id: String,
    name: String,
    icon_url: Option<String>,
    /// Текстовые категории и каналы (type 4/0/5/15 — без голоса).
    text_groups: Vec<UiChannelGroup>,
    /// Голосовые (type 2/13) + участники.
    voice_channels: Vec<UiVoiceChannel>,
}

struct UiChannelGroup {
    label: String,
    channels: Vec<Channel>,
}

struct UiVoiceChannel {
    channel: Channel,
    members: Vec<VoiceState>,
}

/// Снять снапшот гильдии из кэша ядра (каналы сгруппированы по категориям).
fn snapshot_guild(core: &AppState, guild_id: &str) -> Option<UiGuild> {
    let cache = core.cache.read().ok()?;
    let g = cache.guilds.get(guild_id)?;
    let mut categories: Vec<&Channel> = g.channels.iter().filter(|c| c.kind == 4).collect();
    categories.sort_by_key(|c| c.position.unwrap_or(0));

    let mut text_groups: Vec<UiChannelGroup> = Vec::new();
    // Каналы без категории — первой группой.
    let orphan: Vec<Channel> = g
        .channels
        .iter()
        .filter(|c| matches!(c.kind, 0 | 5 | 15) && c.parent_id.is_none())
        .cloned()
        .collect();
    if !orphan.is_empty() {
        text_groups.push(UiChannelGroup {
            label: "КАНАЛЫ".into(),
            channels: orphan,
        });
    }
    for cat in categories {
        let mut chans: Vec<Channel> = g
            .channels
            .iter()
            .filter(|c| {
                matches!(c.kind, 0 | 5 | 15) && c.parent_id.as_deref() == Some(cat.id.as_str())
            })
            .cloned()
            .collect();
        chans.sort_by_key(|c| c.position.unwrap_or(0));
        text_groups.push(UiChannelGroup {
            label: cat.name.clone().unwrap_or_default().to_uppercase(),
            channels: chans,
        });
    }
    let mut voice: Vec<UiVoiceChannel> = g
        .channels
        .iter()
        .filter(|c| matches!(c.kind, 2 | 13))
        .map(|ch| UiVoiceChannel {
            members: g
                .voice_states
                .iter()
                .filter(|vs| vs.channel_id.as_deref() == Some(ch.id.as_str()))
                .cloned()
                .collect(),
            channel: ch.clone(),
        })
        .collect();
    voice.sort_by_key(|v| v.channel.position.unwrap_or(0));

    Some(UiGuild {
        id: g.id.clone(),
        name: g.name.clone(),
        icon_url: g.icon.as_ref().map(|h| guild_icon_url(&g.id, h)),
        text_groups,
        voice_channels: voice,
    })
}

fn guild_icon_url(guild_id: &str, hash: &str) -> String {
    let ext = if hash.starts_with("a_") { "gif" } else { "png" };
    format!("https://cdn.discordapp.com/icons/{guild_id}/{hash}.{ext}?size=128")
}

pub fn avatar_url(user_id: &str, hash: &str) -> String {
    let ext = if hash.starts_with("a_") { "gif" } else { "png" };
    format!("https://cdn.discordapp.com/avatars/{user_id}/{hash}.{ext}?size=64")
}

/// Display-имя из author/member.
fn display_name(m: &Message) -> String {
    if let Some(a) = &m.author {
        a.global_name
            .clone()
            .or_else(|| Some(a.username.clone()))
            .unwrap_or_default()
    } else {
        "???".into()
    }
}

fn time_hm(ts: &Option<String>) -> String {
    ts.as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn time_day(ts: &Option<String>) -> String {
    ts.as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.format("%d.%m.%Y").to_string())
        .unwrap_or_default()
}

/* ------------------------------ Рендер ------------------------------ */

pub fn render(
    model: &mut AppModel,
    _window: &mut Window,
    cx: &mut gpui::Context<AppModel>,
) -> gpui::AnyElement {
    let (sel_guild, sel_channel, settings_open) = match &model.screen {
        Screen::Main(m) => (
            m.selected_guild.clone(),
            m.selected_channel.clone(),
            m.settings_open,
        ),
        _ => (None, None, false),
    };

    // Настройки — полностраничный экран (как в официальном клиенте).
    if settings_open {
        return crate::settings::render(model, cx).into_any_element();
    }

    gpui::div()
        .flex()
        .size_full()
        .font_family(ui_font())
        .text_size(px(14.))
        .text_color(rgb(TEXT_PRIMARY))
        .child(guild_rail(model, cx, sel_guild.as_deref()))
        .child(channel_sidebar(
            model,
            cx,
            sel_guild.as_deref(),
            sel_channel.as_deref(),
        ))
        .child(chat_pane(model, cx, sel_channel.as_deref()))
        .child(member_pane(
            model,
            cx,
            sel_guild.as_deref(),
            sel_channel.as_deref(),
        ))
        .into_any_element()
}

/* --------------------------- Рейл гильдий --------------------------- */

/// Кнопка-плитка сервера 48×48 (кружок → скруглённый квадрат на ховере,
/// как в официальном клиенте). active — белый прямоугольник слева.
fn rail_guild_tile(
    cx: &mut gpui::Context<AppModel>,
    id: &str,
    name: &str,
    icon: Option<String>,
    active: bool,
    unread: u32,
    on_click: impl Fn(&mut AppModel, &mut gpui::Context<AppModel>) + 'static,
) -> impl IntoElement {
    let initials: SharedString = name
        .chars()
        .filter(|c| c.is_alphabetic())
        .take(2)
        .collect::<String>()
        .to_uppercase()
        .into();
    let tile = gpui::div()
        .id(gpui::SharedString::from(format!("rail-g-{id}")))
        .size(px(48.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .cursor_pointer()
        .map(|el| {
            if active {
                el.bg(rgb(BLURPLE))
                    .text_color(gpui::white())
                    .rounded(px(16.))
            } else {
                el.bg(rgb(0x313338))
                    .text_color(rgb(TEXT_PRIMARY))
                    .hover(|s| s.bg(rgb(BLURPLE)).text_color(gpui::white()).rounded(px(16.)))
            }
        })
        .child(match icon {
            Some(url) => gpui::img(url).size(px(48.)).into_any_element(),
            None => gpui::div()
                .text_size(px(16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(initials)
                .into_any_element(),
        })
        .on_mouse_down(MouseButton::Left, cx.listener(move |m, _, _, cx| on_click(m, cx)));

    // Строка: пилюля активности + плитка + бейдж непрочитанных.
    gpui::div()
        .relative()
        .w(px(72.))
        .flex()
        .items_center()
        .justify_center()
        .when(active, |row| {
            row.child(
                gpui::div()
                    .absolute()
                    .left_0()
                    .w(px(4.))
                    .h(px(40.))
                    .rounded_full()
                    .bg(gpui::white()),
            )
        })
        .child(tile)
        .when(unread > 0 && !active, |row| {
            row.child(
                gpui::div()
                    .absolute()
                    .right_1()
                    .bottom_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(16.))
                    .h(px(16.))
                    .px_1()
                    .rounded_full()
                    .bg(rgb(RED))
                    .text_color(gpui::white())
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(if unread > 99 {
                        "99+".to_string()
                    } else {
                        unread.to_string()
                    }),
            )
        })
}

/// Свёрнутая папка: 48×48 с сеткой 2×2 мини-иконок (до 4 серверов + «+N»).
/// Плитка становится цветом папки на ховере и когда внутри выбран сервер.
fn rail_folder_tile(
    cx: &mut gpui::Context<AppModel>,
    folder: &crate::state::GuildFolder,
    index: usize,
    tiles: &HashMap<String, (String, Option<String>, u32)>,
    active: bool,
    unread: u32,
) -> impl IntoElement {
    let color = folder
        .color
        .map(|c| rgb((c & 0xffffff) as u32))
        .unwrap_or(rgb(BLURPLE));
    let mut body = gpui::div()
        .id("rail-folder")
        .size(px(48.))
        .rounded_full()
        .overflow_hidden()
        .cursor_pointer()
        .bg(rgb(0x313338))
        .hover(|s| s.bg(color).rounded(px(16.)))
        .relative();
    if active {
        body = body.bg(color).rounded(px(16.));
    }
    // Сетка 2×2 из мини-иконок (углы, отступ 6px).
    for (i, gid) in folder.guild_ids.iter().take(4).enumerate() {
        let (top, left) = match i {
            0 => (true, true),
            1 => (true, false),
            2 => (false, true),
            _ => (false, false),
        };
        let tile_info = tiles.get(gid);
        let mini: gpui::AnyElement = match tile_info {
            Some((_, Some(url), _)) => gpui::div()
                .absolute()
                .when(top, |el| el.top_1p5())
                .when(!top, |el| el.bottom_1p5())
                .when(left, |el| el.left_1p5())
                .when(!left, |el| el.right_1p5())
                .size(px(16.))
                .rounded_full()
                .overflow_hidden()
                .child(gpui::img(url.clone()).size(px(16.)))
                .into_any_element(),
            Some((name, None, _)) => gpui::div()
                .absolute()
                .when(top, |el| el.top_1p5())
                .when(!top, |el| el.bottom_1p5())
                .when(left, |el| el.left_1p5())
                .when(!left, |el| el.right_1p5())
                .size(px(16.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(avatar_color(gid))
                .text_size(px(9.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(gpui::white())
                .child(first_letter(name))
                .into_any_element(),
            // Гильдия ещё грузится (раскладка приехала раньше GUILD_CREATE).
            None => gpui::div()
                .absolute()
                .when(top, |el| el.top_1p5())
                .when(!top, |el| el.bottom_1p5())
                .when(left, |el| el.left_1p5())
                .when(!left, |el| el.right_1p5())
                .size(px(16.))
                .rounded_full()
                .bg(rgb(0x35363c))
                .into_any_element(),
        };
        body = body.child(mini);
    }
    let row = gpui::div()
        .relative()
        .w(px(72.))
        .flex()
        .items_center()
        .justify_center()
        .when(active, |r| {
            r.child(
                gpui::div()
                    .absolute()
                    .left_0()
                    .w(px(4.))
                    .h(px(40.))
                    .rounded_full()
                    .bg(gpui::white()),
            )
        })
        .child(body.on_mouse_down(MouseButton::Left, cx.listener(move |m, _, _, cx| {
            // Клик по свёрнутой папке — раскрыть столбиком (повторный клик
            // по шапке сворачивает).
            if let Screen::Main(st) = &mut m.screen {
                st.open_folder = Some(index);
            }
            cx.notify();
        })))
        .when(unread > 0 && !active, |r| {
            r.child(
                gpui::div()
                    .absolute()
                    .right_1()
                    .bottom_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(16.))
                    .h(px(16.))
                    .px_1()
                    .rounded_full()
                    .bg(rgb(RED))
                    .text_color(gpui::white())
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(if unread > 99 {
                        "99+".to_string()
                    } else {
                        unread.to_string()
                    }),
            )
        });
    row
}

/// Раскрытая папка: цветной столбик с шапкой-именем и серверами внутри.
fn rail_folder_open(
    cx: &mut gpui::Context<AppModel>,
    folder: &crate::state::GuildFolder,
    tiles: &HashMap<String, (String, Option<String>, u32)>,
    selected: Option<&str>,
) -> impl IntoElement {
    let color = folder
        .color
        .map(|c| rgb((c & 0xffffff) as u32))
        .unwrap_or(rgb(BLURPLE));
    let title = folder
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .or_else(|| {
            folder
                .guild_ids
                .iter()
                .find_map(|gid| tiles.get(gid).map(|(n, _, _)| n.clone()))
        })
        .unwrap_or_else(|| "Папка".into());

    // Шапка: имя папки, клик — свернуть.
    let head = gpui::div()
        .id("folder-head")
        .h(px(20.))
        .min_w(px(48.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(gpui::white())
        .hover(|s| s.opacity(0.8))
        .child(title.clone())
        .on_mouse_down(MouseButton::Left, cx.listener(|m, _, _, cx| {
            if let Screen::Main(st) = &mut m.screen {
                st.open_folder = None;
            }
            cx.notify();
        }));

    let mut col = gpui::div()
        .id(SharedString::from(format!("folder-open-{}", title)))
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .py_2()
        .px_1()
        .w(px(56.))
        .rounded_xl()
        .bg(color);
    col = col.child(head);
    for gid in folder.guild_ids.iter() {
        let Some((name, icon, unread)) = tiles.get(gid) else {
            continue;
        };
        let active = selected == Some(gid.as_str());
        let g = gid.clone();
        col = col.child(rail_guild_tile(
            cx,
            gid,
            name,
            icon.clone(),
            active,
            *unread,
            move |m, cx| {
                // Клик по серверу папку не сворачивает (как в оригинале).
                crate::actions::select_guild(m, Some(g.clone()), cx);
            },
        ));
    }
    col
}
fn guild_rail(
    model: &mut AppModel,
    cx: &mut gpui::Context<AppModel>,
    selected: Option<&str>,
) -> impl IntoElement {
    // Снимок плиток: id → (имя, иконка, непрочитанные) — кэш ядра.
    let tiles: HashMap<String, (String, Option<String>, u32)> = {
        let cache = model.core.cache.read().unwrap();
        cache
            .guilds
            .values()
            .map(|g| {
                let unread: u32 = match &model.screen {
                    Screen::Main(m) => g
                        .channels
                        .iter()
                        .map(|c| m.unread.get(&c.id).copied().unwrap_or(0))
                        .sum(),
                    _ => 0,
                };
                (
                    g.id.clone(),
                    (
                        g.name.clone(),
                        g.icon.as_ref().map(|h| guild_icon_url(&g.id, h)),
                        unread,
                    ),
                )
            })
            .collect()
    };
    let (rail, folders, open_folder) = match &model.screen {
        Screen::Main(m) => (m.rail.clone(), m.folders.clone(), m.open_folder),
        _ => (Vec::new(), Vec::new(), None),
    };
    // Раскладка ещё не приезжала — все известные серверы подряд (гильдии
    // из папок попадут сюда только если события раскладки не было вовсе).
    let rail: Vec<crate::state::RailItem> = if rail.is_empty() {
        tiles.keys().cloned().map(crate::state::RailItem::Guild).collect()
    } else {
        rail
    };

    let mut column = gpui::div().flex().flex_col().gap_2().items_center().py_3();

    // Домашняя кнопка: краб-логотип (клик — ЛС/друзья).
    let home_active = selected.is_none();
    column = column.child(
        gpui::div()
            .relative()
            .w(px(72.))
            .flex()
            .items_center()
            .justify_center()
            .when(home_active, |r| {
                r.child(
                    gpui::div()
                        .absolute()
                        .left_0()
                        .w(px(4.))
                        .h(px(40.))
                        .rounded_full()
                        .bg(gpui::white()),
                )
            })
            .child(
                gpui::div()
                    .id("rail-home")
                    .size(px(48.))
                    .rounded(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .map(|el| {
                        if home_active {
                            el.bg(rgb(BLURPLE))
                        } else {
                            el.hover(|s| s.bg(rgb(BLURPLE)))
                        }
                    })
                    .child(gpui::img(crate::assets::logo_image()).size(px(26.)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|m: &mut AppModel, _, _, cx| {
                            crate::actions::select_guild(m, None, cx);
                        }),
                    ),
            ),
    );
    // Разделитель (как в официальном клиенте).
    column =
        column.child(gpui::div().w(px(32.)).h(px(2.)).rounded_full().bg(rgb(0x35363c)));

    for item in rail {
        match item {
            crate::state::RailItem::Guild(id) => {
                let Some((name, icon, unread)) = tiles.get(&id) else {
                    continue;
                };
                let active = selected == Some(id.as_str());
                let gid = id.clone();
                column = column.child(rail_guild_tile(
                    cx,
                    &id,
                    name,
                    icon.clone(),
                    active,
                    *unread,
                    move |m, cx| crate::actions::select_guild(m, Some(gid.clone()), cx),
                ));
            }
            crate::state::RailItem::Folder(i) => {
                let Some(folder) = folders.get(i) else {
                    continue;
                };
                let expanded = open_folder == Some(i);
                let active = folder
                    .guild_ids
                    .iter()
                    .any(|g| selected == Some(g.as_str()));
                let unread: u32 = folder
                    .guild_ids
                    .iter()
                    .map(|g| tiles.get(g).map(|t| t.2).unwrap_or(0))
                    .sum();
                if expanded {
                    column = column.child(rail_folder_open(cx, folder, &tiles, selected));
                } else {
                    column = column.child(rail_folder_tile(cx, folder, i, &tiles, active, unread));
                }
            }
        }
    }

    gpui::div()
        .w(px(72.))
        .h_full()
        .flex()
        .flex_col()
        .bg(rgb(BG_BASE))
        .child(
            gpui::div()
                .id("rail-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(column),
        )
}

/* ------------------------ Сайдбар каналов ------------------------ */

fn channel_sidebar(
    model: &mut AppModel,
    cx: &mut gpui::Context<AppModel>,
    selected_guild: Option<&str>,
    selected_channel: Option<&str>,
) -> impl IntoElement {
    let (header, body): (AnyElement, AnyElement) = match selected_guild {
        Some(gid) => {
            let g = snapshot_guild(&model.core, gid);
            let guild_name = g.as_ref().map(|x| x.name.clone()).unwrap_or_default();
            let mut col = gpui::div().flex().flex_col().px_2().gap_2();

            if let Some(g) = g {
                // Текстовые группы.
                for group in &g.text_groups {
                    if group.channels.is_empty() {
                        continue;
                    }
                    col = col.child(
                        gpui::div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_FADED))
                            .child(group.label.clone()),
                    );
                    for ch in &group.channels {
                        let active = selected_channel == Some(ch.id.as_str());
                        let unread = match &model.screen {
                            Screen::Main(m) => m.unread.get(&ch.id).copied().unwrap_or(0),
                            _ => 0,
                        };
                        let (cid, cname) = (ch.id.clone(), ch.name.clone().unwrap_or_default());
                        col = col.child(
                            gpui::div()
                                .id(gpui::SharedString::from(format!("ch-{cid}")))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_size(px(14.))
                                .cursor_pointer()
                                .map(|el| {
                                    if active {
                                        el.bg(rgb(BG_ACTIVE)).text_color(rgb(TEXT_PRIMARY))
                                    } else {
                                        el.text_color(rgb(TEXT_MUTED)).hover(|s| {
                                            s.bg(rgb(BG_HOVER)).text_color(rgb(TEXT_PRIMARY))
                                        })
                                    }
                                })
                                .child(gpui::div().text_color(rgb(TEXT_FADED)).child("#"))
                                .child(cname.clone())
                                .when(unread > 0 && !active, |el| {
                                    el.child(
                                        gpui::div()
                                            .ml_auto()
                                            .min_w(px(18.))
                                            .h(px(18.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .px_1()
                                            .rounded_full()
                                            .bg(rgb(RED))
                                            .text_color(gpui::white())
                                            .text_size(px(11.))
                                            .child(unread.to_string()),
                                    )
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |m: &mut AppModel, _, _, cx| {
                                        crate::actions::select_channel(m, &cid.clone(), cx);
                                    }),
                                ),
                        );
                    }
                }
                // Голосовые каналы + участники.
                if !g.voice_channels.is_empty() {
                    col = col.child(
                        gpui::div()
                            .px_2()
                            .pt_3()
                            .pb_1()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_FADED))
                            .child("ГОЛОС"),
                    );
                }
                for vc in &g.voice_channels {
                    col = col.child(voice_channel_row(model, cx, gid, vc));
                }
            } else {
                col = col.child(
                    gpui::div()
                        .px_2()
                        .py_2()
                        .text_size(px(13.))
                        .text_color(rgb(TEXT_MUTED))
                        .child("Подписка на сервер… (op 37)"),
                );
            }

            (
                gpui::div()
                    .px_4()
                    .py_3()
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(guild_name)
                    .into_any_element(),
                gpui::div()
                    .id("chans-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(col)
                    .into_any_element(),
            )
        }
        None => {
            // Дом: ЛС + друзья.
            let (dms, friends) = {
                let cache = model.core.cache.read().unwrap();
                let dms: Vec<(String, String, Option<String>, u32)> = cache
                    .dms
                    .iter()
                    .map(|ch| {
                        let name = dm_name(ch);
                        let icon = ch
                            .recipients
                            .as_ref()
                            .and_then(|r| r.first())
                            .and_then(|u| u.avatar.as_ref())
                            .map(|h| avatar_url(&u_id(ch), h));
                        let unread = match &model.screen {
                            Screen::Main(m) => m.unread.get(&ch.id).copied().unwrap_or(0),
                            _ => 0,
                        };
                        (ch.id.clone(), name, icon, unread)
                    })
                    .collect();
                let friends: Vec<(String, String, String, Option<String>)> = cache
                    .relationships
                    .values()
                    .filter(|r| r.kind == 1)
                    .map(|r| {
                        let status = cache
                            .friend_presences
                            .get(&r.user.id)
                            .map(|p| p.status.clone())
                            .unwrap_or_else(|| "offline".into());
                        (
                            r.user.id.clone(),
                            r.user
                                .global_name
                                .clone()
                                .unwrap_or_else(|| r.user.username.clone()),
                            status,
                            r.user.avatar.clone(),
                        )
                    })
                    .collect();
                (dms, friends)
            };

            let mut col = gpui::div().flex().flex_col().px_2().gap_1();
            col = col.child(
                gpui::div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_FADED))
                    .child("ЛИЧНЫЕ СООБЩЕНИЯ"),
            );
            for (id, name, icon, unread) in dms {
                let active = selected_channel == Some(id.as_str());
                let (did, dname) = (id.clone(), name.clone());
                let row = gpui::div()
                    .id(gpui::SharedString::from(format!("dm-{id}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .map(|el| {
                        if active {
                            el.bg(rgb(BG_ACTIVE)).text_color(rgb(TEXT_PRIMARY))
                        } else {
                            el.text_color(rgb(TEXT_MUTED))
                                .hover(|s| s.bg(rgb(BG_HOVER)).text_color(rgb(TEXT_PRIMARY)))
                        }
                    })
                    .child(avatar_or_initial(&id, icon.as_deref(), &name, px(24.)))
                    .child(dname.clone())
                    .when(unread > 0 && !active, |el| {
                        el.child(
                            gpui::div()
                                .ml_auto()
                                .min_w(px(18.))
                                .h(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .px_1()
                                .rounded_full()
                                .bg(rgb(RED))
                                .text_color(gpui::white())
                                .text_size(px(11.))
                                .child(unread.to_string()),
                        )
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |m: &mut AppModel, _, _, cx| {
                            // ДМ выбирается как канал «домашней» гильдии.
                            if let Screen::Main(state) = &mut m.screen {
                                state.selected_guild = None;
                            }
                            crate::actions::select_channel(m, &did.clone(), cx);
                        }),
                    );
                col = col.child(row);
            }
            col = col.child(
                gpui::div()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_FADED))
                    .child("ДРУЗЬЯ"),
            );
            for (uid, name, status, avatar) in friends {
                let dm_uid = uid.clone();
                let av_url = avatar.as_deref().map(|h| avatar_url(&uid, h));
                col = col.child(
                    gpui::div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_color(rgb(TEXT_MUTED))
                        .hover(|s| s.bg(rgb(BG_HOVER)).text_color(rgb(TEXT_PRIMARY)))
                        // Аватар с точкой статуса.
                        .child(
                            gpui::div()
                                .relative()
                                .size(px(32.))
                                .child(avatar_or_initial(&uid, av_url.as_deref(), &name, px(32.)))
                                .child(
                                    gpui::div()
                                        .absolute()
                                        .bottom_0()
                                        .right_0()
                                        .size(px(14.))
                                        .rounded_full()
                                        .bg(rgb(BG_PANEL))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            gpui::div()
                                                .size(px(10.))
                                                .rounded_full()
                                                .bg(status_color(&status)),
                                        ),
                                ),
                        )
                        .child(
                            gpui::div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(
                                    gpui::div()
                                        .text_size(px(14.))
                                        .text_color(rgb(TEXT_PRIMARY))
                                        .overflow_hidden()
                                        .child(name),
                                )
                                .child(
                                    gpui::div()
                                        .text_size(px(11.))
                                        .text_color(rgb(TEXT_FADED))
                                        .child(status_label(&status)),
                                ),
                        )
                        // «Написать»: создать/открыть ЛС с другом.
                        .child(
                            gpui::div()
                                .id(gpui::SharedString::from(format!("fr-dm-{uid}")))
                                .ml_auto()
                                .px_1()
                                .rounded_md()
                                .text_size(px(11.))
                                .text_color(rgb(TEXT_FADED))
                                .cursor_pointer()
                                .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
                                .child("✉")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |m: &mut AppModel, _, _, cx| {
                                        crate::actions::create_dm(m, &dm_uid.clone(), cx);
                                    }),
                                ),
                        ),
                );
            }

            (
                gpui::div()
                    .px_4()
                    .py_3()
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Личные сообщения")
                    .into_any_element(),
                gpui::div()
                    .id("dms-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(col)
                    .into_any_element(),
            )
        }
    };

    gpui::div()
        .w(px(240.))
        .h_full()
        .flex()
        .flex_col()
        .bg(rgb(BG_PANEL))
        .child(header)
        .child(body)
        .child(user_panel(model, cx))
}

fn u_id(ch: &Channel) -> String {
    ch.recipients
        .as_ref()
        .and_then(|r| r.first())
        .map(|u| u.id.clone())
        .unwrap_or_default()
}

fn dm_name(ch: &Channel) -> String {
    match ch.recipients.as_ref() {
        Some(rs) if rs.len() == 1 => rs[0]
            .global_name
            .clone()
            .or_else(|| Some(rs[0].username.clone()))
            .unwrap_or_default(),
        Some(rs) if rs.len() > 1 => {
            format!("{} участников", rs.len())
        }
        _ => "ЛС".into(),
    }
}

fn voice_channel_row(
    model: &mut AppModel,
    cx: &mut gpui::Context<AppModel>,
    guild_id: &str,
    vc: &UiVoiceChannel,
) -> impl IntoElement {
    let connected_here = {
        let target = model.core.voice_target.lock().unwrap().clone();
        target
            .map(|t| t.channel_id == vc.channel.id)
            .unwrap_or(false)
    };
    let speaking = match &model.screen {
        Screen::Main(m) => vc.members.iter().any(|vs| m.speaking.contains(&vs.user_id)),
        _ => false,
    };
    // Участники голосового канала (аватарки — из кэша ядра).
    let member_rows: Vec<(String, String, Option<String>)> = {
        let cache = model.core.cache.read().unwrap();
        vc.members
            .iter()
            .map(|vs| {
                let mem = cache.guilds.get(guild_id).and_then(|g| g.members.get(&vs.user_id));
                let name = mem
                    .and_then(|m| {
                        m.nick.clone().or_else(|| {
                            m.user
                                .as_ref()
                                .and_then(|u| u.global_name.clone().or(Some(u.username.clone())))
                        })
                    })
                    .or_else(|| {
                        // Гидратации кэша могло не быть — аватар из ленивого списка.
                        let lists = model.core.member_lists.lock().unwrap();
                        lists
                            .get(guild_id)
                            .and_then(|l| {
                                l.rows.iter().find_map(|r| match r {
                                    MemberListRow::Member {
                                        user_id,
                                        global_name,
                                        nick,
                                        username,
                                        avatar,
                                        ..
                                    } if *user_id == vs.user_id => Some((
                                        nick.clone()
                                            .or(global_name.clone())
                                            .unwrap_or_else(|| username.clone()),
                                        avatar.clone(),
                                    )),
                                    _ => None,
                                })
                            })
                            .map(|(n, _)| n)
                    })
                    .unwrap_or_else(|| "…".into());
                let av = mem
                    .and_then(|m| m.user.as_ref().and_then(|u| u.avatar.clone()))
                    .or_else(|| {
                        let lists = model.core.member_lists.lock().unwrap();
                        lists.get(guild_id).and_then(|l| {
                            l.rows.iter().find_map(|r| match r {
                                MemberListRow::Member {
                                    user_id, avatar, ..
                                } if *user_id == vs.user_id => avatar.clone(),
                                _ => None,
                            })
                        })
                    })
                    .map(|h| avatar_url(&vs.user_id, &h));
                (vs.user_id.clone(), name, av)
            })
            .collect()
    };

    let mut col = gpui::div().flex().flex_col();
    let (cid, cname) = (
        vc.channel.id.clone(),
        vc.channel.name.clone().unwrap_or_default(),
    );
    col = col.child(
        gpui::div()
            .id(gpui::SharedString::from(format!("vc-{cid}")))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .text_size(px(14.))
            .cursor_pointer()
            .map(|el| {
                if connected_here {
                    el.bg(rgb(BG_ACTIVE)).text_color(rgb(TEXT_PRIMARY))
                } else {
                    el.text_color(rgb(TEXT_MUTED))
                        .hover(|s| s.bg(rgb(BG_HOVER)).text_color(rgb(TEXT_PRIMARY)))
                }
            })
            .when(speaking, |el| el.text_color(rgb(GREEN)))
            .child("🔊")
            .child(cname.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |m: &mut AppModel, _, _, cx| {
                    let target = m.core.voice_target.lock().unwrap().clone();
                    let same = target.map(|t| t.channel_id == cid).unwrap_or(false);
                    let gid = match &m.screen {
                        Screen::Main(st) => st.selected_guild.clone(),
                        _ => None,
                    };
                    if same {
                        crate::actions::voice_join_leave(m, gid, None, cx);
                    } else {
                        crate::actions::voice_join_leave(m, gid, Some(cid.clone()), cx);
                    }
                }),
            ),
    );
    for (uid, name, av) in member_rows {
        let is_speaking = match &model.screen {
            Screen::Main(m) => m.speaking.contains(&uid),
            _ => false,
        };
        col = col.child(
            gpui::div()
                .id(gpui::SharedString::from(format!("vcm-{uid}")))
                .flex()
                .items_center()
                .gap_2()
                .pl_6()
                .py_0p5()
                .text_size(px(13.))
                .text_color(rgb(TEXT_MUTED))
                .when(is_speaking, |el| el.text_color(rgb(GREEN)))
                .child(avatar_or_initial(&uid, av.as_deref(), &name, px(20.)))
                .child(name),
        );
    }
    col
}

/* --------------------------- Панель юзера --------------------------- */

fn user_panel(model: &mut AppModel, cx: &mut gpui::Context<AppModel>) -> impl IntoElement {
    let me = {
        let cache = model.core.cache.read().unwrap();
        cache.user.clone()
    };
    let (in_voice, voice_channel_name, self_mute, self_deaf) = {
        let target = model.core.voice_target.lock().unwrap().clone();
        let (m, d) = target
            .as_ref()
            .map(|t| m_core_desired(&model.core, t.guild_id.as_deref().unwrap_or(&t.channel_id)))
            .unwrap_or((false, false));
        let name = target
            .as_ref()
            .map(|t| {
                // Имя канала из кэша.
                let cache = model.core.cache.read().unwrap();
                if let Some(gid) = &t.guild_id {
                    cache
                        .guilds
                        .get(gid)
                        .and_then(|g| {
                            g.channels
                                .iter()
                                .find(|c| c.id == t.channel_id)
                                .and_then(|c| c.name.clone())
                        })
                        .unwrap_or_default()
                } else {
                    cache
                        .dms
                        .iter()
                        .find(|c| c.id == t.channel_id)
                        .map(dm_name)
                        .unwrap_or_default()
                }
            })
            .unwrap_or_default();
        (target.is_some(), name, m, d)
    };

    let name = me
        .as_ref()
        .map(|u| {
            u.global_name
                .clone()
                .or_else(|| Some(u.username.clone()))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let avatar = me
        .as_ref()
        .and_then(|u| u.avatar.as_ref().map(|h| avatar_url(&u.id, h)));

    // Состояние стрима/камеры — для кнопок голосовой панели.
    let (streaming, stream_paused) = match &model.screen {
        Screen::Main(m) => {
            let paused = m
                .my_stream_key
                .as_ref()
                .and_then(|k| m.streams.get(k))
                .map(|st| st.paused)
                .unwrap_or(false);
            (m.my_stream_key.is_some(), paused)
        }
        _ => (false, false),
    };
    let camera_on = matches!(&model.screen, Screen::Main(m) if m.camera_on);

    let mut panel = gpui::div().flex().flex_col().bg(rgb(0x232428));

    if in_voice {
        let mut voice_row = gpui::div().flex().items_center().gap_2().px_2().pt_2();
        voice_row = voice_row.child(
            gpui::div()
                .flex()
                .flex_col()
                .child(
                    gpui::div()
                        .text_size(px(13.))
                        .text_color(rgb(GREEN))
                        .child("🔊 Голос"),
                )
                .child(
                    gpui::div()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT_MUTED))
                        .child(voice_channel_name),
                ),
        );
        // Кнопки трансляции: Go Live (op 18), пауза (op 22), камера (op 4).
        let mut btns = gpui::div().flex().items_center().gap_1().ml_auto();
        btns = btns.child(
            gpui::div()
                .id("btn-golive")
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_size(px(12.))
                .cursor_pointer()
                .map(|el| {
                    if streaming {
                        el.bg(rgb(RED)).text_color(gpui::white())
                    } else {
                        el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_MUTED))
                    }
                })
                .child("🖥")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|m: &mut AppModel, _, _, cx| {
                        crate::actions::stream_toggle(m, cx);
                    }),
                ),
        );
        if streaming {
            btns = btns.child(
                gpui::div()
                    .id("btn-stream-pause")
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .map(|el| {
                        if stream_paused {
                            el.bg(rgb(BLURPLE)).text_color(gpui::white())
                        } else {
                            el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_MUTED))
                        }
                    })
                    .child("⏸")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|m: &mut AppModel, _, _, cx| {
                            crate::actions::stream_pause_toggle(m, cx);
                        }),
                    ),
            );
        }
        btns = btns.child(
            gpui::div()
                .id("btn-camera")
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_size(px(12.))
                .cursor_pointer()
                .map(|el| {
                    if camera_on {
                        el.bg(rgb(GREEN)).text_color(gpui::white())
                    } else {
                        el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_MUTED))
                    }
                })
                .child("📷")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|m: &mut AppModel, _, _, cx| {
                        crate::actions::camera_toggle(m, cx);
                    }),
                ),
        );
        voice_row = voice_row.child(btns);
        panel = panel.child(voice_row);
    }

    panel.child(
        gpui::div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_2()
            .child(avatar_or_initial(
                &me.as_ref().map(|u| u.id.clone()).unwrap_or_default(),
                avatar.as_deref(),
                &name,
                px(32.),
            ))
            .child(
                gpui::div()
                    .flex()
                    .flex_col()
                    .child(gpui::div().text_size(px(13.)).child(name))
                    .child(
                        gpui::div()
                            .text_size(px(11.))
                            .text_color(rgb(TEXT_FADED))
                            .child(if in_voice {
                                "в голосовом канале"
                            } else {
                                "онлайн"
                            }),
                    ),
            )
            .child(
                gpui::div()
                    .flex()
                    .ml_auto()
                    .gap_1()
                    .child(
                        gpui::div()
                            .id("btn-mute")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .text_size(px(14.))
                            .map(|el| {
                                if self_mute {
                                    el.bg(rgb(RED)).text_color(gpui::white())
                                } else {
                                    el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_MUTED))
                                }
                            })
                            .child("🎙")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    crate::actions::voice_toggle(m, true, cx);
                                }),
                            ),
                    )
                    .child(
                        gpui::div()
                            .id("btn-deaf")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .text_size(px(14.))
                            .map(|el| {
                                if self_deaf {
                                    el.bg(rgb(RED)).text_color(gpui::white())
                                } else {
                                    el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_MUTED))
                                }
                            })
                            .child("🎧")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    crate::actions::voice_toggle(m, false, cx);
                                }),
                            ),
                    )
                    .child(
                        gpui::div()
                            .id("btn-settings")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .text_size(px(14.))
                            .bg(rgb(BG_INPUT))
                            .text_color(rgb(TEXT_MUTED))
                            .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
                            .child("⚙")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    if let Screen::Main(st) = &mut m.screen {
                                        st.settings_open = true;
                                    }
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(
                        gpui::div()
                            .id("btn-logout")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .text_size(px(14.))
                            .bg(rgb(BG_INPUT))
                            .text_color(rgb(TEXT_MUTED))
                            .hover(|s| s.text_color(rgb(RED)))
                            .child("⏻")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    crate::actions::logout(m, cx);
                                }),
                            ),
                    ),
            ),
    )
}

fn m_core_desired(core: &AppState, key: &str) -> (bool, bool) {
    core.voice_desired
        .lock()
        .unwrap()
        .get(key)
        .copied()
        .unwrap_or((false, false))
}

/* ------------------------------ Чат ------------------------------ */

fn chat_pane(
    model: &mut AppModel,
    cx: &mut gpui::Context<AppModel>,
    selected_channel: Option<&str>,
) -> impl IntoElement {
    let Screen::Main(m) = &mut model.screen else {
        unreachable!()
    };

    // Автоскролл к низу при новых сообщениях/смене канала.
    if m.scroll_bottom_pending {
        model.chat_scroll.scroll_to_bottom();
        m.scroll_bottom_pending = false;
    }

    // Заголовок.
    let channel_name = selected_channel
        .map(|cid| {
            let cache = model.core.cache.read().unwrap();
            // Имя канала или ЛС.
            for g in cache.guilds.values() {
                if let Some(c) = g.channels.iter().find(|c| c.id == cid) {
                    return format!("# {}", c.name.clone().unwrap_or_default());
                }
            }
            if let Some(c) = cache.dms.iter().find(|c| c.id == cid) {
                return dm_name(c);
            }
            "…".into()
        })
        .unwrap_or_else(|| "Выбери канал".into());

    let mut col = gpui::div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(380.))
        .bg(rgb(BG_CHAT));

    if let Some(banner) = m.banner.clone() {
        col = col.child(
            gpui::div()
                .px_4()
                .py_1()
                .bg(rgb(0x4a3d19))
                .text_size(px(12.))
                .text_color(rgb(0xf0b232))
                .child(banner),
        );
    }
    col = col.child(
        gpui::div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(rgb(0x3f4147))
            .text_size(px(15.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(channel_name),
    );

    // Трансляции Go Live в этом канале (сигналинг op 18/19/22):
    // свои и чужие — чипы под заголовком.
    if let Some(cid) = selected_channel {
        let self_id = model.core.self_id();
        // m (главный экран) уже заимствован выше — читаем его напрямую,
        // без повторного заимствования model.screen.
        let live: Vec<crate::state::StreamUi> = m
            .streams
            .values()
            .filter(|st| st.channel_id == cid)
            .cloned()
            .collect();
        let mut chips = gpui::div().flex().flex_col().gap_1().px_4().py_2();
        let mut any = false;
        for st in live {
            any = true;
            let mine = Some(st.user_id.as_str()) == self_id.as_deref();
            let title = if mine {
                format!("🔴 Твой стрим · {} смотрят", st.viewers)
            } else {
                format!("🔴 Стрим · {} смотрят", st.viewers)
            };
            let mut chip = gpui::div()
                .id(gpui::SharedString::from(format!(
                    "stream-{}",
                    st.stream_key
                )))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(rgb(BG_PANEL))
                .text_size(px(12.))
                .text_color(rgb(TEXT_MUTED))
                .map(|el| {
                    if st.paused {
                        el.text_color(rgb(TEXT_FADED))
                    } else {
                        el.text_color(rgb(TEXT_PRIMARY))
                    }
                })
                .child(if st.paused { "⏸".to_string() } else { title });
            if mine {
                chip = chip
                    .child(
                        gpui::div()
                            .id("stream-pause")
                            .ml_auto()
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(BLURPLE))
                            .hover(|s| s.text_color(gpui::white()))
                            .child(if st.paused {
                                "Продолжить"
                            } else {
                                "Пауза"
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    crate::actions::stream_pause_toggle(m, cx);
                                }),
                            ),
                    )
                    .child(
                        gpui::div()
                            .id("stream-stop")
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(RED))
                            .hover(|s| s.text_color(gpui::white()))
                            .child("Стоп")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    crate::actions::stop_my_stream(m, cx);
                                }),
                            ),
                    );
            }
            chips = chips.child(chip);
        }
        if any {
            col = col.child(chips);
        }
    }

    if let Some(cid) = selected_channel {
        // Лента сообщений.
        let history_len = m.messages.get(cid).map(Vec::len).unwrap_or(0);
        let show: Vec<Message> = m
            .messages
            .get(cid)
            .map(|list| list[list.len().saturating_sub(150)..].to_vec())
            .unwrap_or_default();

        let mut feed = gpui::div().flex().flex_col().flex_1().py_2();
        if history_len >= 50 {
            feed = feed.child(
                gpui::div()
                    .id("load-older")
                    .flex()
                    .justify_center()
                    .py_2()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(BLURPLE))
                    .hover(|s| s.text_color(gpui::white()))
                    .child(if m.loading_history {
                        "Загружаем…".to_string()
                    } else {
                        "Загрузить ещё ↑".to_string()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|m: &mut AppModel, _, _, cx| {
                            crate::actions::load_older(m, cx);
                        }),
                    ),
            );
        }
        if m.loading_history && history_len == 0 {
            feed = feed.child(
                gpui::div()
                    .flex()
                    .justify_center()
                    .py_6()
                    .text_size(px(13.))
                    .text_color(rgb(TEXT_MUTED))
                    .child("Загружаем историю…"),
            );
        }

        let self_id = model.core.self_id();
        // Группировка: тот же автор, < 5 минут, без ответа.
        let mut prev: Option<(String, Option<String>)> = None;
        let mut prev_day = String::new();
        for msg in &show {
            let day = time_day(&msg.timestamp);
            if day != prev_day {
                prev = None;
                prev_day = day.clone();
                feed = feed.child(
                    gpui::div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .py_2()
                        .child(gpui::div().flex_1().h(px(1.)).bg(rgb(0x3f4147)))
                        .child(
                            gpui::div()
                                .text_size(px(11.))
                                .text_color(rgb(TEXT_FADED))
                                .child(day),
                        )
                        .child(gpui::div().flex_1().h(px(1.)).bg(rgb(0x3f4147))),
                );
            }
            let grouped = prev
                .as_ref()
                .map(|(aid, ts)| {
                    *aid == msg
                        .author
                        .as_ref()
                        .map(|a| a.id.clone())
                        .unwrap_or_default()
                        && ts.is_some()
                        && msg.timestamp.is_some()
                        && msg.message_reference.is_none()
                        && same_minute_window(ts, &msg.timestamp)
                })
                .unwrap_or(false);
            let own = msg
                .author
                .as_ref()
                .map(|a| Some(a.id.as_str()) == self_id.as_deref())
                .unwrap_or(false);
            feed = feed.child(message_row(cx, msg, grouped, own));
            prev = Some((
                msg.author
                    .as_ref()
                    .map(|a| a.id.clone())
                    .unwrap_or_default(),
                msg.timestamp.clone(),
            ));
        }

        col = col.child(
            gpui::div()
                .id("chat-feed")
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&model.chat_scroll)
                .child(feed),
        );

        // «Печатает…»
        let typing: Vec<String> = m
            .typing
            .get(cid)
            .map(|users| {
                let now = std::time::Instant::now();
                users
                    .values()
                    .filter(|(_, till)| *till > now)
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();
        if !typing.is_empty() {
            let label = match typing.len() {
                1 => format!("{} печатает…", typing[0]),
                2 => format!("{} и {} печатают…", typing[0], typing[1]),
                _ => format!("{} и ещё {} печатают…", typing[0], typing.len() - 1),
            };
            col = col.child(
                gpui::div()
                    .px_4()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(rgb(TEXT_MUTED))
                    .child(label),
            );
        }

        // Композер.
        let composer = m.composer.clone();
        let reply = m.reply_to.clone();
        let edit = m.edit_target.clone();
        let uploading = m.uploading;
        let send_error = m.send_error.clone();
        let mut composer_wrap = gpui::div().flex().flex_col().px_4().pb_4().pt_1().gap_1();
        if let Some(err) = send_error {
            composer_wrap = composer_wrap.child(
                gpui::div()
                    .text_size(px(12.))
                    .text_color(rgb(RED))
                    .child(err),
            );
        }
        // Режим редактирования: баннер + подсказка (Enter = сохранить).
        if edit.is_some() {
            composer_wrap = composer_wrap.child(
                gpui::div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(YELLOW))
                    .child("✏ Редактирование сообщения")
                    .child(
                        gpui::div()
                            .text_color(rgb(TEXT_FADED))
                            .child("(Enter — сохранить)"),
                    )
                    .child(
                        gpui::div()
                            .id("edit-cancel")
                            .ml_auto()
                            .cursor_pointer()
                            .text_color(rgb(TEXT_FADED))
                            .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
                            .child("✕ Отмена")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    crate::actions::edit_cancel(m, cx);
                                }),
                            ),
                    ),
            );
        }
        if let Some(r) = reply.filter(|_| edit.is_none()) {
            composer_wrap = composer_wrap.child(
                gpui::div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(TEXT_MUTED))
                    .child("Ответ")
                    .child(
                        gpui::div()
                            .text_color(rgb(TEXT_PRIMARY))
                            .child(display_name(&r)),
                    )
                    .child(
                        gpui::div()
                            .id("reply-cancel")
                            .cursor_pointer()
                            .text_color(rgb(TEXT_FADED))
                            .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
                            .child("✕")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    if let Screen::Main(st) = &mut m.screen {
                                        st.reply_to = None;
                                    }
                                    cx.notify();
                                }),
                            ),
                    ),
            );
        }
        composer_wrap = composer_wrap
            .child(
                gpui::div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .rounded_lg()
                    .bg(rgb(BG_INPUT))
                    // «+» слева: вложения (нативный диалог → multipart) —
                    // как в официальном клиенте.
                    .child(
                        gpui::div()
                            .id("btn-upload")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(30.))
                            .rounded_full()
                            .bg(rgb(0x494b53))
                            .text_size(px(17.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(if uploading {
                                rgb(TEXT_FADED)
                            } else {
                                rgb(TEXT_PRIMARY)
                            })
                            .map(|el| {
                                if uploading {
                                    el
                                } else {
                                    el.cursor_pointer()
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                }
                            })
                            .child("＋")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    crate::actions::upload_pick(m, cx);
                                }),
                            ),
                    )
                    .child(gpui::div().flex_1().child(composer))
                    .child(
                        gpui::div()
                            .id("btn-send")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(30.))
                            .rounded_full()
                            .bg(rgb(BLURPLE))
                            .text_color(gpui::white())
                            .text_size(px(15.))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.85))
                            .child(if edit.is_some() { "✓" } else { "➤" })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|m: &mut AppModel, _, _, cx| {
                                    // В режиме правки Enter/✓ сохраняет,
                                    // typing триггерить не нужно.
                                    if matches!(
                                        &m.screen,
                                        Screen::Main(st) if st.edit_target.is_some()
                                    ) {
                                        crate::actions::edit_save(m, cx);
                                    } else {
                                        crate::actions::trigger_typing(m);
                                        crate::actions::send_message(m, cx);
                                    }
                                }),
                            ),
                    ),
            )
            // Enter = отправка/сохранение правки (Submit всплывает от поля).
            .on_action(cx.listener(|m: &mut AppModel, _: &Submit, _w, cx| {
                if matches!(&m.screen, Screen::Main(st) if st.edit_target.is_some()) {
                    crate::actions::edit_save(m, cx);
                } else {
                    crate::actions::trigger_typing(m);
                    crate::actions::send_message(m, cx);
                }
            }));
        col = col.child(composer_wrap);
    } else {
        col = col.child(
            gpui::div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(rgb(TEXT_MUTED))
                .child("Выбери канал слева"),
        );
    }

    col
}

fn same_minute_window(a: &Option<String>, b: &Option<String>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    let (Ok(a), Ok(b)) = (
        DateTime::parse_from_rfc3339(a),
        DateTime::parse_from_rfc3339(b),
    ) else {
        return false;
    };
    (b - a).num_minutes().abs() < 5
}

fn message_row(
    cx: &mut gpui::Context<AppModel>,
    msg: &Message,
    grouped: bool,
    own: bool,
) -> impl IntoElement {
    let author = display_name(msg);
    let time = time_hm(&msg.timestamp);
    let avatar = msg
        .author
        .as_ref()
        .and_then(|a| a.avatar.as_ref().map(|h| avatar_url(&a.id, h)));
    let uid = msg
        .author
        .as_ref()
        .map(|a| a.id.clone())
        .unwrap_or_default();

    let mut row = gpui::div()
        .id(gpui::SharedString::from(format!("msg-{}", msg.id)))
        .flex()
        .gap_3()
        .px_4()
        .py_0p5()
        .when(!grouped, |el| el.pt_2())
        .hover(|s| s.bg(rgb(0x2e3036)));

    // Линия ответа.
    if let Some(reply) = &msg.referenced_message {
        row = row.child(
            gpui::div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(12.))
                .text_color(rgb(TEXT_MUTED))
                .child("↰")
                .child(display_name(reply))
                .child(
                    gpui::div()
                        .text_color(rgb(TEXT_FADED))
                        .child(reply.content.chars().take(80).collect::<String>()),
                ),
        );
    }

    // Аватар/отступ.
    row = row.child(if grouped {
        gpui::div().w(px(40.)).into_any_element()
    } else {
        avatar_or_initial(&uid, avatar.as_deref(), &author, px(40.)).into_any_element()
    });

    // Тело: шапка + контент + вложения + реакции.
    let mut body = gpui::div().flex().flex_col().gap_1().min_w_0();
    if !grouped {
        body = body.child(
            gpui::div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    gpui::div()
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT_PRIMARY))
                        .child(author.clone()),
                )
                .children(msg.author.as_ref().filter(|a| a.bot).map(|_| {
                    gpui::div()
                        .px_1()
                        .rounded(px(3.))
                        .bg(rgb(BLURPLE))
                        .text_size(px(10.))
                        .text_color(gpui::white())
                        .child("BOT")
                }))
                .child(
                    gpui::div()
                        .text_size(px(11.))
                        .text_color(rgb(TEXT_FADED))
                        .child(time),
                ),
        );
    }
    if !msg.content.is_empty() {
        // Переносы строк: gpui не имеет pre-wrap — рендерим построчно.
        let lines: Vec<&str> = msg.content.split('\n').collect();
        body = body.child(
            gpui::div()
                .text_size(px(14.))
                .text_color(rgb(TEXT_PRIMARY))
                .line_height(gpui::relative(1.4))
                .flex()
                .flex_col()
                .children(
                    lines
                        .into_iter()
                        .map(|line| gpui::div().child(line.to_string())),
                ),
        );
    }
    if msg.edited_timestamp.is_some() {
        body = body.child(
            gpui::div()
                .text_size(px(11.))
                .text_color(rgb(TEXT_FADED))
                .child("(изменено)"),
        );
    }
    // Вложения-картинки.
    for att in &msg.attachments {
        let is_image = att
            .content_type
            .as_deref()
            .map(|t| t.starts_with("image/"))
            .unwrap_or(false);
        if is_image {
            body = body.child(gpui::img(att.url.clone()).max_w(px(360.)).rounded_md());
        } else {
            body = body.child(
                gpui::div()
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(BG_PANEL))
                    .text_size(px(13.))
                    .text_color(rgb(TEXT_PRIMARY))
                    .child("📎")
                    .child(att.filename.clone())
                    .child(
                        gpui::div()
                            .text_size(px(11.))
                            .text_color(rgb(TEXT_FADED))
                            .child(format!("{} КБ", att.size / 1024)),
                    ),
            );
        }
    }
    // Реакции.
    if let Some(reactions) = &msg.reactions {
        if !reactions.is_empty() {
            let mut chips = gpui::div().flex().flex_wrap().gap_1();
            for r in reactions {
                let emoji = r.emoji.name.clone().unwrap_or_else(|| "❔".into());
                let (mid, emoji_name, mine) = (msg.id.clone(), emoji.clone(), r.me);
                chips = chips.child(
                    gpui::div()
                        .id(gpui::SharedString::from(format!(
                            "react-{mid}-{emoji_name}"
                        )))
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .text_size(px(12.))
                        .cursor_pointer()
                        .map(|el| {
                            if mine {
                                el.bg(rgb(0x3c45a5)).text_color(gpui::white())
                            } else {
                                el.bg(rgb(BG_INPUT)).text_color(rgb(TEXT_MUTED))
                            }
                        })
                        .child(emoji_name.clone())
                        .child(r.count.to_string())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |m: &mut AppModel, _, _, cx| {
                                crate::actions::toggle_reaction(
                                    m,
                                    &mid.clone(),
                                    &emoji_name.clone(),
                                    cx,
                                );
                            }),
                        ),
                );
            }
            body = body.child(chips);
        }
    }
    // Действия строки: ответ всем, своим — правка и удаление (как в
    // официале, только всегда видимы — мелкие и приглушённые).
    let mut actions = gpui::div().flex().items_center().gap_1();
    {
        let reply_msg = msg.clone();
        actions = actions.child(
            gpui::div()
                .id(gpui::SharedString::from(format!("act-reply-{}", msg.id)))
                .px_1()
                .rounded_md()
                .text_size(px(12.))
                .text_color(rgb(TEXT_FADED))
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
                .child("↩")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |m: &mut AppModel, _, _, cx| {
                        if let Screen::Main(st) = &mut m.screen {
                            st.reply_to = Some(reply_msg.clone());
                            st.edit_target = None;
                        }
                        cx.notify();
                    }),
                ),
        );
        if own {
            let edit_id = msg.id.clone();
            actions = actions.child(
                gpui::div()
                    .id(gpui::SharedString::from(format!("act-edit-{}", msg.id)))
                    .px_1()
                    .rounded_md()
                    .text_size(px(12.))
                    .text_color(rgb(TEXT_FADED))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
                    .child("✏")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |m: &mut AppModel, _, _, cx| {
                            crate::actions::edit_start(m, &edit_id.clone(), cx);
                        }),
                    ),
            );
            let del_id = msg.id.clone();
            actions = actions.child(
                gpui::div()
                    .id(gpui::SharedString::from(format!("act-del-{}", msg.id)))
                    .px_1()
                    .rounded_md()
                    .text_size(px(12.))
                    .text_color(rgb(TEXT_FADED))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(RED)))
                    .child("🗑")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |m: &mut AppModel, _, _, cx| {
                            crate::actions::delete_message(m, &del_id.clone(), cx);
                        }),
                    ),
            );
        }
    }
    row.child(body).child(actions)
}

/* --------------------------- Участники --------------------------- */

fn member_pane(
    model: &mut AppModel,
    cx: &mut gpui::Context<AppModel>,
    selected_guild: Option<&str>,
    _selected_channel: Option<&str>,
) -> impl IntoElement {
    let rows: Vec<MemberListRow> = match (selected_guild, &model.screen) {
        (Some(gid), Screen::Main(_)) => {
            let lists = model.core.member_lists.lock().unwrap();
            lists.get(gid).map(|l| l.rows.clone()).unwrap_or_default()
        }
        _ => Vec::new(),
    };
    let is_bot = model.core.is_bot.load(std::sync::atomic::Ordering::SeqCst);
    let self_id = model.core.self_id();
    let confirm_mod = match &model.screen {
        Screen::Main(m) => m.confirm_mod.clone(),
        _ => None,
    };
    let total = match selected_guild {
        Some(gid) => model
            .core
            .cache
            .read()
            .unwrap()
            .guilds
            .get(gid)
            .map(|g| g.member_count)
            .unwrap_or(0),
        None => 0,
    };

    let mut col = gpui::div().flex().flex_col().px_2().py_2();
    if !rows.is_empty() {
        for row in rows {
            col = col.child(match row {
                MemberListRow::Group { id, count } => gpui::div()
                    .id(gpui::SharedString::from(format!("grp-{id}")))
                    .flex()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_FADED))
                    .child(uppercase_group(&id))
                    .child(
                        gpui::div()
                            .ml_auto()
                            .text_color(rgb(TEXT_FADED))
                            .child(count.to_string()),
                    ),
                MemberListRow::Member {
                    user_id,
                    username,
                    global_name,
                    nick,
                    avatar,
                    status,
                    activities,
                    ..
                } => {
                    let name = nick.or(global_name).unwrap_or(username);
                    let activity = activities
                        .first()
                        .and_then(|a| a.get("name").and_then(|v| v.as_str()))
                        .map(String::from);
                    let speaking = match &model.screen {
                        Screen::Main(m) => m.speaking.contains(&user_id),
                        _ => false,
                    };
                    let av_url = avatar
                        .as_deref()
                        .filter(|h| !h.is_empty())
                        .map(|h| avatar_url(&user_id, h));
                    gpui::div()
                        .id(gpui::SharedString::from(format!("mem-{user_id}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_color(rgb(TEXT_MUTED))
                        .hover(|s| s.bg(rgb(BG_HOVER)).text_color(rgb(TEXT_PRIMARY)))
                        .when(speaking, |el| el.text_color(rgb(GREEN)))
                        // Аватар с точкой статуса (как в официале).
                        .child(
                            gpui::div()
                                .relative()
                                .size(px(32.))
                                .child(avatar_or_initial(&user_id, av_url.as_deref(), &name, px(32.)))
                                .child(
                                    gpui::div()
                                        .absolute()
                                        .bottom_0()
                                        .right_0()
                                        .size(px(14.))
                                        .rounded_full()
                                        .bg(rgb(BG_PANEL))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            gpui::div()
                                                .size(px(10.))
                                                .rounded_full()
                                                .bg(status_color(&status)),
                                        ),
                                ),
                        )
                        .child(
                            gpui::div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(
                                    gpui::div()
                                        .text_size(px(14.))
                                        .text_color(rgb(TEXT_PRIMARY))
                                        .overflow_hidden()
                                        .child(name),
                                )
                                .children(activity.map(|a| {
                                    gpui::div()
                                        .text_size(px(11.))
                                        .text_color(rgb(TEXT_FADED))
                                        .overflow_hidden()
                                        .child(a)
                                })),
                        )
                        // Действия (только юзер-сессии и не по себе):
                        // ЛС, кик, бан — кик/бан с подтверждением вторым кликом.
                        .when(
                            !is_bot && Some(user_id.as_str()) != self_id.as_deref(),
                            |el| {
                                el.child(member_actions(cx, &user_id, confirm_mod.as_ref()))
                            },
                        )
                }
            });
        }
    } else if is_bot && total > 0 {
        // Ботам список приходит чанками op 8 — покажем счётчик.
        col = col.child(
            gpui::div()
                .px_2()
                .py_2()
                .text_size(px(12.))
                .text_color(rgb(TEXT_FADED))
                .child(format!("Участников: {total}")),
        );
    } else {
        col = col.child(
            gpui::div()
                .px_2()
                .py_2()
                .text_size(px(12.))
                .text_color(rgb(TEXT_FADED))
                .child("Список участников подписывается при выборе канала"),
        );
    }

    let visible = selected_guild.is_some();
    gpui::div().when(visible, |el| {
        el.w(px(240.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(BG_PANEL))
            .border_l_1()
            .border_color(rgb(0x3f4147))
            .child(
                gpui::div()
                    .id("members-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(col),
            )
    })
}

fn uppercase_group(id: &str) -> String {
    match id {
        "online" => "В СЕТИ".into(),
        "offline" => "ОФФЛАЙН".into(),
        "idle" => "НЕ АКТИВЕН".into(),
        "dnd" => "НЕ БЕСПОКОИТЬ".into(),
        other => other.to_uppercase(),
    }
}

/// Кнопки строки участника: ЛС + кик/бан (второй клик подтверждает).
fn member_actions(
    cx: &mut gpui::Context<AppModel>,
    user_id: &str,
    confirm_mod: Option<&(String, crate::state::ModAction)>,
) -> impl IntoElement {
    let mut acts = gpui::div().flex().items_center().gap_1().ml_auto();
    let dm_uid = user_id.to_string();
    acts = acts.child(
        gpui::div()
            .id(gpui::SharedString::from(format!("mem-dm-{user_id}")))
            .px_1()
            .rounded_md()
            .text_size(px(11.))
            .text_color(rgb(TEXT_FADED))
            .cursor_pointer()
            .hover(|s| s.text_color(rgb(TEXT_PRIMARY)))
            .child("✉")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |m: &mut AppModel, _, _, cx| {
                    crate::actions::create_dm(m, &dm_uid.clone(), cx);
                }),
            ),
    );
    for (act, icon, idpart) in [
        (crate::state::ModAction::Kick, "⏏", "kick"),
        (crate::state::ModAction::Ban, "⛔", "ban"),
    ] {
        let armed = confirm_mod == Some(&(user_id.to_string(), act));
        let uid = user_id.to_string();
        acts = acts.child(
            gpui::div()
                .id(gpui::SharedString::from(format!("mem-{idpart}-{user_id}")))
                .px_1()
                .rounded_md()
                .text_size(px(11.))
                .cursor_pointer()
                .map(|el| {
                    if armed {
                        el.bg(rgb(RED)).text_color(gpui::white()).child("Точно?")
                    } else {
                        el.text_color(rgb(TEXT_FADED))
                            .hover(|s| s.text_color(rgb(RED)))
                            .child(icon)
                    }
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |m: &mut AppModel, _, _, cx| {
                        crate::actions::member_action(m, &uid.clone(), act, cx);
                    }),
                ),
        );
    }
    acts
}

fn first_letter(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}

/* --------------------------- Мелочи --------------------------- */

fn status_label(status: &str) -> &'static str {
    match status {
        "online" => "в сети",
        "idle" => "не активен",
        "dnd" => "не беспокоить",
        _ => "оффлайн",
    }
}

/// Аватарка (картинка) или цветной круг с инициалом.
pub(crate) fn avatar_or_initial(
    user_id: &str,
    avatar_url: Option<&str>,
    name: &str,
    size: gpui::Pixels,
) -> impl IntoElement {
    match avatar_url {
        Some(url) => gpui::div()
            .flex()
            .items_center()
            .justify_center()
            .size(size)
            .overflow_hidden()
            .rounded_full()
            .child(gpui::img(url.to_string()).size(size))
            .into_any_element(),
        None => gpui::div()
            .flex()
            .items_center()
            .justify_center()
            .size(size)
            .rounded_full()
            .bg(avatar_color(user_id))
            .text_color(gpui::white())
            .text_size(size * 0.42)
            .child(first_letter(name))
            .into_any_element(),
    }
}
