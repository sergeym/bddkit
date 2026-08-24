use crate::json::path;
use crate::polling::{AttemptError, AttemptResult};
use crate::world::World;
use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_aes256_key(key_hex: &str) -> Result<[u8; 32], String> {
    if key_hex.len() != 64 {
        return Err("AES-256 key must be exactly 64 hexadecimal characters".to_string());
    }

    let mut key = [0; 32];
    for (byte, pair) in key.iter_mut().zip(key_hex.as_bytes().as_chunks::<2>().0) {
        let high = hex_nibble(pair[0])
            .ok_or_else(|| "AES-256 key must contain only hexadecimal characters".to_string())?;
        let low = hex_nibble(pair[1])
            .ok_or_else(|| "AES-256 key must contain only hexadecimal characters".to_string())?;
        *byte = (high << 4) | low;
    }
    Ok(key)
}

fn encrypt_aes256_cbc(plaintext: &str, key: &[u8; 32], iv: &[u8; 16]) -> String {
    let ciphertext = cbc::Encryptor::<aes::Aes256>::new(key.into(), iv.into())
        .encrypt_padded_vec::<Pkcs7>(plaintext.as_bytes());
    STANDARD.encode(ciphertext)
}

pub fn encrypt_with_aes(
    world: &mut World,
    plaintext: &str,
    key_hex: &str,
    prefix: &str,
) -> Result<(), String> {
    let key = decode_aes256_key(key_hex)?;
    let mut iv = [0; 16];
    getrandom::fill(&mut iv).map_err(|_| "failed to read system entropy".to_string())?;
    let ciphertext = encrypt_aes256_cbc(plaintext, &key, &iv);
    let iv_hex = iv.iter().map(|byte| format!("{byte:02x}")).collect();

    world.vars.set(&format!("{prefix}_ciphertext"), ciphertext);
    world.vars.set(&format!("{prefix}_ivHex"), iv_hex);
    Ok(())
}

pub fn set_variable(w: &mut World, name: &str, value: &str, global: bool) -> Result<(), String> {
    if global {
        w.vars.set_global(name, value.to_string());
    } else {
        w.vars.set(name, value.to_string());
    }
    Ok(())
}

/// Scalar values are stored as-is; strings without JSON quotes.
fn scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn extract_from_json(w: &mut World, p: &str, name: &str, global: bool) -> Result<(), String> {
    let ex = w.http.last().ok_or("no request has been sent yet")?;
    let v = ex.json()?;
    let found = path::read(&v, p)?;
    let value = scalar(found);
    if global {
        w.vars.set_global(name, value);
    } else {
        w.vars.set(name, value);
    }
    Ok(())
}

pub fn extract_from_cookies(
    w: &mut World,
    cookie: &str,
    name: &str,
    global: bool,
) -> Result<(), String> {
    let ex = w.http.last().ok_or("no request has been sent yet")?;
    let value = ex
        .set_cookie(cookie)
        .ok_or_else(|| format!("cookie {cookie:?} not found in the response"))?;
    if global {
        w.vars.set_global(name, value);
    } else {
        w.vars.set(name, value);
    }
    Ok(())
}

pub fn variable_equals(w: &World, name: &str, expected: &str, negate: bool) -> AttemptResult {
    let got = w
        .vars
        .get(name)
        .ok_or_else(|| AttemptError::Fatal(format!("variable {name:?} is not set")))?;
    let equal = got == expected;
    match (equal, negate) {
        (true, false) | (false, true) => Ok(()),
        (false, false) => Err(AttemptError::NotYet(format!(
            "    expected: {expected}\n    actual:   {got}"
        ))),
        (true, true) => Err(AttemptError::NotYet(format!(
            "value must not equal {expected:?}, but it does"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn world() -> World {
        let resources = HashMap::from([(
            "main".to_string(),
            crate::http::ApiResource::new(
                "http://x.local",
                5,
                Vec::new(),
                crate::options::Options::default(),
            )
            .expect("valid base URL"),
        )]);
        let apis = Arc::new(
            crate::http::Apis::new(resources, Some("main".to_string()))
                .expect("default API exists"),
        );
        World::new(
            apis,
            Arc::new(crate::unique::Generator::new()),
            crate::db::DbHandle::new(None, String::new()),
            None,
            None,
            crate::options::Options::default(),
        )
    }

    #[test]
    fn aes256_cbc_matches_a_known_fixed_vector() {
        let key =
            decode_aes256_key("000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F")
                .expect("valid key");
        let iv = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x1f,
        ];

        assert_eq!(
            encrypt_aes256_cbc("555555", &key, &iv),
            "NHHyjRAmNadQ7tSNhxFESA=="
        );
    }

    #[test]
    fn aes_step_exports_decryptable_ciphertext_and_lowercase_iv() {
        let mut world = world();
        let key = [0x11; 32];
        encrypt_with_aes(&mut world, "plain text", &"11".repeat(32), "otp")
            .expect("encryption succeeds");

        let iv_hex = world.vars.get("otp_ivHex").expect("IV exported");
        assert_eq!(iv_hex.len(), 32);
        assert!(
            iv_hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        let iv: Vec<u8> = iv_hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                    .expect("valid hex")
            })
            .collect();
        let ciphertext = STANDARD
            .decode(
                world
                    .vars
                    .get("otp_ciphertext")
                    .expect("ciphertext exported"),
            )
            .expect("valid Base64");
        let plaintext = cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
            .expect("fixed key and IV sizes")
            .decrypt_padded_vec::<Pkcs7>(&ciphertext)
            .expect("valid padding");

        assert_eq!(plaintext, b"plain text");
    }

    #[test]
    fn aes_step_rejects_non_hex_key_without_exports() {
        let mut world = world();
        let error = encrypt_with_aes(&mut world, "secret", &"z0".repeat(32), "otp")
            .expect_err("non-hex key is rejected");

        assert!(error.contains("hexadecimal"), "{error}");
        assert_eq!(world.vars.get("otp_ciphertext"), None);
        assert_eq!(world.vars.get("otp_ivHex"), None);
    }

    #[test]
    fn aes_step_rejects_non_256_bit_key_without_exports() {
        let mut world = world();
        let error = encrypt_with_aes(&mut world, "secret", &"11".repeat(16), "otp")
            .expect_err("short key is rejected");

        assert!(error.contains("64"), "{error}");
        assert_eq!(world.vars.get("otp_ciphertext"), None);
        assert_eq!(world.vars.get("otp_ivHex"), None);
    }
}
