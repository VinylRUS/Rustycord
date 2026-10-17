//! Минимальный декодер `PreloadedUserSettings` (protobuf) — ровно столько,
//! сколько нужно для раскладки рейла серверов. Современный Discord хранит
//! пользовательские настройки в protobuf (`user_settings_proto` в READY,
//! base64), а не в legacy-JSON `guild_folders`. Схема свёрена с пакетом
//! discord-protos (discord-userdoccers, PreloadedUserSettings.proto):
//!
//! ```text
//! PreloadedUserSettings
//!   field 14 (LD) → GuildFolders
//!     field 1 (LD, repeat) → GuildFolder
//!       field 1 (fixed64, repeat) guild_ids  — 8 сырых байт LE, packed ИЛИ
//!                                            поштучно (wire 1 / packed wire 2)
//!       field 2 (LD)  id    : google.protobuf.Int64Value  {1: varint}
//!       field 3 (LD)  name  : google.protobuf.StringValue {1: bytes}
//!       field 4 (LD)  color : google.protobuf.UInt64Value {1: varint}
//!     field 2 (fixed64, repeat) guild_positions — порядок отдельных
//!                                             (не в папках) гильдий рейла
//! ```
//!
//! ВАЖНО (урок v0.9.4): guild_ids/guild_positions — fixed64, а НЕ varint.
//! Чтение их как varint давало мусорные id: папки полнились «призраками»,
//! а реальные гильдии выпадали из раскладки в конец рейла. Поштучный
//! fixed64 — wire-тип 1; packed — чанк из групп по 8 байт little-endian.
//!
//! Декодер намеренно ручной: один файл, без prost-генерации, unknown-поля
//! пропускаются, битый payload → None (сессия не должна страдать от кривых
//! настроек).

use base64::Engine as _;

/// Одна папка рейла: имя/цвет опциональны (безымянная папка — просто группа).
#[derive(Debug, Clone, PartialEq)]
pub struct GuildFolderInfo {
    pub id: Option<u64>,
    pub name: Option<String>,
    pub color: Option<i32>,
    pub guild_ids: Vec<String>,
}

/// Раскладка рейла: папки + порядок «отдельных» гильдий.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuildLayout {
    pub folders: Vec<GuildFolderInfo>,
    pub guild_positions: Vec<String>,
}

/* --------------------------- wire reader --------------------------- */

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        for shift in (0..64).step_by(7) {
            let byte = *self.buf.get(self.pos)?;
            self.pos += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
        }
        None // >10 байт — не валидный varint
    }

    /// Тег поля → (номер поля, wire-тип). None = конец/битый буфер.
    fn tag(&mut self) -> Option<(u32, u8)> {
        let v = self.varint()?;
        Some(((v >> 3) as u32, (v & 0x7) as u8))
    }

    /// Length-delimited срез (wire type 2).
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = self.varint()? as usize;
        let end = self.pos.checked_add(len)?;
        if end > self.buf.len() {
            return None;
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Some(slice)
    }

    /// fixed64 (wire type 1): 8 сырых байт little-endian.
    fn fixed64(&mut self) -> Option<u64> {
        let end = self.pos.checked_add(8)?;
        if end > self.buf.len() {
            return None;
        }
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.buf[self.pos..end]);
        self.pos = end;
        Some(u64::from_le_bytes(b))
    }

    /// Пропуск неизвестного поля по wire-типу.
    fn skip(&mut self, wire: u8) -> Option<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => self.pos = self.pos.checked_add(8)?,
            2 => {
                self.bytes()?;
            }
            5 => self.pos = self.pos.checked_add(4)?,
            _ => return None,
        }
        if self.pos > self.buf.len() {
            return None;
        }
        Some(())
    }
}

/// Повторяющийся fixed64 (снежинки Discord): packed-чанк из групп по 8 байт
/// LE (wire 2) или поштучно (wire 1). Всё, что не бьётся на группы по 8,
/// считаем мусором → None (декодер не должен выдумывать id).
fn read_repeated_fixed64(chunk: &[u8], out: &mut Vec<u64>) -> Option<()> {
    if chunk.len() % 8 != 0 {
        return None;
    }
    for g in chunk.chunks_exact(8) {
        let mut b = [0u8; 8];
        b.copy_from_slice(g);
        out.push(u64::from_le_bytes(b));
    }
    Some(())
}

/// Обёртка google.protobuf.UInt64Value / Int32Value: {1: varint}.
fn wrapped_varint(buf: &[u8]) -> Option<u64> {
    let mut r = Reader::new(buf);
    while let Some((no, wire)) = r.tag() {
        match (no, wire) {
            (1, 0) => return r.varint(),
            _ => r.skip(wire)?,
        }
    }
    None
}

/// Обёртка google.protobuf.StringValue: {1: bytes}.
fn wrapped_string(buf: &[u8]) -> Option<String> {
    let mut r = Reader::new(buf);
    while let Some((no, wire)) = r.tag() {
        match (no, wire) {
            (1, 2) => return String::from_utf8(r.bytes()?.to_vec()).ok(),
            _ => r.skip(wire)?,
        }
    }
    None
}

