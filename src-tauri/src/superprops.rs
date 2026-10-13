//! X-Super-Properties — «паспорт клиента», который браузерный Discord шлёт
//! с каждым запросом. Пользовательские токены без этого заголовка (и без
//! браузерного User-Agent) часто получают 401 на write-эндпоинтах, поэтому
//! реверс-инжиниринг here: собираем тот же base64(JSON), что и оригинал.
//!
//! `client_build_number` меняется с каждым релизом Discord. Мы тянем его
//! прямо с discord.com (страница /login подключает JS-бандлы, в которых
//! зашит номер сборки), с фолбэком на константу.

use base64::Engine;
use serde_json::json;

/// Дефолт на случай, если discord.com недоступен. Обновляется автоматически
/// при каждом `connect` — здесь просто правдоподобное значение.
pub const FALLBACK_BUILD_NUMBER: u64 = 4173;

/// Браузерная личина: Chrome на Windows — самый распространённый профиль
/// пользователя Discord, наименьшие шансы попасть под эвристики.
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// base64(X-Super-Properties JSON) с конкретным build number.
pub fn super_properties(build_number: u64) -> String {
    let props = json!({
        "os": "Windows",
        "browser": "Chrome",
        "device": "",
        "system_locale": "ru-RU",
        "browser_user_agent": BROWSER_UA,
        "browser_version": "131.0.0.0",
        "os_version": "10",
        "referrer": "",
        "referring_domain": "",
        "referrer_current": "",
        "referring_domain_current": "",
        "release_channel": "stable",
        "client_build_number": build_number,
        "client_event_source": null,
        "design_id": 0
    });
    b64(props.to_string().as_bytes())
}

/// base64(X-Context-Properties) для POST /auth/login — браузер помечает,
/// *откуда* выполнен вход ({location: "Login"}).
pub fn context_properties_login() -> String {
    b64(json!({ "location": "Login" }).to_string().as_bytes())
}

/// Вытащить актуальный client_build_number с discord.com.
///
/// `/login` отдаёт HTML со ссылками на `/assets/*.js`; в одном из первых
/// бандлов есть `"build_number":12345`. Тянем максимум 6 бандлов, берём
/// первое совпадение. Любая ошибка → None (используем фолбэк/кэш).
pub async fn fetch_build_number() -> Option<u64> {
    let http = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;

    let html = http
        .get("https://discord.com/login")
        .header("Accept-Language", "ru-RU,ru;q=0.9")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;

    // <script src="/assets/....js" — берём до 6 первых бандлов.
    let mut assets: Vec<String> = Vec::new();
    let mut rest = html.as_str();
    while let Some(pos) = rest.find("src=\"/assets/") {
        rest = &rest[pos + 5..];
        if let Some(end) = rest.find('"') {
            assets.push(format!("https://discord.com{}", &rest[..end]));
            if assets.len() >= 6 {
                break;
            }
        } else {
            break;
        }
    }

    for url in assets {
        let Ok(js) = http.get(&url).send().await.ok()?.text().await else {
            continue;
        };
        // "build_number":12345  или  build_number:"12345"
        if let Some(idx) = js.find("build_number") {
            let tail: String = js[idx..].chars().take(48).collect();
            let digits: String = tail
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<u64>() {
                if n > 1000 {
                    tracing::info!("superprops: client_build_number = {n} (из {url})");
                    return Some(n);
                }
            }
        }
    }
    None
}
