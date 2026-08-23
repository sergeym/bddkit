//! SRP-6a client. Pure computation: no I/O, no protocol state, no HTTP.
//!
//! Implementations of SRP-6a agree on the algebra and disagree on what is fed
//! to the hash. `Variant` selects between the two families this tool supports;
//! see the design spec for the exact routines.

use num_bigint::BigUint;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

/// RFC 5054 Appendix A, 4096-bit group. Its generator is 5.
pub const RFC5054_4096_PRIME_HEX: &str = "\
FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DD\
EF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7E\
DEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF\
5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36C\
E3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA\
051015728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E\
1E4C7ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6BF12FFA06D98A0864D87602733EC86A64521F2B1817\
7B200CBBE117577A615D6C770988C0BAD946E208E24FA074E5AB3143DB5BFCE0FD108E4B82D120A92108011A723C12A\
787E6D788719A10BDBA5B2699C327186AF4E23C1A946834B6150BDA2583E9CA2AD44CE8DBBBC2DB04DE8EF92E8EFC14\
1FBECAA6287C59474E6BC05D99B2964FA090C3A2233BA186515BE7ED1F612970CEE2D7AFB81BDD762170481CD006912\
7D5B05AA993B4EA988D8FDDC186FFB7DC90A6C08F4DF435C934063199FFFFFFFFFFFFFFFF";