fn parse_guild_folder(buf: &[u8]) -> Option<GuildFolderInfo> {
    let mut info = GuildFolderInfo {
        id: None,
        name: None,
        color: None,
        guild_ids: Vec::new(),
    };
    let mut r = Reader::new(buf);
    while let Some((no, wire)) = r.tag() {
        match (no, wire) {
            (1, 1) => {
                // Поштучный guild_id — fixed64 (8 байт LE).
                info.guild_ids.push(r.fixed64()?.to_string());
            }
            (1, 2) => {
                // Packed-набор guild_ids — чанк из групп по 8 байт LE.
                let chunk = r.bytes()?;
                let mut ids = Vec::new();
                read_repeated_fixed64(chunk, &mut ids)?;
                info.guild_ids.extend(ids.iter().map(|v| v.to_string()));
            }
            (2, 2) => info.id = wrapped_varint(r.bytes()?),
            (3, 2) => info.name = wrapped_string(r.bytes()?),
            (4, 2) => info.color = wrapped_varint(r.bytes()?).map(|v| v as i32),
            _ => r.skip(wire)?,
        }
    }
    // Дубликаты теоретически возможны (многолетние инкрементальные записи
    // настроек), а дубликат в keyed-each Svelte роняет весь рендер рейла —
    // режем здесь, на входе в UI.
    info.guild_ids = dedup_keep_order(info.guild_ids);
    Some(info)
}

/// Дедупликация с сохранением порядка первого вхождения.
fn dedup_keep_order(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|x| seen.insert(x.clone()))
        .collect()
}

fn parse_guild_folders(buf: &[u8]) -> Option<GuildLayout> {
    let mut layout = GuildLayout::default();
    let mut r = Reader::new(buf);
    while let Some((no, wire)) = r.tag() {
        match (no, wire) {
            (1, 2) => layout.folders.push(parse_guild_folder(r.bytes()?)?),
            (2, 1) => {
                // Поштучная позиция — fixed64 (8 байт LE).
                if let Some(v) = r.fixed64() {
                    layout.guild_positions.push(v.to_string());
                }
            }
            (2, 2) => {
                // Packed-набор позиций — чанк из групп по 8 байт LE.
                let chunk = r.bytes()?;
                let mut ids = Vec::new();
                read_repeated_fixed64(chunk, &mut ids)?;
                layout
                    .guild_positions
                    .extend(ids.iter().map(|v| v.to_string()));
            }
            _ => r.skip(wire)?,
        }
    }
    layout.guild_positions = dedup_keep_order(layout.guild_positions);
    Some(layout)
}

/// Точка входа: base64 `user_settings_proto` → раскладка рейла.
/// None = не прото/нет папок/битые данные.
pub fn decode_layout(b64: &str) -> Option<GuildLayout> {
    let trimmed = b64.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .ok()?;
    if bytes.is_empty() {
        return None;
    }
    let mut r = Reader::new(&bytes);
    while let Some((no, wire)) = r.tag() {
        if (no, wire) == (14, 2) {
            return parse_guild_folders(r.bytes()?);
        }
        r.skip(wire)?;
    }
    None
}

