//! Secrets section encryption: password → Argon2id → 32-byte key → XChaCha20-Poly1305.
//! API verified against argon2 0.6 / chacha20poly1305 0.11 (`Array::from_slice` is deprecated there,
//! hence `new_from_slice` for the key and `XNonce::from([u8; 24])` for the nonce).

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::format::{KdfSpec, SecretsEnvelope, SecretsPlain};
use crate::error::{AppError, AppResult};

pub const KDF_ALG: &str = "argon2id";
pub const CIPHER: &str = "xchacha20poly1305";
const AAD: &[u8] = b"cc-router-export/v1/secrets";
pub const MIN_PASSWORD_CHARS: usize = 10;

// Upper bounds on attacker-controlled KDF params: a crafted file must not be able to make
// the importer allocate gigabytes or spin for minutes.
const MAX_M_KIB: u32 = 262_144;
const MAX_T: u32 = 10;
const MAX_P: u32 = 4;

#[derive(Debug, Clone, Copy)]
pub struct KdfCost {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfCost {
    pub const DEFAULT: Self = Self { m_kib: 65_536, t: 3, p: 1 };
    #[cfg(test)]
    pub const FAST: Self = Self { m_kib: 8, t: 1, p: 1 };
}

#[derive(Debug, PartialEq, Eq)]
pub enum CryptoError {
    /// Wrong password or corrupted / tampered file. AEAD cannot tell these apart.
    Invalid,
    Unsupported(&'static str),
}

impl From<CryptoError> for AppError {
    fn from(e: CryptoError) -> Self {
        match e {
            CryptoError::Invalid => AppError::BadRequest("密码错误或文件已损坏".into()),
            CryptoError::Unsupported(what) => {
                AppError::BadRequest(format!("文件的加密参数不受支持 ({what})"))
            }
        }
    }
}

pub fn validate_password(password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AppError::BadRequest(format!("密码至少 {MIN_PASSWORD_CHARS} 个字符")));
    }
    Ok(())
}

fn derive_key(password: &str, salt: &[u8], cost: KdfCost) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    let params = Params::new(cost.m_kib, cost.t, cost.p, Some(32))
        .map_err(|_| CryptoError::Unsupported("kdf"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key[..])
        .map_err(|_| CryptoError::Unsupported("kdf"))?;
    Ok(key)
}

fn cost_of(env: &SecretsEnvelope) -> Result<KdfCost, CryptoError> {
    if env.kdf.alg != KDF_ALG {
        return Err(CryptoError::Unsupported("kdf"));
    }
    if env.cipher != CIPHER {
        return Err(CryptoError::Unsupported("cipher"));
    }
    let c = KdfCost { m_kib: env.kdf.m_kib, t: env.kdf.t, p: env.kdf.p };
    if c.m_kib > MAX_M_KIB || c.t == 0 || c.t > MAX_T || c.p == 0 || c.p > MAX_P {
        return Err(CryptoError::Unsupported("kdf params"));
    }
    Ok(c)
}

