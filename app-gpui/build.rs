//! Вшить иконку приложения в .exe на Windows (ресурс с id=1): её подхва-
//! тывают и проводник/панель задач, и gpui — окна создаются с классом
//! `Zed::Window`, где hIcon = LoadImageW(module, 1).
//! На других платформах иконка живёт в пакете (icns/PNG/desktop).
//! assets/icon.ico сгенерирован scripts/make_icon_assets.py
//! («краб, кусающий Вампуса» — см. packaging/icons).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Err(e) = winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
        {
            println!("cargo:warning=winresource: {e}");
        }
    }
}