/* ------------------------------ tests ------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;

    /// Крошечный protobuf-писатель для тестов.
    struct W(Vec<u8>);
    impl W {
        fn new() -> Self {
            Self(Vec::new())
        }
        fn varint(&mut self, mut v: u64) {
            loop {
                let byte = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    self.0.push(byte);
                    break;
                }
                self.0.push(byte | 0x80);
            }
        }
        fn tag(&mut self, no: u32, wire: u8) {
            self.varint((u64::from(no) << 3) | u64::from(wire));
        }
        fn ld(&mut self, no: u32, inner: &[u8]) {
            self.tag(no, 2);
            self.varint(inner.len() as u64);
            self.0.extend_from_slice(inner);
        }
        fn vint(&mut self, no: u32, v: u64) {
            self.tag(no, 0);
            self.varint(v);
        }
        /// Поштучный fixed64 (wire 1) — как шлёт сериализатор для
        /// unpacked repeated fixed64.
        fn fixed(&mut self, no: u32, v: u64) {
            self.tag(no, 1);
            self.0.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn decodes_folders_and_positions() {
        // Снежинки масштаба реальных id Discord (~10^18) — round-trip
        // через fixed64 обязан давать те же десятичные строки.
        let gid_a: u64 = 1_302_345_678_901_234_567;
        let gid_b: u64 = 1_309_876_543_210_987_654;
        let pos_a: u64 = 1_295_111_222_333_444_555;
        let pos_b: u64 = 1_311_999_888_777_666_555;

        // GuildFolder { guild_ids: [gid_a, gid_b] (packed fixed64),
        //              name: "Игры", color: 0x5865F2, id: 7 }
        let mut folder = W::new();
        let mut ids = W::new();
        ids.0.extend_from_slice(&gid_a.to_le_bytes());
        ids.0.extend_from_slice(&gid_b.to_le_bytes());
        folder.ld(1, &ids.0);
        let mut name = W::new();
        name.ld(1, b"\xd0\x98\xd0\xb3\xd1\x80\xd1\x8b"); // "Игры" utf-8
        folder.ld(3, &name.0);
        let mut color = W::new();
        color.vint(1, 5_786_610); // 0x5865F2
        folder.ld(4, &color.0);
        let mut fid = W::new();
        fid.vint(1, 7);
        folder.ld(2, &fid.0);

        // GuildFolders { folders: [folder], guild_positions: [pos_a, pos_b]
        // (packed fixed64) }
        let mut folders = W::new();
        folders.ld(1, &folder.0);
        let mut pos = W::new();
        pos.0.extend_from_slice(&pos_a.to_le_bytes());
        pos.0.extend_from_slice(&pos_b.to_le_bytes());
        folders.ld(2, &pos.0);

        // PreloadedUserSettings { guild_folders: folders (field 14) }
        let mut settings = W::new();
        settings.ld(14, &folders.0);
        // мусорное поле до целевого (проверяем skip)
        settings.ld(2, b"junk");

        let layout = decode_layout(&b64(&settings.0)).expect("layout");
        assert_eq!(layout.folders.len(), 1);
        assert_eq!(
            layout.folders[0].guild_ids,
            vec![gid_a.to_string(), gid_b.to_string()]
        );
        assert_eq!(layout.folders[0].name.as_deref(), Some("Игры"));
        assert_eq!(layout.folders[0].color, Some(5_786_610));
        assert_eq!(layout.folders[0].id, Some(7));
        assert_eq!(
            layout.guild_positions,
            vec![pos_a.to_string(), pos_b.to_string()]
        );
    }

    #[test]
    fn unpacked_fixed64_mixed_with_packed() {
        // Реальный сериализатор может чередовать packed-чанк и поштучные
        // fixed64 (прото это разрешает) — собираем всё в порядке следования.
        let g1: u64 = 111;
        let g2: u64 = 1_302_000_111_222_333_444;
        let g3: u64 = 333;
        let mut folder = W::new();
        let mut packed = W::new();
        packed.0.extend_from_slice(&g1.to_le_bytes());
        packed.0.extend_from_slice(&g2.to_le_bytes());
        folder.ld(1, &packed.0); // packed [g1, g2]
        folder.fixed(1, g2); // поштучный дубль
        folder.fixed(1, g3); // поштучный новый
        let mut folders = W::new();
        folders.ld(1, &folder.0);
        folders.fixed(2, 555); // поштучная позиция
        folders.fixed(2, 555); // дубль позиции
        let mut settings = W::new();
        settings.ld(14, &folders.0);

        let layout = decode_layout(&b64(&settings.0)).expect("layout");
        assert_eq!(
            layout.folders[0].guild_ids,
            vec![g1.to_string(), g2.to_string(), g3.to_string()]
        );
        assert_eq!(layout.guild_positions, vec!["555"]);
    }

    #[test]
    fn varint_guild_ids_do_not_become_garbage() {
        // РЕГРЕССИЯ v0.9.4: varint-кодированные значения в fixed64-полях —
        // неверный wire-тип. Раньше читались как id (мусор → папки-призраки),
        // теперь молча пропускаются как unknown: раскладка декодируется,
        // но мусорных id не рождает.
        let mut folder = W::new();
        folder.vint(1, 111); // varint в fixed64-поле — игнор
        folder.vint(1, 222);
        let mut folders = W::new();
        folders.ld(1, &folder.0);
        folders.vint(2, 999); // varint-позиция — игнор
        let mut settings = W::new();
        settings.ld(14, &folders.0);

        let layout = decode_layout(&b64(&settings.0)).expect("layout");
        assert_eq!(layout.folders.len(), 1);
        assert!(layout.folders[0].guild_ids.is_empty());
        assert!(layout.guild_positions.is_empty());
    }

    #[test]
    fn packed_chunk_not_multiple_of_8_is_none() {
        // Packed-чанк фиксированной длины обязан биться на группы по 8 байт;
        // обрывок — признак битого блоба/схемного дрейфа → весь decode
        // возвращаем как «нет раскладки» (рейл покажет простой список).
        let mut folder = W::new();
        let mut bad_chunk = W::new();
        bad_chunk.varint(111);
        bad_chunk.varint(222); // 3 байта — не бьётся на группы по 8
        folder.ld(1, &bad_chunk.0);
        let mut folders = W::new();
        folders.ld(1, &folder.0);
        let mut settings = W::new();
        settings.ld(14, &folders.0);

        assert!(decode_layout(&b64(&settings.0)).is_none());
    }

    #[test]
    fn garbage_base64_is_none() {
        // Битый base64 и пустая строка → None, не паника.
        assert!(decode_layout("!!!").is_none());
        assert!(decode_layout("").is_none());
    }

    #[test]
    fn no_folders_section_is_none() {
        // Настройки без field 14.
        let mut settings = W::new();
        settings.ld(2, b"something");
        assert!(decode_layout(&b64(&settings.0)).is_none());
    }
}
