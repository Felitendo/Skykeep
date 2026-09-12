//! SRP-6a as Apple's Apple ID sign-in uses it.
//!
//! Apple migrated Apple ID auth to SRP; the old cleartext `POST /appleauth/auth/signin`
//! now returns 503. This is the client half of `/signin/init` + `/signin/complete`.
//!
//! Apple's specifics, all of which matter and none of which are the textbook default:
//!   * SHA-256 over the RFC 5054 2048-bit group.
//!   * RFC 5054 padding: `u` and `k` hash their operands left-padded to len(N).
//!   * The username is OMITTED from `x` (`x = H(s || H(":" || p))`).
//!   * `p` is not the password. It is PBKDF2-HMAC-SHA256 over SHA-256(password), where the
//!     `s2k_fo` protocol feeds the LOWERCASE HEX of that digest instead of its raw bytes.
//!
//! Correctness note: the reference `srp` Python package has two backends, and they disagree.
//! Its OpenSSL backend (`_ctsrp`, the one that actually runs) round-trips the salt through a
//! BIGNUM, which strips leading zero bytes and corrupts M1 for roughly 1 salt in 256. SRP-6a
//! treats the salt as an opaque byte string, so we hash it raw. `srp_vectors.json` pins both
//! the agreed values and that edge case, asserting the spec-correct answer.

use hmac::Hmac;
use num_bigint::{BigInt, BigUint, Sign};
use sha2::{Digest, Sha256};

/// RFC 5054 2048-bit group.
const N_HEX: &str = "AC6BDB41324A9A9BF166DE5E1389582FAF72B6651987EE07FC3192943DB56050\
A37329CBB4A099ED8193E0757767A13DD52312AB4B03310DCD7F48A9DA04FD50\
E8083969EDB767B0CF6095179A163AB3661A05FBD5FAAAE82918A9962F0B93B8\
55F97993EC975EEAA80D740ADBF4FF747359D041D5C33EA71D281E446B14773B\
CA97B43A23FB801676BD207A436C6481F1D2B9078717461A5B9D32E688F87748\
544523B524B0D57D5EA77A2775D2ECFA032CFBDBF52FB3786160279004E57AE6\
AF874E7303CE53299CCC041C7BC308D82A5698F3A8D0C38271AE35F8E9DBFBB6\
94B5C803D89F7AE435DE236D525F54759B65E372FCD68EF20FA7111F9E4AFF73";
const G: u32 = 2;

fn n() -> BigUint {
    BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("static group modulus is valid")
}

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// Minimal big-endian bytes, matching Python's `long_to_bytes`.
fn be(v: &BigUint) -> Vec<u8> {
    v.to_bytes_be()
}

/// Left-pad to `width` bytes — RFC 5054's PAD().
fn pad(bytes: &[u8], width: usize) -> Vec<u8> {
    let mut out = vec![0u8; width.saturating_sub(bytes.len())];
    out.extend_from_slice(bytes);
    out
}

/// Which key-derivation variant the server asked for in `/signin/init`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// Feed the raw SHA-256 digest of the password into PBKDF2.
    S2k,
    /// Feed the lowercase hex of that digest instead.
    S2kFo,
}

impl Protocol {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "s2k" => Some(Self::S2k),
            "s2k_fo" => Some(Self::S2kFo),
            _ => None,
        }
    }
}

