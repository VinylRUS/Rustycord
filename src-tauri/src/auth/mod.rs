//! Авторизация как в оригинальном клиенте: email+пароль (+2FA) и QR-код
//! (remote auth). Оба пути выдают **пользовательский** токен — это то же
//! самое, чем пользуется официальный веб-клиент.

pub mod login;
pub mod remote;

use base64::Engine;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use sha2::Sha256;

/// RSA-ключевая пара одной QR-сессии: сервер шифрует ею nonce и итоговый
/// токен, расшифровать может только мы.
pub struct RemoteAuthKeys {
    pub private: RsaPrivateKey,
    pub public_b64: String,
}

impl RemoteAuthKeys {
    pub fn generate() -> anyhow::Result<Self> {
        // 2048 бит — стандарт remote-auth; генерация занимает доли секунды.
        let private = RsaPrivateKey::new(&mut OsRng, 2048)?;
        let public = RsaPublicKey::from(&private);
        // SPKI DER в base64 — именно так ожидает op:"init".
        let der = public
            .to_public_key_der()
            .map_err(|e| anyhow::anyhow!("DER encode: {e}"))?;
        Ok(Self {
            private,
            public_b64: base64::engine::general_purpose::STANDARD.encode(der.as_bytes()),
        })
    }

    /// Расшифровать поле (nonce / encrypted_token), зашифрованное нашим
    /// публичным ключом (RSA-OAEP-SHA256, как в браузерном клиенте).
    pub fn decrypt(&self, b64_data: &str) -> anyhow::Result<Vec<u8>> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64_data)
            .map_err(|e| anyhow::anyhow!("base64: {e}"))?;
        let plain = self
            .private
            .decrypt(Oaep::new::<Sha256>(), &raw)
            .map_err(|e| anyhow::anyhow!("RSA-OAEP: {e}"))?;
        Ok(plain)
    }
}

/// proof = base64(HMAC-SHA256(key = nonce, msg = nonce)) — доказательство
/// серверу, что мы владеем приватным ключом, не раскрывая его.
pub fn nonce_proof(nonce_b64: &str, keys: &RemoteAuthKeys) -> anyhow::Result<String> {
    let nonce = keys.decrypt(nonce_b64)?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(&nonce).map_err(|e| anyhow::anyhow!("hmac init: {e}"))?;
    mac.update(&nonce);
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}