pub fn seal(plain: &SecretsPlain, password: &str, cost: KdfCost) -> AppResult<SecretsEnvelope> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut salt)
        .and_then(|_| getrandom::fill(&mut nonce))
        .map_err(|e| AppError::internal(format!("系统随机数不可用: {e}")))?;
    let key = derive_key(password, &salt, cost)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key[..])
        .map_err(|_| AppError::internal("密钥长度错误"))?;
    let msg = Zeroizing::new(serde_json::to_vec(plain)?);
    let ciphertext = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg: &msg, aad: AAD })
        .map_err(|_| AppError::internal("加密失败"))?;
    Ok(SecretsEnvelope {
        kdf: KdfSpec {
            alg: KDF_ALG.into(),
            m_kib: cost.m_kib,
            t: cost.t,
            p: cost.p,
            salt: STANDARD.encode(salt),
        },
        cipher: CIPHER.into(),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

pub fn open(env: &SecretsEnvelope, password: &str) -> Result<SecretsPlain, CryptoError> {
    let cost = cost_of(env)?;
    let salt = STANDARD.decode(&env.kdf.salt).map_err(|_| CryptoError::Invalid)?;
    let nonce: [u8; 24] = STANDARD
        .decode(&env.nonce)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(CryptoError::Invalid)?;
    let ciphertext = STANDARD.decode(&env.ciphertext).map_err(|_| CryptoError::Invalid)?;
    let key = derive_key(password, &salt, cost)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key[..]).map_err(|_| CryptoError::Invalid)?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(&XNonce::from(nonce), Payload { msg: &ciphertext, aad: AAD })
            .map_err(|_| CryptoError::Invalid)?,
    );
    serde_json::from_slice(&plain).map_err(|_| CryptoError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::format::{SecretBind, SecretItem};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use uuid::Uuid;

    fn sample() -> SecretsPlain {
        let mut p = SecretsPlain { auth_token: Some("tok".into()), ..Default::default() };
        p.items.insert(
            "s1".into(),
            SecretItem {
                value: "sk-1".into(),
                bind: SecretBind { subscription_id: Uuid::nil(), field: "api_key".into(), destinations: vec!["https://a".into()] },
            },
        );
        p
    }

    #[test]
    fn roundtrip() {
        let env = seal(&sample(), "correct horse battery", KdfCost::FAST).unwrap();
        assert_eq!(env.kdf.alg, "argon2id");
        assert_eq!(env.cipher, "xchacha20poly1305");
        assert_eq!(open(&env, "correct horse battery").unwrap(), sample());
    }

    #[test]
    fn wrong_password_is_invalid() {
        let env = seal(&sample(), "correct horse battery", KdfCost::FAST).unwrap();
        assert_eq!(open(&env, "wrong password!!"), Err(CryptoError::Invalid));
    }

    fn flip_b64(s: &str) -> String {
        let mut bytes = STANDARD.decode(s).unwrap();
        bytes[0] ^= 0x01;
        STANDARD.encode(bytes)
    }

    #[test]
    fn any_tampered_byte_is_the_same_invalid_error() {
        let env = seal(&sample(), "correct horse battery", KdfCost::FAST).unwrap();
        let mut a = env.clone();
        a.ciphertext = flip_b64(&a.ciphertext);
        let mut b = env.clone();
        b.nonce = flip_b64(&b.nonce);
        let mut c = env.clone();
        c.kdf.salt = flip_b64(&c.kdf.salt);
        for e in [a, b, c] {
            assert_eq!(open(&e, "correct horse battery"), Err(CryptoError::Invalid));
        }
    }

    #[test]
    fn oversized_kdf_params_are_rejected_before_running_the_kdf() {
        let mut env = seal(&sample(), "correct horse battery", KdfCost::FAST).unwrap();
        env.kdf.m_kib = 10_000_000; // would allocate ~10 GB if actually run
        assert!(matches!(open(&env, "correct horse battery"), Err(CryptoError::Unsupported(_))));
        let mut env2 = seal(&sample(), "correct horse battery", KdfCost::FAST).unwrap();
        env2.kdf.t = 0;
        assert!(matches!(open(&env2, "correct horse battery"), Err(CryptoError::Unsupported(_))));
    }

    #[test]
    fn unknown_algorithms_are_rejected() {
        let mut env = seal(&sample(), "correct horse battery", KdfCost::FAST).unwrap();
        env.kdf.alg = "scrypt".into();
        assert!(matches!(open(&env, "correct horse battery"), Err(CryptoError::Unsupported(_))));
        let mut env2 = seal(&sample(), "correct horse battery", KdfCost::FAST).unwrap();
        env2.cipher = "aes-256-gcm".into();
        assert!(matches!(open(&env2, "correct horse battery"), Err(CryptoError::Unsupported(_))));
    }

    #[test]
    fn password_length_counts_chars_not_bytes() {
        assert!(validate_password("123456789").is_err());
        assert!(validate_password("1234567890").is_ok());
        assert!(validate_password("密码密码密码密码密码").is_ok(), "10 个汉字 = 10 字符");
    }

    #[test]
    fn default_cost_matches_the_spec() {
        let c = KdfCost::DEFAULT;
        assert_eq!((c.m_kib, c.t, c.p), (65_536, 3, 1));
    }
}