/// Apple's password pre-hash: PBKDF2 over a SHA-256 digest of the password.
pub fn derive_key(password: &str, protocol: Protocol, salt: &[u8], iterations: u32) -> [u8; 32] {
    let digest = Sha256::digest(password.as_bytes());
    let input: Vec<u8> = match protocol {
        Protocol::S2k => digest.to_vec(),
        Protocol::S2kFo => hex_lower(&digest).into_bytes(),
    };
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2::<Hmac<Sha256>>(&input, salt, iterations, &mut out)
        .expect("pbkdf2 with a 32-byte output never fails");
    out
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A client-side SRP exchange. Hold between `/signin/init` and `/signin/complete`.
pub struct Client {
    username: String,
    a: BigUint,
    pub a_pub: BigUint,
}

/// Proof values for `/signin/complete`.
pub struct Proof {
    pub m1: [u8; 32],
    pub m2: [u8; 32],
}

impl Client {
    /// Start with a caller-supplied private key. Test/vector use.
    pub fn with_private_key(username: &str, a_bytes: &[u8]) -> Self {
        let n = n();
        let a = BigUint::from_bytes_be(a_bytes);
        let a_pub = BigUint::from(G).modpow(&a, &n);
        Self {
            username: username.to_string(),
            a,
            a_pub,
        }
    }

    /// Start with a fresh random private key. Production use.
    pub fn new(username: &str) -> Self {
        use rand::RngCore;
        let mut a = [0u8; 32];
        rand::rng().fill_bytes(&mut a);
        Self::with_private_key(username, &a)
    }

    /// Base64-ready public key `A` for `/signin/init`.
    pub fn a_public_bytes(&self) -> Vec<u8> {
        be(&self.a_pub)
    }

    /// Finish the exchange against the server's `salt` and `B`, producing M1/M2.
    ///
    /// Returns `None` if the SRP-6a safety checks fail (`B % N == 0`, or `u == 0`),
    /// which means the server's challenge is unusable and auth must be abandoned.
    pub fn process_challenge(
        &self,
        password: &str,
        protocol: Protocol,
        salt: &[u8],
        b_pub_bytes: &[u8],
        iterations: u32,
    ) -> Option<Proof> {
        let n = n();
        let g = BigUint::from(G);
        let width = be(&n).len();

        let b_pub = BigUint::from_bytes_be(b_pub_bytes);
        if (&b_pub % &n) == BigUint::from(0u32) {
            return None; // SRP-6a safety check
        }

        // k = H(PAD(N) || PAD(g))
        let k = BigUint::from_bytes_be(&sha256(&[&pad(&be(&n), width), &pad(&be(&g), width)]));

        // u = H(PAD(A) || PAD(B))
        let u = BigUint::from_bytes_be(&sha256(&[
            &pad(&be(&self.a_pub), width),
            &pad(&be(&b_pub), width),
        ]));
        if u == BigUint::from(0u32) {
            return None; // SRP-6a safety check
        }

        // x = H(salt || H(":" || derived_key)) — username omitted, salt hashed RAW.
        let derived = derive_key(password, protocol, salt, iterations);
        let inner = sha256(&[b":", &derived]);
        let x = BigUint::from_bytes_be(&sha256(&[salt, &inner]));

        // S = (B - k*v)^(a + u*x) mod N, with the base reduced into [0, N).
        let v = g.modpow(&x, &n);
        let base = {
            let diff = BigInt::from_biguint(Sign::Plus, b_pub.clone())
                - BigInt::from_biguint(Sign::Plus, &k * &v);
            let n_i = BigInt::from_biguint(Sign::Plus, n.clone());
            let reduced = ((diff % &n_i) + &n_i) % &n_i;
            reduced.to_biguint().expect("reduced value is non-negative")
        };
        let s_shared = base.modpow(&(&self.a + &u * &x), &n);
        let key = sha256(&[&be(&s_shared)]);

        // M1 = H( H(N) XOR H(PAD(g)) || H(I) || salt || A || B || K )
        let h_n = sha256(&[&be(&n)]);
        let h_g = sha256(&[&pad(&be(&g), width)]);
        let mut h_n_xor_g = [0u8; 32];
        for i in 0..32 {
            h_n_xor_g[i] = h_n[i] ^ h_g[i];
        }
        let h_i = sha256(&[self.username.as_bytes()]);
        let m1 = sha256(&[&h_n_xor_g, &h_i, salt, &be(&self.a_pub), &be(&b_pub), &key]);

        // M2 = H(A || M1 || K)
        let m2 = sha256(&[&be(&self.a_pub), &m1, &key]);

        Some(Proof { m1, m2 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../testdata/srp_vectors.json"))
            .expect("vectors are valid JSON")
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Every value must match srp==1.0.22 — the library icloudpd pins — on inputs where
    /// both of its backends agree.
    #[test]
    fn matches_reference_implementation() {
        let v = vectors();
        let username = v["username"].as_str().unwrap();
        let password = v["password"].as_str().unwrap();
        let iterations = v["iterations"].as_u64().unwrap() as u32;
        let a = hex(v["a_hex"].as_str().unwrap());
        let b_pub = hex(v["B_hex"].as_str().unwrap());

        for (name, proto) in [("s2k", Protocol::S2k), ("s2k_fo", Protocol::S2kFo)] {
            let case = &v["cases"][name];
            let salt = hex(case["salt_hex"].as_str().unwrap());

            let derived = derive_key(password, proto, &salt, iterations);
            assert_eq!(
                hex_lower(&derived),
                case["derived_key"].as_str().unwrap(),
                "{name}: derived key"
            );

            let client = Client::with_private_key(username, &a);
            assert_eq!(
                hex_lower(&client.a_public_bytes()),
                case["A"].as_str().unwrap(),
                "{name}: public key A"
            );

            let p = client
                .process_challenge(password, proto, &salt, &b_pub, iterations)
                .expect("challenge is valid");
            assert_eq!(hex_lower(&p.m1), case["M1"].as_str().unwrap(), "{name}: M1");
            assert_eq!(hex_lower(&p.m2), case["M2"].as_str().unwrap(), "{name}: M2");
        }
    }

    /// A salt beginning with 0x00 is where the reference library's OpenSSL backend goes wrong:
    /// it round-trips the salt through a BIGNUM and loses the leading byte. We must produce the
    /// spec-correct M1, and specifically must NOT reproduce that bug.
    #[test]
    fn hashes_salt_as_raw_bytes_not_bignum() {
        let v = vectors();
        let case = &v["leading_zero_salt"];
        let salt = hex(case["salt_hex"].as_str().unwrap());
        assert_eq!(
            salt[0], 0x00,
            "this vector only means something with a leading zero salt"
        );

        let client = Client::with_private_key(
            v["username"].as_str().unwrap(),
            &hex(v["a_hex"].as_str().unwrap()),
        );
        let p = client
            .process_challenge(
                v["password"].as_str().unwrap(),
                Protocol::S2k,
                &salt,
                &hex(v["B_hex"].as_str().unwrap()),
                v["iterations"].as_u64().unwrap() as u32,
            )
            .expect("challenge is valid");

        assert_eq!(
            hex_lower(&p.m1),
            case["M1"].as_str().unwrap(),
            "spec-correct M1"
        );
        assert_ne!(
            hex_lower(&p.m1),
            case["ctsrp_buggy_M1"].as_str().unwrap(),
            "must not reproduce the reference implementation's BIGNUM salt bug"
        );
    }

    #[test]
    fn rejects_degenerate_server_key() {
        let client = Client::with_private_key("someone@example.invalid", &[7u8; 32]);
        // B == N  =>  B % N == 0  =>  SRP-6a safety check must reject.
        let b = hex(N_HEX);
        assert!(client
            .process_challenge("pw", Protocol::S2k, b"salt", &b, 1000)
            .is_none());
    }
}
