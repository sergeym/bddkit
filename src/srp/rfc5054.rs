//! Routines as specified in RFC 5054. All inputs are hashed as raw bytes, and
//! A, B and g are padded to the byte length of the prime where the RFC says so.

use crate::srp::SrpParams;
use num_bigint::BigUint;

/// x = H(salt | H(identity ":" password))
pub fn compute_x(p: &SrpParams, salt: &[u8], identity: &str, password: &str) -> BigUint {
    let inner = p.hash.digest(format!("{identity}:{password}").as_bytes());
    let mut input = salt.to_vec();
    input.extend_from_slice(&inner);
    BigUint::from_bytes_be(&p.hash.digest(&input))
}

/// u = H(PAD(A) | PAD(B))
pub fn compute_u(p: &SrpParams, a_pub: &BigUint, b_pub: &BigUint) -> BigUint {
    let mut input = p.pad(a_pub);
    input.extend_from_slice(&p.pad(b_pub));
    BigUint::from_bytes_be(&p.hash.digest(&input))
}

/// M1 = H(H(N) XOR H(PAD(g)) | H(identity) | salt | A | B | K)
pub fn compute_m1(
    p: &SrpParams,
    identity: &str,
    salt: &[u8],
    a_pub: &BigUint,
    b_pub: &BigUint,
    key: &[u8],
) -> BigUint {
    let hash_n = p.hash.digest(&p.pad(&p.prime));
    let hash_g = p.hash.digest(&p.pad(&p.generator));
    let mut input: Vec<u8> = hash_n
        .iter()
        .zip(hash_g.iter())
        .map(|(left, right)| left ^ right)
        .collect();
    input.extend_from_slice(&p.hash.digest(identity.as_bytes()));
    input.extend_from_slice(salt);
    input.extend_from_slice(&a_pub.to_bytes_be());
    input.extend_from_slice(&b_pub.to_bytes_be());
    input.extend_from_slice(key);
    BigUint::from_bytes_be(&p.hash.digest(&input))
}

/// M2 = H(A | M1 | K)
pub fn compute_m2(p: &SrpParams, a_pub: &BigUint, m1: &BigUint, key: &[u8]) -> BigUint {
    let mut input = a_pub.to_bytes_be();
    input.extend_from_slice(&m1.to_bytes_be());
    input.extend_from_slice(key);
    BigUint::from_bytes_be(&p.hash.digest(&input))
}

/// K = H(S)
pub fn session_key(p: &SrpParams, s: &BigUint) -> Vec<u8> {
    p.hash.digest(&s.to_bytes_be())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srp::{HashAlg, RFC5054_1024_PRIME_HEX, Variant, hex};
    use num_bigint::BigUint;

    fn params() -> SrpParams {
        SrpParams {
            variant: Variant::Rfc5054,
            prime: BigUint::parse_bytes(RFC5054_1024_PRIME_HEX.as_bytes(), 16)
                .expect("valid prime"),
            generator: BigUint::from(2u32),
            hash: HashAlg::Sha1,
        }
    }

    fn salt() -> Vec<u8> {
        vec![
            0xBE, 0xB2, 0x53, 0x79, 0xD1, 0xA8, 0x58, 0x1E, 0xB5, 0xA7, 0x27, 0x67, 0x3A, 0x24,
            0x41, 0xEE,
        ]
    }

    #[test]
    fn x_matches_the_rfc_vector() {
        let x = compute_x(&params(), &salt(), "alice", "password123");
        assert_eq!(hex(&x), "94b7555aabe9127cc58ccf4993db6cf84d16c124");
    }

    #[test]
    fn verifier_matches_the_rfc_vector() {
        let p = params();
        let x = compute_x(&p, &salt(), "alice", "password123");
        assert_eq!(
            hex(&p.generator.modpow(&x, &p.prime)),
            "7e273de8696ffc4f4e337d05b4b375beb0dde1569e8fa00a9886d8129bada1f1822223ca1a605b530e379ba4729fdc59f105b4787e5186f5c671085a1447b52a48cf1970b4fb6f8400bbf4cebfbb168152e08ab5ea53d15c1aff87b2b9da6e04e058ad51cc72bfc9033b564e26480d78e955a5e29e7ab245db2be315e2099afb"
        );
    }

    #[test]
    fn u_matches_the_rfc_vector() {
        let p = params();
        let a_pub = BigUint::parse_bytes(b"61d5e490f6f1b79547b0704c436f523dd0e560f0c64115bb72557ec44352e8903211c04692272d8b2d1a5358a2cf1b6e0bfcf99f921530ec8e39356179eae45e42ba92aeaced825171e1e8b9af6d9c03e1327f44be087ef06530e69f66615261eef54073ca11cf5858f0edfdfe15efeab349ef5d76988a3672fac47b0769447b", 16).expect("valid hex");
        let b_pub = BigUint::parse_bytes(b"bd0c61512c692c0cb6d041fa01bb152d4916a1e77af46ae105393011baf38964dc46a0670dd125b95a981652236f99d9b681cbf87837ec996c6da04453728610d0c6ddb58b318885d7d82c7f8deb75ce7bd4fbaa37089e6f9c6059f388838e7a00030b331eb76840910440b1b27aaeaeeb4012b7d7665238a8e3fb004b117b58", 16).expect("valid hex");
        assert_eq!(
            hex(&compute_u(&p, &a_pub, &b_pub)),
            "ce38b9593487da98554ed47d70a7ae5f462ef019"
        );
    }
}
