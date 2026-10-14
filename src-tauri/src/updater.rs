//! Проверка обновлений: смотрим последний релиз на GitHub и сравниваем
//! с текущей версией приложения. Вызывается с фронта при старте;
//! никаких авто-загрузок — только уведомление со ссылкой на релиз
//! (решение «скачать» всегда за пользователем).

use serde::Deserialize;
use serde_json::Value;

pub const RELEASES_API: &str = "https://api.github.com/repos/VinylRUS/Rustycord/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/VinylRUS/Rustycord/releases/latest";

#[derive(Debug, Deserialize)]
struct GithubRelease {
    /// Может отсутствовать у draft-релизов — обрабатываем как «нет обновлений».
    tag_name: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
}

/// Ответ для фронтенда.
#[derive(Debug, serde::Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub is_newer: bool,
    pub url: String,
    pub notes: String,
    pub published_at: String,
    /// 404 от API — ни одного публичного релиза нет (например, dev-сборка).
    pub no_releases: bool,
}

/// Сравнение версий «1.2.10» > «1.2.9»: численно по компонентам,
/// префикс «v» игнорируется, нечисловые хвосты (0.6.0-beta) сравниваются
/// как «меньше релиза».
pub fn version_is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<i64> {
        s.trim_start_matches(['v', 'V'])
            .split('.')
            .map(|part| {
                let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<i64>().unwrap_or(0)
            })
            .collect()
    };
    let a = parse(latest);
    let b = parse(current);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// Тянем последний публичный релиз. Ошибки сети не критичны —
/// возвращаем их как `Err(String)`, фронтенд молча проглотит.
pub async fn check(current: &str) -> Result<UpdateInfo, String> {
    let ua = format!("RustyCord/{current} (github.com/VinylRUS/Rustycord)");
    let http = reqwest::Client::builder()
        .user_agent(&ua)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = http
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().as_u16() == 404 {
        // Публичных релизов ещё нет — ничего не показываем.
        return Ok(UpdateInfo {
            current: current.to_string(),
            latest: current.to_string(),
            is_newer: false,
            url: RELEASES_PAGE.to_string(),
            notes: String::new(),
            published_at: String::new(),
            no_releases: true,
        });
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub API: HTTP {}", resp.status()));
    }

    let rel: GithubRelease = resp.json().await.map_err(|e| e.to_string())?;
    let tag = rel.tag_name.unwrap_or_default();
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    Ok(UpdateInfo {
        current: current.to_string(),
        is_newer: !latest.is_empty() && version_is_newer(&latest, current),
        url: rel.html_url.unwrap_or_else(|| RELEASES_PAGE.to_string()),
        // Заметки урезаем до компактного превью — полный changelog на странице релиза.
        notes: rel
            .body
            .as_deref()
            .map(|b| notes_preview(b, 400))
            .unwrap_or_default(),
        published_at: rel.published_at.unwrap_or_default(),
        no_releases: false,
        latest,
    })
}

/// Урезаем markdown-заметки релиза до первых строк без тяжёлого рендера.
pub fn notes_preview(notes: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for line in notes.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        // Пропускаем заголовки-разделители и ссылки-артефакты changelog'а.
        if l.starts_with("---") || l.starts_with("https://") || l.starts_with("[!") {
            continue;
        }
        if out.len() + l.len() > max_chars {
            break;
        }
        out.push_str(l.trim_start_matches("# "));
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Заглушка-компат: Value для команды Tauri (серилизуем UpdateInfo напрямую).
pub fn to_value(info: &UpdateInfo) -> Value {
    serde_json::to_value(info).unwrap_or(Value::Null)
}
