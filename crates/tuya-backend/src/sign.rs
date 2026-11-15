//! The signing and sealing scheme of the Tuya device-sharing API.
//!
//! Ported from Alegzandr/lueur `crates/lueur-shell/src/tuya_cloud.rs`,
//! commit be7ba9b14be8fc1031ee962c9069c744e88e8252, MIT; Copyright (c) 2026
//! Lueur contributors. See `LICENSE-tuya` in this crate.
//!
//! Every request carries five `X-*` headers; four of them and the two sealed
//! payloads are signed:
//!
//! - `hash_key = md5_hex(rid + refresh_token)`
//! - `aes_secret = first 16 ASCII chars of hmac_sha256_hex(key = rid, msg = hash_key)`
//! - `to_sign` = non-empty headers as `k=v` joined by `||`, then the sealed
//!   query and the sealed body appended with no separator
//! - `sign = hmac_sha256_hex(key = hash_key, msg = to_sign)`
//!
//! Sealing is AES-128-GCM under `aes_secret`; the nonce is 12 printable
//! characters. The SDK builds the sealed string as `b64(nonce) +
//! b64(ciphertext ‖ tag)` and opens it as `b64(nonce ‖ ciphertext ‖ tag)`.
//! Because 12 is a multiple of 3, the two base64 strings are byte-identical,
//! so one [`seal`] and one [`open`] serve both ends; [`open`] still splits
//! the decoded bytes at 12 so a nonce of any length is handled the way the
//! SDK handles it.
//!
//! The sealing key is 16 bytes, so it is AES-128-GCM, not AES-256-GCM.
//!
//! Everything here is a pure function taking the request id, the nonce and
//! the time as arguments, so the tests can replay the Python SDK's vectors.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use sha2::Sha256;

/// `hmac`'s `KeyInit` is digest 0.10's; `aes-gcm`'s `KeyInit` is digest
/// 0.11's under the same name. HMAC here is the digest 0.10 one.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as hmac::digest::KeyInit>::new_from_slice(key)
        .expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

/// The header names that go into the signature, in this order. Empty values
/// are still sent but are left out of `to_sign`.
pub const SIGNED_HEADERS: [&str; 5] = ["X-appKey", "X-requestId", "X-sid", "X-time", "X-token"];

/// The 48-character alphabet the SDK draws its nonce from. A 12-character
/// nonce is drawn from it, so `ABCDEFGHJKMN` is a legal one.
const NONCE_CHARS: &[u8] = b"ABCDEFGHJKMNPQRSTWXYZabcdefhijkmnprstwxyz2345678";

/// Lower-case hex, the form every digest in this scheme is used in.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// `hex(md5(rid + refresh_token))`, the HMAC key of the signature.
pub fn hash_key(rid: &str, refresh_token: &str) -> String {
    let mut md5 = Md5::new();
    md5.update(rid.as_bytes());
    md5.update(refresh_token.as_bytes());
    hex(&md5.finalize())
}

/// The 16-character sealing key: the first 16 hex characters of
/// `hmac_sha256(key = rid, msg = hash_key)`.
///
/// The SDK appends `_` plus a sid-derived string when `sid` is non-empty.
/// The sharing API always sends an empty sid, so that branch is not ported:
/// [`sign`] skips the empty header and the sealing key is unaffected.
pub fn aes_secret(rid: &str, hash_key: &str) -> String {
    hex(&hmac_sha256(rid.as_bytes(), hash_key.as_bytes()))[..16].to_string()
}

/// A 12-character nonce from the SDK's alphabet, from OS randomness.
pub fn random_nonce() -> Result<String, SignError> {
    let mut raw = [0u8; 12];
    getrandom::fill(&mut raw).map_err(|e| SignError::Random(e.to_string()))?;
    Ok(raw
        .iter()
        .map(|b| NONCE_CHARS[usize::from(*b) % NONCE_CHARS.len()] as char)
        .collect())
}

