//! Voice packet encryption — NaCl `crypto_secretbox`
//! (XSalsa20 stream cipher + Poly1305 MAC), the scheme behind Discord's
//! "xsalsa20_poly1305" mode.
//!
//! Wire format for each RTP packet in this mode:
//!   [ 12-byte RTP header ][ 16-byte Poly1305 tag ][ encrypted opus payload ]
//! and the 24-byte nonce is simply the RTP header padded with zeros —
//! the header doubles as the nonce, which is why it is sent in the clear.

use anyhow::{bail, Result};
use xsalsa20poly1305::aead::{AeadInPlace, KeyInit};
use xsalsa20poly1305::{Nonce, Tag, XSalsa20Poly1305};

pub struct VoiceCrypto {
    cipher: XSalsa20Poly1305,
}

impl VoiceCrypto {
    pub fn new(secret_key: &[u8]) -> Result<Self> {
        let cipher = XSalsa20Poly1305::new_from_slice(secret_key).map_err(|_| {
            anyhow::anyhow!(
                "ключ голоса должен быть 32 байта, получили {}",
                secret_key.len()
            )
        })?;
        Ok(Self { cipher })
    }

    fn nonce_from_header(header: &[u8]) -> [u8; 24] {
        let mut nonce = [0u8; 24];
        let n = header.len().min(12);
        nonce[..n].copy_from_slice(&header[..n]);
        nonce
    }

    /// Decrypt an xsalsa20_poly1305 RTP payload (tag-first layout).
    pub fn decrypt_rtp(&self, rtp_header: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
        if payload.len() < 16 {
            bail!("RTP payload короче тега ({} байт)", payload.len());
        }
        let (tag, ciphertext) = payload.split_at(16);
        let nonce = Self::nonce_from_header(rtp_header);
        let mut buf = ciphertext.to_vec();
        self.cipher
            .decrypt_in_place_detached(
                Nonce::from_slice(&nonce),
                b"",
                &mut buf,
                Tag::from_slice(tag),
            )
            .map_err(|_| anyhow::anyhow!("xsalsa20poly1305: MAC не сошёлся"))?;
        Ok(buf)
    }

    /// Encrypt an opus payload into the wire format (tag-first).
    /// Используется при отправке микрофона (roadmap) и в unit-тестах.
    #[allow(dead_code)]
    pub fn encrypt_rtp(&self, rtp_header: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = Self::nonce_from_header(rtp_header);
        let mut buf = plaintext.to_vec();
        // Detached encryption returns the Poly1305 tag separately, so we can
        // lay it out tag-first without any post-shuffle.
        let tag = self
            .cipher
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), b"", &mut buf)
            .map_err(|_| anyhow::anyhow!("xsalsa20poly1305: ошибка шифрования"))?;
        let mut out = Vec::with_capacity(16 + buf.len());
        out.extend_from_slice(&tag);
        out.extend_from_slice(&buf);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [7u8; 32];
        let header = [0x80, 0x78, 0x00, 0x01, 0, 0, 0, 0, 1, 2, 3, 4];
        let audio = b"opus-bytes-here".to_vec();
        let c = VoiceCrypto::new(&key).unwrap();
        let wire = c.encrypt_rtp(&header, &audio).unwrap();
        assert_eq!(wire.len(), 16 + audio.len());
        let back = c.decrypt_rtp(&header, &wire).unwrap();
        assert_eq!(back, audio);
    }

    #[test]
    fn tampered_tag_is_rejected() {
        let key = [7u8; 32];
        let header = [0x80, 0x78, 0x00, 0x01, 0, 0, 0, 0, 1, 2, 3, 4];
        let c = VoiceCrypto::new(&key).unwrap();
        let mut wire = c.encrypt_rtp(&header, b"payload").unwrap();
        wire[0] ^= 0xFF; // break the Poly1305 tag
        assert!(c.decrypt_rtp(&header, &wire).is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let header = [0x80, 0x78, 0x00, 0x01, 0, 0, 0, 0, 1, 2, 3, 4];
        let a = VoiceCrypto::new(&[1u8; 32]).unwrap();
        let b = VoiceCrypto::new(&[2u8; 32]).unwrap();
        let wire = a.encrypt_rtp(&header, b"secret").unwrap();
        assert!(b.decrypt_rtp(&header, &wire).is_err());
    }
}