/// RFC 5054 Appendix A, 1024-bit group. Its generator is 2. Present because the
/// published Appendix B test vectors use it; too small for real deployments —
/// which is why it exists for tests only and is never offered to a config.
#[cfg(test)]
pub const RFC5054_1024_PRIME_HEX: &str = "\
EEAF0AB9ADB38DD69C33F80AFA8FC5E86072618775FF3C0B9EA2314C9C256576D674DF7496EA81D3383B4813D692C6E0\
E0D5D8E250B98BE48E495C1D6089DAD15DC7D7B46154D6B6CE8EF4AD69B15D4982559B297BCF1885C529F566660E57EC\
68EDBC3C05726CC02FD4CBF4976EAA9AFD5138FE8376435B9FC61D2FC0EB06E3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Values are hashed as their hexadecimal string representations, as done
    /// by browser SRP clients built on `thinbus-srp`.
    HexString,
    /// Values are hashed as raw bytes, as specified in RFC 5054.
    Rfc5054,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlg {
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlg {
    pub fn digest(&self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha1 => Sha1::digest(data).to_vec(),
            Self::Sha256 => Sha256::digest(data).to_vec(),
            Self::Sha512 => Sha512::digest(data).to_vec(),
        }
    }

    /// Width of this digest in hex characters.
    pub fn hex_width(&self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SrpParams {
    pub variant: Variant,
    pub prime: BigUint,
    pub generator: BigUint,
    pub hash: HashAlg,
}

impl SrpParams {
    /// Byte length of the prime; every PAD() in the protocol uses it.
    pub fn pad_len(&self) -> usize {
        (self.prime.bits() as usize).div_ceil(8)
    }

    pub fn pad(&self, value: &BigUint) -> Vec<u8> {
        let bytes = value.to_bytes_be();
        let mut out = vec![0u8; self.pad_len().saturating_sub(bytes.len())];
        out.extend_from_slice(&bytes);
        out
    }

    /// k = H(PAD(N) | PAD(g)). Always derived, never configured: a configurable
    /// k is a way to silently disagree with the server.
    pub fn k(&self) -> BigUint {
        let mut input = self.pad(&self.prime);
        input.extend_from_slice(&self.pad(&self.generator));
        BigUint::from_bytes_be(&self.hash.digest(&input))
    }
}

/// Shortest lowercase hex, leading zeros stripped. Matches the textual form
/// SRP implementations exchange for A, B and S.
pub fn hex(value: &BigUint) -> String {
    format!("{value:x}")
}

/// Lowercase hex left-padded with zeros to `width` characters.
pub fn hex_padded(value: &BigUint, width: usize) -> String {
    format!("{value:0width$x}")
}

pub mod hex_string;
pub mod rfc5054;

/// The values a scenario needs after the server has answered the first step.
#[derive(Debug, Clone)]
pub struct LoginProof {
    pub m1: String,
    pub m2: String,
    pub session_key: String,
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("system entropy is available");
    buf.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A fresh salt as 32 random bytes in lowercase hex. Salts are opaque strings;
/// nothing in either variant constrains their length or content.
pub fn generate_salt() -> String {
    random_hex(32)
}

fn parse(label: &str, value: &str) -> Result<BigUint, String> {
    BigUint::parse_bytes(value.as_bytes(), 16)
        .ok_or_else(|| format!("value {label} is not a hexadecimal number: {value:?}"))
}

/// Hex string to raw bytes, length preserved. Must NOT go through `BigUint`:
/// a number has no leading zero byte, so `"00ab"` and `"ab"` would decode to
/// the same single byte and silently change every hash they feed.
fn hex_bytes(label: &str, value: &str) -> Result<Vec<u8>, String> {
    let bad = || format!("value {label} is not a hexadecimal number: {value:?}");
    let digits: Vec<u8> = value
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()
        .ok_or_else(bad)?;
    if !digits.len().is_multiple_of(2) {
        return Err(bad());
    }
    Ok(digits.chunks(2).map(|pair| (pair[0] << 4) | pair[1]).collect())
}

fn x_of(p: &SrpParams, salt: &str, identity: &str, password: &str) -> Result<BigUint, String> {
    Ok(match p.variant {
        Variant::HexString => hex_string::compute_x(p, salt, identity, password),
        Variant::Rfc5054 => {
            rfc5054::compute_x(p, &hex_bytes("salt", salt)?, identity, password)
        }
    })
}

/// v = g^x mod N, as hex.
pub fn compute_verifier(
    p: &SrpParams,
    salt: &str,
    identity: &str,
    password: &str,
) -> Result<String, String> {
    let x = x_of(p, salt, identity, password)?;
    Ok(hex(&p.generator.modpow(&x, &p.prime)))
}

/// A fresh client ephemeral: the private value and its public counterpart,
/// both as hex.
pub fn start_login(p: &SrpParams) -> (String, String) {
    let a = BigUint::parse_bytes(random_hex(32).as_bytes(), 16).expect("generated hex parses");
    let a_pub = p.generator.modpow(&a, &p.prime);
    (hex(&a), hex(&a_pub))
}

/// S = (B - k*g^x)^(a + u*x) mod N, then the proofs derived from it.
///
/// `b_str` is passed through to the hash untouched in the hex-string variant,
/// so the caller must hand over the server's value exactly as received.
pub fn complete_login(
    p: &SrpParams,
    salt: &str,
    identity: &str,
    password: &str,
    a_hex: &str,
    b_str: &str,
) -> Result<LoginProof, String> {
    let n = &p.prime;
    let a = parse("a", a_hex)?;
    let b_pub = parse("B", b_str)?;
    if (&b_pub % n).bits() == 0 {
        return Err("server sent B that is a multiple of N — this value is invalid".into());
    }
    let a_pub = p.generator.modpow(&a, n);
    let a_pub_hex = hex(&a_pub);
    let x = x_of(p, salt, identity, password)?;
    let k = p.k();

    let u = match p.variant {
        Variant::HexString => hex_string::compute_u(p, &a_pub_hex, b_str),
        Variant::Rfc5054 => rfc5054::compute_u(p, &a_pub, &b_pub),
    };

    // B - k*g^x can go negative, so reduce it into [0, N) by adding N once.
    let kgx = (&k * p.generator.modpow(&x, n)) % n;
    let base = (&b_pub + n - kgx) % n;
    let s = base.modpow(&(&a + &u * &x), n);

    Ok(match p.variant {
        // Both proofs are exported zero-padded to the digest width, for two
        // different reasons.
        //
        // M1 is the CLIENT's outbound value: a server that compares it as a
        // fixed-width string expects the padded form, and would disagree with
        // the stripped-leading-zero form some browser clients produce about one
        // login in 16 — whenever M1's leading nibble happens to be zero.
        //
        // M2 is the server's value, and is compared against the string the
        // server returned using the ordinary variable-equality step, so the two
        // representations have to agree exactly for the same reason.
        Variant::HexString => {
            let s_hex = hex(&s);
            let m1 = hex_string::compute_m1(p, &a_pub_hex, b_str, &s_hex);
            let width = p.hash.hex_width();
            LoginProof {
                m2: hex_padded(&hex_string::compute_m2(p, &a_pub_hex, &m1, &s_hex), width),
                m1: hex_padded(&m1, width),
                session_key: hex_string::session_key(p, &s_hex),
            }
        }
        Variant::Rfc5054 => {
            let salt_bytes = hex_bytes("salt", salt)?;
            let key = rfc5054::session_key(p, &s);
            let m1 = rfc5054::compute_m1(p, identity, &salt_bytes, &a_pub, &b_pub, &key);
            let width = p.hash.hex_width();
            LoginProof {
                m2: hex_padded(&rfc5054::compute_m2(p, &a_pub, &m1, &key), width),
                m1: hex_padded(&m1, width),
                session_key: key.iter().map(|byte| format!("{byte:02x}")).collect(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // `use super::*` re-exports the module's own public items, not the names it
    // imports, so BigUint has to be brought in again here.
    use num_bigint::BigUint;

    fn params_4096_sha256() -> SrpParams {
        SrpParams {
            variant: Variant::HexString,
            prime: BigUint::parse_bytes(RFC5054_4096_PRIME_HEX.as_bytes(), 16).expect("valid prime"),
            generator: BigUint::from(5u32),
            hash: HashAlg::Sha256,
        }
    }

    fn params_1024_sha1() -> SrpParams {
        SrpParams {
            variant: Variant::Rfc5054,
            prime: BigUint::parse_bytes(RFC5054_1024_PRIME_HEX.as_bytes(), 16).expect("valid prime"),
            generator: BigUint::from(2u32),
            hash: HashAlg::Sha1,
        }
    }

    /// Published value for the RFC 5054 4096-bit group with SHA-256, taken from
    /// an independent client implementation that ships it as a literal.
    #[test]
    fn k_matches_the_published_constant_for_the_4096_bit_group() {
        assert_eq!(
            hex(&params_4096_sha256().k()),
            "3509477ea9fca66eadb7cf7b1bd0eb508f54d3989a9c988006a7d0b338374dd2"
        );
    }

    /// RFC 5054 Appendix B.
    #[test]
    fn k_matches_the_rfc_vector_for_the_1024_bit_group() {
        assert_eq!(
            hex(&params_1024_sha1().k()),
            "7556aa045aef2cdd07abaf0f665c3e818913186f"
        );
    }

    #[test]
    fn hex_strips_leading_zeros_and_hex_padded_restores_them() {
        let value = BigUint::parse_bytes(b"0e78fe", 16).expect("valid hex");
        assert_eq!(hex(&value), "e78fe");
        assert_eq!(hex_padded(&value, 6), "0e78fe");
    }

    /// Plays the server half of the protocol so the client can be checked
    /// end to end: the server proof only matches if x, u, S, M1 and M2 all do.
    fn server_proof(p: &SrpParams, salt: &str, identity: &str, password: &str, b_hex: &str, a_pub_hex: &str, m1_hex: &str) -> String {
        let n = &p.prime;
        let b_priv = BigUint::parse_bytes(b_hex.as_bytes(), 16).expect("valid hex");
        let a_pub = BigUint::parse_bytes(a_pub_hex.as_bytes(), 16).expect("valid hex");
        let verifier_hex = compute_verifier(p, salt, identity, password).expect("verifier");
        let v = BigUint::parse_bytes(verifier_hex.as_bytes(), 16).expect("valid hex");
        let b_pub = (p.k() * &v + p.generator.modpow(&b_priv, n)) % n;
        let s = match p.variant {
            Variant::HexString => {
                let u = hex_string::compute_u(p, a_pub_hex, &hex(&b_pub));
                (&a_pub * v.modpow(&u, n)).modpow(&b_priv, n)
            }
            Variant::Rfc5054 => {
                let u = rfc5054::compute_u(p, &a_pub, &b_pub);
                (&a_pub * v.modpow(&u, n)).modpow(&b_priv, n)
            }
        };
        let m1 = BigUint::parse_bytes(m1_hex.as_bytes(), 16).expect("valid hex");
        // Padded, because that is the form a server puts on the wire and the
        // form the client exports for a textual comparison.
        let width = p.hash.hex_width();
        match p.variant {
            Variant::HexString => {
                hex_padded(&hex_string::compute_m2(p, a_pub_hex, &m1, &hex(&s)), width)
            }
            Variant::Rfc5054 => {
                let key = rfc5054::session_key(p, &s);
                hex_padded(&rfc5054::compute_m2(p, &a_pub, &m1, &key), width)
            }
        }
    }

    fn round_trip(p: &SrpParams) {
        let salt = generate_salt();
        let identity = "user@example.test";
        let password = "correct horse battery staple";
        let (a_hex, a_pub_hex) = start_login(p);

        let b_hex = "5c2e91a0d7b34f6812ae09d5c73b6e4f2a81d09c5e6b47a3f0982d1c6b5a4e30";
        let b_priv = BigUint::parse_bytes(b_hex.as_bytes(), 16).expect("valid hex");
        let verifier = compute_verifier(p, &salt, identity, password).expect("verifier");
        let v = BigUint::parse_bytes(verifier.as_bytes(), 16).expect("valid hex");
        let b_pub = (p.k() * v + p.generator.modpow(&b_priv, &p.prime)) % &p.prime;

        let proof = complete_login(p, &salt, identity, password, &a_hex, &hex(&b_pub))
            .expect("login completes");
        assert_eq!(
            proof.m2,
            server_proof(p, &salt, identity, password, b_hex, &a_pub_hex, &proof.m1),
            "client and server must agree on the server proof"
        );
    }

    #[test]
    fn hex_string_variant_agrees_with_a_server() {
        round_trip(&params_4096_sha256());
    }

    #[test]
    fn rfc5054_variant_agrees_with_a_server() {
        round_trip(&params_1024_sha1());
    }

    #[test]
    fn a_wrong_password_produces_a_different_proof() {
        let p = params_4096_sha256();
        let salt = generate_salt();
        let (a_hex, a_pub_hex) = start_login(&p);
        let b_hex = "5c2e91a0d7b34f6812ae09d5c73b6e4f2a81d09c5e6b47a3f0982d1c6b5a4e30";
        let b_priv = BigUint::parse_bytes(b_hex.as_bytes(), 16).expect("valid hex");
        let verifier = compute_verifier(&p, &salt, "u@example.test", "right").expect("verifier");
        let v = BigUint::parse_bytes(verifier.as_bytes(), 16).expect("valid hex");
        let b_pub = (p.k() * v + p.generator.modpow(&b_priv, &p.prime)) % &p.prime;

        let proof = complete_login(&p, &salt, "u@example.test", "wrong", &a_hex, &hex(&b_pub))
            .expect("login computes");
        assert_ne!(
            proof.m2,
            server_proof(&p, &salt, "u@example.test", "right", b_hex, &a_pub_hex, &proof.m1)
        );
    }

    /// The rfc5054 variant decodes the salt as bytes, so a leading zero byte is
    /// significant. Routing it through a BigUint dropped that byte and made
    /// "00beb2…" hash identically to "beb2…" — a silent mismatch with any
    /// server that decoded the same hex correctly.
    #[test]
    fn a_leading_zero_byte_in_the_rfc5054_salt_changes_x() {
        let p = params_1024_sha1();
        let stripped = x_of(&p, "beb25379d1a8581eb5a727673a2441ee", "alice", "password123")
            .expect("x computes");
        let with_zero = x_of(&p, "00beb25379d1a8581eb5a727673a2441ee", "alice", "password123")
            .expect("x computes");
        // The stripped form is the RFC 5054 Appendix B vector: proves the decode
        // itself is right, not merely different.
        assert_eq!(hex(&stripped), "94b7555aabe9127cc58ccf4993db6cf84d16c124");
        assert_ne!(
            with_zero, stripped,
            "a leading zero byte in the salt must change x"
        );
    }

    #[test]
    fn a_salt_that_is_not_whole_hex_bytes_is_rejected_by_name() {
        let p = params_1024_sha1();
        for salt in ["abc", "zz"] {
            let err = x_of(&p, salt, "alice", "password123").expect_err("salt is invalid");
            assert!(err.contains("salt"), "error must name the value: {err}");
        }
    }

    #[test]
    fn salts_differ_between_calls() {
        assert_ne!(generate_salt(), generate_salt());
        assert_eq!(generate_salt().len(), 64);
    }

    #[test]
    fn a_malformed_server_value_is_rejected_by_name() {
        let p = params_4096_sha256();
        let (a_hex, _) = start_login(&p);
        let err = complete_login(&p, "aa", "u", "p", &a_hex, "not-hex").unwrap_err();
        assert!(err.contains("B"), "error must name the offending value: {err}");
    }
}
