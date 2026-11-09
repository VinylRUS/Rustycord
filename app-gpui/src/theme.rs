//! Палитра Discord Dark + мелкие визуальные константы.

use gpui::rgb;
use gpui::Hsla;

/// Самый тёмный уровень (рейл серверов).
pub const BG_BASE: u32 = 0x1e1f22;
/// Панели (список каналов, участники).
pub const BG_PANEL: u32 = 0x2b2d31;
/// Фон чата.
pub const BG_CHAT: u32 = 0x313338;
/// Поля ввода.
pub const BG_INPUT: u32 = 0x383a40;
/// Ховер строк.
pub const BG_HOVER: u32 = 0x35373c;
/// Активная строка.
pub const BG_ACTIVE: u32 = 0x404249;

pub const TEXT_PRIMARY: u32 = 0xdbdee1;
pub const TEXT_MUTED: u32 = 0x949ba4;
pub const TEXT_FADED: u32 = 0x80848e;

pub const BLURPLE: u32 = 0x5865f2;
pub const GREEN: u32 = 0x23a55a;
pub const YELLOW: u32 = 0xf0b232;
pub const RED: u32 = 0xf23f43;

/// Цвет кружка-аватарки по id пользователя (6 фирменных цветов Discord).
pub fn avatar_color(user_id: &str) -> Hsla {
    const PALETTE: [u32; 6] = [
        0x3498db, // синий
        0x2ecc71, // зелёный
        0x911eb4, // фиолетовый
        0xe67e22, // оранжевый
        0xe74c3c, // красный
        0x1abc9c, // бирюзовый
    ];
    let n = user_id
        .chars()
        .filter_map(|c| c.to_digit(10))
        .fold(0u32, |acc, d| acc.wrapping_mul(10).wrapping_add(d));
    rgb(PALETTE[(n % 6) as usize]).into()
}

/// Цвет статуса присенса.
pub fn status_color(status: &str) -> Hsla {
    match status {
        "online" => rgb(GREEN).into(),
        "idle" => rgb(YELLOW).into(),
        "dnd" => rgb(RED).into(),
        _ => rgb(TEXT_MUTED).into(),
    }
}

/// Основной шрифт интерфейса по платформе (кириллица обязательна).
pub fn ui_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Segoe UI"
    } else if cfg!(target_os = "macos") {
        "Helvetica Neue"
    } else {
        "Noto Sans"
    }
}
