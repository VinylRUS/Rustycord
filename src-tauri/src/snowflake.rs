//! Snowflake — Discord's distributed unique ID scheme, decoded.
//!
//! Layout (64 bits, big-endian conceptually):
//!   [ 42 bits: ms since Discord epoch ][ 5: worker ][ 5: process ][ 12: increment ]
//!
//! The Discord epoch is 2015-01-01T00:00:00 UTC — the week the first
//! snowflake IDs shipped. Fun reverse-engineering consequence: the API does
//! not attach timestamps to messages, because the client can decode the id.

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

pub const DISCORD_EPOCH: i64 = 1_420_070_400_000; // 2015-01-01T00:00:00.000Z

#[derive(Debug, Clone, Serialize)]
pub struct SnowflakeInfo {
    pub id: String,
    pub timestamp_ms: i64,
    pub created_at: String, // RFC 3339
    pub worker_id: u16,
    pub process_id: u16,
    pub increment: u16,
    pub age_seconds: i64,
}

pub fn parse(id: &str) -> Result<SnowflakeInfo> {
    let trimmed = id.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        bail!("Snowflake состоит только из цифр");
    }
    let v: u64 = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("Слишком большое число — это точно Snowflake?"))?;
    if v == 0 {
        bail!("Snowflake не может быть нулём");
    }
    let ts = ((v >> 22) as i64).wrapping_add(DISCORD_EPOCH);
    let dt = DateTime::from_timestamp_millis(ts)
        .ok_or_else(|| anyhow::anyhow!("Не удалось интерпретировать timestamp"))?;
    Ok(SnowflakeInfo {
        id: trimmed.to_string(),
        timestamp_ms: ts,
        created_at: dt.to_rfc3339(),
        worker_id: ((v >> 17) & 0x1f) as u16,
        process_id: ((v >> 12) & 0x1f) as u16,
        increment: (v & 0xfff) as u16,
        age_seconds: (Utc::now().timestamp_millis() - ts).max(0) / 1000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_known_snowflake() {
        // Реальный id августа 2023, значения сверены побитово:
        //   timestamp 1692561112874 · worker 17 · process 14 · increment 50
        let info = parse("1142908886972555314").unwrap();
        assert_eq!(info.timestamp_ms, 1692561112874);
        assert_eq!(info.worker_id, 17);
        assert_eq!(info.process_id, 14);
        assert_eq!(info.increment, 50);
        assert!(info.created_at.starts_with("2023-08-20T19:51:52"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("hello").is_err());
        assert!(parse("").is_err());
        assert!(parse("0").is_err());
        // 65-битное число не влезает в u64
        assert!(parse("99999999999999999999999").is_err());
    }
}
