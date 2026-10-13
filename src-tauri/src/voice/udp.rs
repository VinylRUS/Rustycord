//! UDP layer of the voice protocol: IP discovery (NAT hole punching) and
//! RTP header parsing. Audio payloads themselves are encrypted with the
//! xsalsa20_poly1305 secretbox (see crypto.rs).

use anyhow::{bail, Context, Result};
use std::time::Duration;
use tokio::net::UdpSocket;

/// IP discovery packet: 74 bytes, big-endian.
/// [0]=0x1 (request) [1..3]=length(70) [4..8]=ssrc [8..74]=zeros
///
/// The media server replies with the same layout, but type 0x2 and our
/// external IP + port embedded at the tail — that's how clients learn
/// their NAT-mapped address without STUN.
pub async fn discover(sock: &UdpSocket, ssrc: u32) -> Result<(String, u16)> {
    let mut pkt = [0u8; 74];
    pkt[0] = 0x1;
    pkt[1..3].copy_from_slice(&70u16.to_be_bytes());
    pkt[4..8].copy_from_slice(&ssrc.to_be_bytes());

    sock.send(&pkt).await.context("udp discovery send")?;

    let mut buf = [0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(10), sock.recv(&mut buf))
        .await
        .context("udp discovery timeout")?
        .context("udp discovery recv")?;
    if n < 74 || buf[0] != 0x2 {
        bail!(
            "неожиданный ответ IP discovery ({} байт, type {})",
            n,
            buf[0]
        );
    }

    let ip_len = buf[8..72].iter().position(|&b| b == 0).unwrap_or(64);
    let ip = String::from_utf8_lossy(&buf[8..8 + ip_len]).to_string();
    if ip.is_empty() {
        bail!("пустой адрес в ответе discovery");
    }
    let port = u16::from_le_bytes([buf[72], buf[73]]);
    Ok((ip, port))
}

/// Keepalive: media servers expect periodic traffic or drop the mapping.
/// Resending the discovery request shape is the safest universally
/// accepted keepalive.
pub fn keepalive_packet(ssrc: u32) -> [u8; 74] {
    let mut pkt = [0u8; 74];
    pkt[0] = 0x1;
    pkt[1..3].copy_from_slice(&70u16.to_be_bytes());
    pkt[4..8].copy_from_slice(&ssrc.to_be_bytes());
    pkt
}

/// Собирает RTP-заголовок для отправляемого кадра (v2, без расширений).
/// Discord принимает любой payload type; мы используем 0x78 «opus/audio»,
/// как большинство клиентов. `marker` ставится на первый кадр всплеска речи.
pub fn build_rtp_header(marker: bool, sequence: u16, timestamp: u32, ssrc: u32) -> [u8; 12] {
    let mut h = [0u8; 12];
    h[0] = 0x80; // v2, без padding, без extension, cc=0
    h[1] = if marker { 0x80 | 0x78 } else { 0x78 };
    h[2..4].copy_from_slice(&sequence.to_be_bytes());
    h[4..8].copy_from_slice(&timestamp.to_be_bytes());
    h[8..12].copy_from_slice(&ssrc.to_be_bytes());
    h
}

#[derive(Debug, Clone, Copy)]
pub struct RtpHeader {
    /// RTP version — всегда 2 (валидируется в `parse_rtp_header`).
    #[allow(dead_code)] // протокольная документация кадра
    pub version: u8,
    /// P-бит: пакет дополнен нулями до выравнивания (Discord не использует).
    #[allow(dead_code)] // протокольная документация кадра
    pub padding: bool,
    pub extension: bool,
    pub marker: bool,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    /// total header length incl. extension words (12 if no extension)
    pub header_len: usize,
}

/// RTP fixed header (RFC 3550):
///   0                   1                   2                   3
///   V P X M | PT        | sequence number | timestamp          | ssrc
/// The optional extension (X bit) adds a 2-byte profile header plus
/// length-in-words — Discord uses it for transport-wide sequencing.
pub fn parse_rtp_header(buf: &[u8]) -> Option<RtpHeader> {
    if buf.len() < 12 {
        return None;
    }
    let version = buf[0] >> 6;
    if version != 2 {
        return None;
    }
    let h = RtpHeader {
        version,
        padding: buf[0] & 0x20 != 0,
        extension: buf[0] & 0x10 != 0,
        marker: buf[1] & 0x80 != 0,
        payload_type: buf[1] & 0x7f,
        sequence: u16::from_be_bytes([buf[2], buf[3]]),
        timestamp: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
        ssrc: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
        header_len: 12,
    };
    let mut header_len = 12usize;
    if h.extension && buf.len() >= 14 {
        let words = u16::from_be_bytes([buf[12], buf[13]]) as usize;
        // 2 bytes of profile header are part of the 4-byte unit: RFC says
        // length counts 32-bit words *after* the initial 4-byte header.
        header_len += 4 + words * 4;
    }
    Some(RtpHeader { header_len, ..h })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rtp_header() {
        let mut pkt = [0u8; 20];
        pkt[0] = 0x80; // v2, no padding, no ext
        pkt[1] = 0x78; // pt 120
        pkt[2..4].copy_from_slice(&1234u16.to_be_bytes());
        pkt[8..12].copy_from_slice(&0xDEADBEEFu32.to_be_bytes());
        let h = parse_rtp_header(&pkt).unwrap();
        assert_eq!(h.version, 2);
        assert_eq!(h.sequence, 1234);
        assert_eq!(h.ssrc, 0xDEADBEEF);
        assert_eq!(h.header_len, 12);
    }

    #[test]
    fn parses_rtp_extension() {
        let mut pkt = [0u8; 12 + 4 + 8];
        pkt[0] = 0x90; // v2 + extension
        pkt[12..14].copy_from_slice(&2u16.to_be_bytes()); // 2 words
        let h = parse_rtp_header(&pkt).unwrap();
        assert!(h.extension);
        assert_eq!(h.header_len, 12 + 4 + 8);
    }

    #[test]
    fn build_then_parse_roundtrip() {
        let wire = build_rtp_header(true, 0xBEEF, 0x1122_3344, 0xDEAD_BEEF);
        let h = parse_rtp_header(&wire).unwrap();
        assert_eq!(h.version, 2);
        assert!(h.marker, "маркер должен читаться");
        assert_eq!(h.payload_type, 0x78);
        assert_eq!(h.sequence, 0xBEEF);
        assert_eq!(h.timestamp, 0x1122_3344);
        assert_eq!(h.ssrc, 0xDEAD_BEEF);
        assert_eq!(h.header_len, 12);
        let wire2 = build_rtp_header(false, 1, 960, 7);
        let h2 = parse_rtp_header(&wire2).unwrap();
        assert!(!h2.marker);
    }
}
