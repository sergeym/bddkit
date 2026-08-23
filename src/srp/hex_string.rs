//! Routines that hash the hexadecimal string representations of the protocol
//! values rather than their bytes, as used by browser SRP clients built on
//! `thinbus-srp` and by servers written to interoperate with them.

use crate::srp::{SrpParams, hex_padded};
use num_bigint::BigUint;

fn hash_str(p: &SrpParams, text: &str) -> BigUint {
    BigUint::from_bytes_be(&p.hash.digest(text.as_bytes()))
}

fn hash_str_hex(p: &SrpParams, text: &str) -> String {
    p.hash
        .digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// x = H( uppercase( salt_string | hex(H(identity ":" password)) ) )
///
/// The salt is the string as received or as generated at registration. It is
/// deliberately not normalised through an integer: in this variant a salt is an
/// opaque string, and re-serialising one that starts with a zero byte would
/// change x.
pub fn compute_x(p: &SrpParams, salt: &str, identity: &str, password: &str) -> BigUint {
    let inner = hash_str_hex(p, &format!("{identity}:{password}"));
    hash_str(p, &format!("{salt}{inner}").to_uppercase())
}

/// u = H( hex(A) | b_string ). `b_str` is the server's value verbatim.
pub fn compute_u(p: &SrpParams, a_pub_hex: &str, b_str: &str) -> BigUint {
    hash_str(p, &format!("{a_pub_hex}{b_str}"))
}

/// M1 = H( hex(A) | b_string | hex(S) )
pub fn compute_m1(p: &SrpParams, a_pub_hex: &str, b_str: &str, s_hex: &str) -> BigUint {
    hash_str(p, &format!("{a_pub_hex}{b_str}{s_hex}"))
}

/// M2 = H( hex(A) | hex(M1) padded to the digest width | hex(S) )
///
/// The padding is the one fixed-width encoding in this variant. It matters
/// whenever M1's leading nibble is zero.
pub fn compute_m2(p: &SrpParams, a_pub_hex: &str, m1: &BigUint, s_hex: &str) -> BigUint {
    let m1_hex = hex_padded(m1, p.hash.hex_width());
    hash_str(p, &format!("{a_pub_hex}{m1_hex}{s_hex}"))
}

/// The derived session key, as lowercase hex of H(hex(S)).
pub fn session_key(p: &SrpParams, s_hex: &str) -> String {
    hash_str_hex(p, s_hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srp::{HashAlg, RFC5054_1024_PRIME_HEX, SrpParams, Variant, hex};

    const SALT: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
    const IDENTITY: &str = "user@example.test";
    const PASSWORD: &str = "correct horse battery staple";

    const A_PUB: &str = "afc09d0e8064973db60bf3ae47e25e908557d52b694a7017bde933ba5805bfc5d09eb35be03567c21b1e11831117e57724645fe15632be462702d13ac571ad9e7291cee017c5caf720c98e9f268169787236a7a4cc1fa7eb114a252b353e0cf8d26eef8df718a8ed76577c3aace096c6b3c38e3cdd7f3ad107164eb87e807bfd";
    const B_STR: &str = "ee9f2db217673d527389de972634c74b6a1eff01caf11b03d3b42559958189e839350fe5dde116bc79fc7f41a1728da87f9740a81c559a7812f1e8b6be4192c0a4a33546483f1059c8f976757f53ff14e2b05810864b4836bf831ab99a1b11863ff2fff3a7bd0393d56c280a4a4c2b752f72d190b5b64c740f5bd8acbdf03b5c";
    const S_HEX: &str = "913471403f02102b47833c0483e9e096dcd8e2c087351992acf9721a0e13354d3a638400fa754109e3e5539f8f8d6ab6cbf0c64e062f200b47dd735a24d121a9fd033fbe9953c7f4e3fa139f3080c072c07a25a254989b5d41bfa0f1bd42c7c9c05c1c801f9142818ccff784b5c8d3cd6fcf0c7c3d8968a45d75da7d43e0e5f4";

    fn params() -> SrpParams {
        SrpParams {
            variant: Variant::HexString,
            prime: num_bigint::BigUint::parse_bytes(RFC5054_1024_PRIME_HEX.as_bytes(), 16)
                .expect("valid prime"),
            generator: num_bigint::BigUint::from(2u32),
            hash: HashAlg::Sha256,
        }
    }

    /// The salt is hashed as the string it was given, leading zero included.
    /// Parsing it into an integer first would drop that zero and change x.
    #[test]
    fn x_hashes_the_salt_string_verbatim() {
        assert_eq!(
            hex(&compute_x(&params(), SALT, IDENTITY, PASSWORD)),
            "e58fd6ff10b6704c9f1194b98c90b557ad7de68fa2399f3622e77873acc11589"
        );
    }

    #[test]
    fn u_hashes_the_two_hex_strings() {
        assert_eq!(
            hex(&compute_u(&params(), A_PUB, B_STR)),
            "a4323f8727676fbde7a0371a7cd44ff0b285e747f8a9f5ebd34a4c94f5ef88ba"
        );
    }

    #[test]
    fn m1_hashes_a_b_and_s() {
        assert_eq!(
            hex(&compute_m1(&params(), A_PUB, B_STR, S_HEX)),
            "e78fe1c4f3e4f430bc068aa70565c0cedb27ec7558522019845ea9c90a0b997"
        );
    }

    /// M1 enters M2 zero-padded to the digest width. This vector was chosen so
    /// that M1 starts with a zero nibble: an implementation that strips it,
    /// as some browser clients do, produces a different M2 and fails here.
    #[test]
    fn m2_pads_m1_to_the_digest_width() {
        let m1 = compute_m1(&params(), A_PUB, B_STR, S_HEX);
        assert_eq!(
            hex(&compute_m2(&params(), A_PUB, &m1, S_HEX)),
            "1b8e7efaf3c35c057d67ba1a620b735018a59d1a496ae093655d38d007557a2e"
        );
    }

    #[test]
    fn session_key_is_the_hash_of_the_hex_of_s() {
        assert_eq!(
            session_key(&params(), S_HEX),
            "d98140c85492caa3c6536a1abd7ea09696097e3056f65b8628c9a6b90b3e01e1"
        );
    }
}
