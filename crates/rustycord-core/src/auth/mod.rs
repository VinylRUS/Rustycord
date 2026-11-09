//! Авторизация как в оригинальном клиенте: email+пароль (+2FA) и QR-код
//! (remote auth). Оба пути выдают **пользовательский** токен — это то же
//! самое, чем пользуется официальный веб-клиент.

pub mod captcha;
pub mod login;
pub mod remote;

use base64::Engine;
use rsa::pkcs8::EncodePublicKey;
use rsa::rand_core::OsRng;
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};

/// RSA-ключевая пара одной QR-сессии: сервер шифрует ею nonce, профиль
/// пользователя и итоговый токен, расшифровать может только мы.
pub struct RemoteAuthKeys {
    pub private: RsaPrivateKey,
    pub public_b64: String,
    /// SPKI DER (те же байты, что закодированы в `public_b64`) — нужен
    /// для сверки fingerprint из pending_remote_init.
    pub public_der: Vec<u8>,
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
        let public_der = der.as_bytes().to_vec();
        Ok(Self {
            private,
            public_b64: base64::engine::general_purpose::STANDARD.encode(&public_der),
            public_der,
        })
    }

    /// Расшифровать поле (nonce / профиль пользователя / токен), зашифрованное
    /// нашим публичным ключом (RSA-OAEP-SHA256, как в браузерном клиенте).
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

    /// Fingerprint, который сервер обязан прислать в pending_remote_init:
    /// base64URL(SHA-256 от SPKI DER) без паддинга. Сверка гарантирует,
    /// что QR ведёт именно на нашу сессию, а не подменён (docs.discord.food).
    pub fn expected_fingerprint(&self) -> String {
        let digest = Sha256::digest(&self.public_der);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    }
}

/// Ответ на op:"nonce_proof" протокола v2: вернуть РАСШИФРОВАННЫЙ nonce,
/// закодированный в base64URL без «=». HMAC-схемы из v1 больше нет —
/// сервер просто проверяет, что мы владеем приватным ключом.
pub fn nonce_echo(nonce_b64: &str, keys: &RemoteAuthKeys) -> anyhow::Result<String> {
    let nonce = keys.decrypt(nonce_b64)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce))
}