/// Seal `plain` under `secret` with `nonce`, in the SDK's wire form:
/// `b64(nonce) + b64(ciphertext ‖ tag)`.
pub fn seal(plain: &str, secret: &str, nonce: &str) -> Result<String, SignError> {
    let cipher = cipher(secret)?;
    let nonce_arr = nonce_array(nonce)?;
    let sealed = cipher
        .encrypt(&nonce_arr, plain.as_bytes())
        .map_err(|_| SignError::Seal)?;
    Ok(format!(
        "{}{}",
        B64.encode(nonce.as_bytes()),
        B64.encode(sealed)
    ))
}

/// Open a sealed value: split the decoded bytes at 12 (the nonce), decrypt
/// the rest, and read the tag. This accepts both the string [`seal`]
/// returns and a `b64(nonce ‖ ciphertext ‖ tag)` answer, because with a
/// 12-byte nonce they are the same base64 text.
pub fn open(sealed: &str, secret: &str) -> Result<String, SignError> {
    let raw = B64.decode(sealed).map_err(|_| SignError::Open("base64"))?;
    if raw.len() < 12 {
        return Err(SignError::Open("short"));
    }
    let (nonce, body) = raw.split_at(12);
    let cipher = cipher(secret)?;
    let nonce_arr = nonce_array_bytes(nonce)?;
    let plain = cipher
        .decrypt(&nonce_arr, body)
        .map_err(|_| SignError::Open("seal"))?;
    String::from_utf8(plain).map_err(|_| SignError::Open("utf-8"))
}

fn cipher(secret: &str) -> Result<Aes128Gcm, SignError> {
    Aes128Gcm::new_from_slice(secret.as_bytes()).map_err(|_| SignError::Seal)
}

fn nonce_array(nonce: &str) -> Result<Nonce<aes_gcm::aead::consts::U12>, SignError> {
    nonce_array_bytes(nonce.as_bytes())
}

fn nonce_array_bytes(bytes: &[u8]) -> Result<Nonce<aes_gcm::aead::consts::U12>, SignError> {
    Nonce::<aes_gcm::aead::consts::U12>::try_from(bytes).map_err(|_| SignError::Seal)
}

/// Sign one request. `headers` must be the five [`SIGNED_HEADERS`] in order;
/// an empty value is skipped, matching the SDK. `query_encdata` and
/// `body_encdata` are appended directly with no separator.
pub fn sign(
    hash_key: &str,
    headers: &[(&str, &str)],
    query_encdata: &str,
    body_encdata: &str,
) -> String {
    let mut to_sign = headers
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("||");
    to_sign.push_str(query_encdata);
    to_sign.push_str(body_encdata);
    hex(&hmac_sha256(hash_key.as_bytes(), to_sign.as_bytes()))
}

/// The compact JSON the SDK seals: `json.dumps(..., separators=(",", ":"))`
/// on an object, which is what `serde_json::to_string` produces.
pub fn compact_json(value: &serde_json::Value) -> String {
    value.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignError {
    Seal,
    Open(&'static str),
    Random(String),
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignError::Seal => write!(f, "sealing the request failed"),
            SignError::Open(what) => write!(f, "unreadable sealed answer ({what})"),
            SignError::Random(e) => write!(f, "no randomness for the nonce: {e}"),
        }
    }
}

