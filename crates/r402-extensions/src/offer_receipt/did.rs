//! DID key extraction for JWS `kid` (`did:key`, `did:jwk`).

use ed25519_dalek::VerifyingKey as EdVk;
use k256::ecdsa::VerifyingKey as K256Vk;
use p256::ecdsa::VerifyingKey as P256Vk;

use super::OfferReceiptError;
use super::jws::{JwsPublicKey, b64url_decode};

const MULTICODEC_ED25519_PUB: u32 = 0xed;
const MULTICODEC_SECP256K1_PUB: u32 = 0xe7;
const MULTICODEC_P256_PUB: u32 = 0x1200;

/// Extract a verifying key from `did:key` or `did:jwk`.
///
/// # Errors
///
/// Unsupported method, malformed DID, or unknown key type.
pub fn public_key_from_kid(kid: &str) -> Result<JwsPublicKey, OfferReceiptError> {
    let did_part = kid.split('#').next().unwrap_or(kid);
    let mut parts = did_part.splitn(3, ':');
    let scheme = parts.next().unwrap_or("");
    let method = parts.next().unwrap_or("");
    let identifier = parts.next().unwrap_or("");
    if scheme != "did" || method.is_empty() || identifier.is_empty() {
        return Err(OfferReceiptError::InvalidDid(kid.to_owned()));
    }
    match method {
        "key" => key_from_did_key(identifier),
        "jwk" => key_from_did_jwk(identifier),
        other => Err(OfferReceiptError::UnsupportedDid(other.to_owned())),
    }
}

fn key_from_did_key(identifier: &str) -> Result<JwsPublicKey, OfferReceiptError> {
    if !identifier.starts_with('z') {
        return Err(OfferReceiptError::InvalidDid(
            "did:key must use base58-btc ('z')".into(),
        ));
    }
    let decoded = bs58::decode(&identifier[1..])
        .into_vec()
        .map_err(|e| OfferReceiptError::InvalidDid(e.to_string()))?;
    let (codec, key_bytes) = read_multicodec(&decoded)?;
    match codec {
        MULTICODEC_ED25519_PUB => {
            let arr: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| OfferReceiptError::InvalidDid("Ed25519 key length".into()))?;
            let vk =
                EdVk::from_bytes(&arr).map_err(|e| OfferReceiptError::InvalidDid(e.to_string()))?;
            Ok(JwsPublicKey::Eddsa(vk))
        }
        MULTICODEC_SECP256K1_PUB => {
            let vk = K256Vk::from_sec1_bytes(key_bytes)
                .map_err(|e| OfferReceiptError::InvalidDid(e.to_string()))?;
            Ok(JwsPublicKey::Es256k(vk))
        }
        MULTICODEC_P256_PUB => {
            let vk = P256Vk::from_sec1_bytes(key_bytes)
                .map_err(|e| OfferReceiptError::InvalidDid(e.to_string()))?;
            Ok(JwsPublicKey::Es256(vk))
        }
        other => Err(OfferReceiptError::InvalidDid(format!(
            "unsupported did:key multicodec 0x{other:x}"
        ))),
    }
}

fn key_from_did_jwk(identifier: &str) -> Result<JwsPublicKey, OfferReceiptError> {
    let json = b64url_decode(identifier)?;
    let jwk: serde_json::Value = serde_json::from_slice(&json)?;
    jwk_to_key(&jwk)
}

fn jwk_to_key(jwk: &serde_json::Value) -> Result<JwsPublicKey, OfferReceiptError> {
    let kty = jwk
        .get("kty")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    match kty {
        "OKP" => {
            let x = jwk
                .get("x")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| OfferReceiptError::InvalidDid("JWK missing x".into()))?;
            let bytes = b64url_decode(x)?;
            let arr: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| OfferReceiptError::InvalidDid("Ed25519 x length".into()))?;
            let vk =
                EdVk::from_bytes(&arr).map_err(|e| OfferReceiptError::InvalidDid(e.to_string()))?;
            Ok(JwsPublicKey::Eddsa(vk))
        }
        "EC" => {
            let crv = jwk
                .get("crv")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let x = b64url_decode(
                jwk.get("x")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| OfferReceiptError::InvalidDid("JWK missing x".into()))?,
            )?;
            let y = b64url_decode(
                jwk.get("y")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| OfferReceiptError::InvalidDid("JWK missing y".into()))?,
            )?;
            let mut uncompressed = Vec::with_capacity(1 + x.len() + y.len());
            uncompressed.push(0x04);
            uncompressed.extend_from_slice(&x);
            uncompressed.extend_from_slice(&y);
            match crv {
                "P-256" => {
                    let vk = P256Vk::from_sec1_bytes(&uncompressed)
                        .map_err(|e| OfferReceiptError::InvalidDid(e.to_string()))?;
                    Ok(JwsPublicKey::Es256(vk))
                }
                "secp256k1" => {
                    let vk = K256Vk::from_sec1_bytes(&uncompressed)
                        .map_err(|e| OfferReceiptError::InvalidDid(e.to_string()))?;
                    Ok(JwsPublicKey::Es256k(vk))
                }
                other => Err(OfferReceiptError::InvalidDid(format!(
                    "unsupported JWK crv {other}"
                ))),
            }
        }
        other => Err(OfferReceiptError::InvalidDid(format!(
            "unsupported JWK kty {other}"
        ))),
    }
}

fn read_multicodec(bytes: &[u8]) -> Result<(u32, &[u8]), OfferReceiptError> {
    let mut codec = 0u32;
    let mut shift = 0u32;
    for (offset, byte) in bytes.iter().enumerate() {
        codec |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((codec, bytes.get(offset + 1..).unwrap_or(&[])));
        }
        shift += 7;
        if shift > 21 {
            break;
        }
    }
    Err(OfferReceiptError::InvalidDid("invalid multicodec".into()))
}
