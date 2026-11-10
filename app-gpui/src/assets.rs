//! Встроенные ассеты: логотип-краб (кусает Вампуса).
//!
//! Один PNG 512×512 зашит в бинарник (include_bytes) и работает в трёх
//! ролях: картинка GPUI (`img()` сам декодирует через image-фронтенд gpui),
//! домашняя кнопка рейла, логотип экрана логина и иконка трея
//! (декод на старте через image + resize к 64×64).

use std::sync::OnceLock;

/// PNG-байты логотипа (512×512, RGBA).
const LOGO_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Картинка для `img()`: `img(assets::logo_image())`.
pub fn logo_image() -> std::sync::Arc<gpui::Image> {
    static LOGO: OnceLock<std::sync::Arc<gpui::Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, LOGO_PNG.to_vec()))
    })
    .clone()
}

/// RGBA 64×64 для tray-icon::Icon::from_rgba (квадратный краб).
pub fn tray_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(LOGO_PNG).ok()?.thumbnail(64, 64);
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((rgba.into_raw(), w, h))
}