impl std::error::Error for SignError {}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// The vectors the Python SDK produced in ticket 01 (see
    /// `.scratch/smarthome-v1/fixtures/tuya-signing-vectors.json`). Every
    /// value here is synthetic; the real account never appears.
    const VECTORS: &str =
        include_str!("../../../.scratch/smarthome-v1/fixtures/tuya-signing-vectors.json");

    fn vectors() -> Value {
        serde_json::from_str(VECTORS).expect("the signing fixture is JSON")
    }

    fn input<'a>(v: &'a Value, key: &str) -> &'a str {
        v["inputs"][key].as_str().expect("a string input")
    }

    #[test]
    fn the_fixture_is_the_one_ticket_01_produced() {
        let v = vectors();
        assert_eq!(input(&v, "client_id"), "HA_3y9q4ak7g4ephrvke");
        assert_eq!(input(&v, "fixed_nonce"), "ABCDEFGHJKMN");
        assert_eq!(
            v["aes_secret_hmac_sha256_first16hex"].as_str(),
            Some("162723c1c5eb7c79")
        );
    }

    #[test]
    fn every_vector_matches_byte_for_byte() {
        let v = vectors();
        let rid = input(&v, "rid");
        let refresh = input(&v, "refresh_token");
        let nonce = input(&v, "fixed_nonce");
        let sid = input(&v, "sid");
        let headers = [
            ("X-appKey", input(&v, "client_id")),
            ("X-requestId", rid),
            ("X-sid", sid),
            ("X-time", input(&v, "x_time")),
            ("X-token", input(&v, "access_token")),
        ];

        // hash_key and the sealing key.
        let hash_key = hash_key(rid, refresh);
        assert_eq!(hash_key, v["hash_key_md5_rid_plus_refresh"]);
        let secret = aes_secret(rid, &hash_key);
        assert_eq!(secret, v["aes_secret_hmac_sha256_first16hex"]);

        // Both sealed payloads, from the fixed nonce and the fixed secret.
        let query_enc = seal(input(&v, "query_json"), &secret, nonce).unwrap();
        assert_eq!(query_enc, v["query_encdata"]);
        let body_enc = seal(input(&v, "body_json"), &secret, nonce).unwrap();
        assert_eq!(body_enc, v["body_encdata"]);

        // All three signatures.
        assert_eq!(
            sign(&hash_key, &headers, &query_enc, ""),
            v["sign_get_with_query"]
        );
        assert_eq!(
            sign(&hash_key, &headers, "", &body_enc),
            v["sign_post_with_body"]
        );
        let without_token: Vec<(&str, &str)> = headers
            .iter()
            .copied()
            .filter(|(k, _)| *k != "X-token")
            .collect();
        assert_eq!(
            sign(&hash_key, &without_token, "", ""),
            v["sign_no_token_no_payload"]
        );

        // And the body opens to the plaintext the SDK decrypted.
        let opened = open(&body_enc, &secret).unwrap();
        assert_eq!(opened, v["decrypt_check"]);
        assert_eq!(opened, input(&v, "body_json"));
    }

    #[test]
    fn seal_then_open_round_trips() {
        let secret = "162723c1c5eb7c79";
        let sealed = seal(r#"{"homeId":"123"}"#, secret, "ABCDEFGHJKMN").unwrap();
        assert!(sealed.starts_with("QUJDREVGR0hKS01O"));
        assert_eq!(open(&sealed, secret).unwrap(), r#"{"homeId":"123"}"#);
        // The nonce is the first 16 base64 characters, as the SDK writes it.
        assert_eq!(B64.decode(&sealed[..16]).unwrap(), b"ABCDEFGHJKMN");
    }

    #[test]
    fn a_tampered_seal_is_refused() {
        let secret = "162723c1c5eb7c79";
        let sealed = seal("hello", secret, "ABCDEFGHJKMN").unwrap();
        let mut bytes = B64.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert_eq!(
            open(&B64.encode(bytes), secret),
            Err(SignError::Open("seal"))
        );
    }

    #[test]
    fn an_empty_header_does_not_reach_the_signature() {
        let hk = hash_key("rid", "refresh");
        let with_empty = sign(
            &hk,
            &[("X-appKey", "a"), ("X-sid", ""), ("X-time", "1")],
            "",
            "",
        );
        let without = sign(&hk, &[("X-appKey", "a"), ("X-time", "1")], "", "");
        assert_eq!(with_empty, without);
    }

    #[test]
    fn nonces_are_legal_and_unique() {
        let a = random_nonce().unwrap();
        let b = random_nonce().unwrap();
        assert_eq!(a.len(), 12);
        assert!(a.bytes().all(|b| NONCE_CHARS.contains(&b)));
        assert_ne!(a, b);
    }

    #[test]
    fn compact_json_has_no_spaces() {
        let value = serde_json::json!({"a": 1, "b": [true, "x"]});
        assert_eq!(compact_json(&value), r#"{"a":1,"b":[true,"x"]}"#);
    }
}
