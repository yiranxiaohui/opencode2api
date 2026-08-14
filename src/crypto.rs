use aes_gcm::aead::consts::U12;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::ApiError;

pub const KEY_LEN: usize = 32; // AES-256 key = argon2 default output length

fn generate_random_token(prefix: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn generate_client_key() -> String {
    generate_random_token("sk-")
}

pub fn generate_admin_token() -> String {
    generate_random_token("oca_admin_")
}

pub fn generate_web_session_token() -> String {
    generate_random_token("oca_session_")
}

pub fn hash_client_key(key: &str) -> String {
    B64.encode(Sha256::digest(key.as_bytes()))
}

pub fn encode_master_key(key: &[u8; KEY_LEN]) -> String {
    B64.encode(key)
}

pub fn decode_master_key(encoded: &str) -> Result<Zeroizing<[u8; KEY_LEN]>, ApiError> {
    let bytes = B64
        .decode(encoded)
        .map_err(|_| ApiError::Internal("invalid persisted master key".into()))?;
    let key = bytes
        .try_into()
        .map_err(|_| ApiError::Internal("invalid persisted master key length".into()))?;
    Ok(Zeroizing::new(key))
}

/// Hash a master password, returning the PHC string (salt embedded).
pub fn hash_password(pw: &[u8]) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default()
        .hash_password(pw, &salt)
        .map_err(|e| ApiError::Internal(format!("argon2: {e}")))?
        .to_string();
    Ok(phc)
}

/// Constant-time-ish verify of a password against a stored PHC string.
pub fn verify_password(phc: &str, pw: &[u8]) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default().verify_password(pw, &parsed).is_ok()
}

/// Deterministically re-derive the AES-256 key from the password and the stored
/// PHC string (salt + params live inside the PHC string). `Argon2::default()`
/// is Argon2id, and its default output length is exactly 32 bytes.
pub fn derive_key(phc: &str, pw: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>, ApiError> {
    let parsed = PasswordHash::new(phc).map_err(|e| ApiError::Internal(format!("argon2: {e}")))?;
    let salt = parsed
        .salt
        .ok_or_else(|| ApiError::Internal("missing salt in stored hash".into()))?;
    let mut salt_buf = [0u8; 64];
    let raw_salt = salt
        .decode_b64(&mut salt_buf)
        .map_err(|e| ApiError::Internal(format!("argon2: {e}")))?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::default()
        .hash_password_into(pw, raw_salt, &mut key[..])
        .map_err(|e| ApiError::Internal(format!("argon2: {e}")))?;
    Ok(key)
}

/// Encrypt plaintext with AES-256-GCM, fresh random nonce per call.
/// Output format: base64(nonce[12] || ciphertext || tag[16]).
pub fn encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<String, ApiError> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| ApiError::Internal(format!("aes: {e}")))?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let nonce_arr =
        Nonce::<U12>::try_from(&nonce[..]).map_err(|_| ApiError::Internal("bad nonce".into()))?;
    let ct = cipher
        .encrypt(&nonce_arr, plaintext)
        .map_err(|e| ApiError::Internal(format!("aes: {e}")))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(B64.encode(out))
}

/// Decrypt ciphertext produced by `encrypt`. Returns plaintext in a Zeroizing buffer.
pub fn decrypt(key: &[u8; KEY_LEN], encoded: &str) -> Result<Zeroizing<Vec<u8>>, ApiError> {
    let raw = B64
        .decode(encoded)
        .map_err(|_| ApiError::Internal("bad ciphertext encoding".into()))?;
    if raw.len() <= 12 {
        return Err(ApiError::Internal("bad ciphertext length".into()));
    }
    let (nonce, ct) = raw.split_at(12);
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| ApiError::Internal(format!("aes: {e}")))?;
    let nonce_arr =
        Nonce::<U12>::try_from(nonce).map_err(|_| ApiError::Internal("bad nonce".into()))?;
    let pt = cipher
        .decrypt(&nonce_arr, ct)
        .map_err(|_| ApiError::Internal("decrypt failed (wrong master password?)".into()))?;
    Ok(Zeroizing::new(pt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let phc = hash_password(b"hunter2").unwrap();
        let key = derive_key(&phc, b"hunter2").unwrap();
        let enc = encrypt(&key, b"sk-secret-123").unwrap();
        let dec = decrypt(&key, &enc).unwrap();
        assert_eq!(&*dec, b"sk-secret-123");
        assert_ne!(enc, "sk-secret-123");
    }

    #[test]
    fn wrong_password_rejected() {
        let phc = hash_password(b"right").unwrap();
        assert!(verify_password(&phc, b"right"));
        assert!(!verify_password(&phc, b"wrong"));
    }

    #[test]
    fn nonce_unique() {
        let phc = hash_password(b"pw").unwrap();
        let key = derive_key(&phc, b"pw").unwrap();
        let a = encrypt(&key, b"same").unwrap();
        let b = encrypt(&key, b"same").unwrap();
        assert_ne!(a, b, "fresh nonce should produce distinct ciphertext");
    }

    #[test]
    fn derived_key_is_deterministic() {
        let phc = hash_password(b"pw").unwrap();
        let k1 = derive_key(&phc, b"pw").unwrap();
        let k2 = derive_key(&phc, b"pw").unwrap();
        assert_eq!(&*k1, &*k2);
    }

    #[test]
    fn client_keys_are_unique_and_hashable() {
        let a = generate_client_key();
        let b = generate_client_key();
        assert!(a.starts_with("sk-"));
        assert_ne!(a, b);
        assert_eq!(hash_client_key(&a), hash_client_key(&a));
        assert_ne!(hash_client_key(&a), hash_client_key(&b));
    }

    #[test]
    fn management_tokens_have_distinct_prefixes() {
        assert!(generate_admin_token().starts_with("oca_admin_"));
        assert!(generate_web_session_token().starts_with("oca_session_"));
    }

    #[test]
    fn persisted_master_key_roundtrip() {
        let key = Zeroizing::new([42u8; KEY_LEN]);
        let restored = decode_master_key(&encode_master_key(&key)).unwrap();
        assert_eq!(&*restored, &*key);
    }
}
