//! Compact JWS create / extract / verify (`ES256`, `ES256K`, `EdDSA`).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{
    Signature as EdSignature, SigningKey as EdSigningKey, Verifier as EdVerifier,
    VerifyingKey as EdVk,
};
use k256::ecdsa::signature::Signer as K256Signer;
use k256::ecdsa::{
    Signature as K256Signature, SigningKey as K256SigningKey, VerifyingKey as K256Vk,
};
use p256::ecdsa::{
    Signature as P256Signature, SigningKey as P256SigningKey, VerifyingKey as P256Vk,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::canonicalize::canonicalize_serialize;
use super::did::public_key_from_kid;
use super::{OfferReceiptError, SignedOffer, SignedReceipt};

/// JWS compact header (`alg` + `kid`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JwsHeader {
    /// JWS algorithm.
    pub alg: String,
    /// Key identifier DID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
}

/// Public key material for JWS verification.
#[derive(Debug, Clone, Copy)]
pub enum JwsPublicKey {
    /// P-256 (`ES256`).
    Es256(P256Vk),
    /// secp256k1 (`ES256K`).
    Es256k(K256Vk),
    /// Ed25519 (`EdDSA`).
    Eddsa(EdVk),
}

/// Pluggable JWS signer. `sign` returns base64url (no pad) signature bytes.
pub trait JwsSigner: Send + Sync {
    /// Key identifier DID.
    fn kid(&self) -> &str;
    /// JWS `alg` (`ES256`, `ES256K`, `EdDSA`).
    fn algorithm(&self) -> &'static str;
    /// Sign the JWS signing input (`header.payload` ASCII).
    ///
    /// # Errors
    ///
    /// [`OfferReceiptError::Sign`].
    fn sign(&self, signing_input: &[u8]) -> Result<String, OfferReceiptError>;
}

/// ES256 (P-256) JWS signer.
#[derive(Clone)]
pub struct Es256JwsSigner {
    kid: String,
    signing_key: P256SigningKey,
}

impl std::fmt::Debug for Es256JwsSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Es256JwsSigner")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl Es256JwsSigner {
    /// Builds a signer from a 32-byte secret.
    ///
    /// # Errors
    ///
    /// Invalid scalar.
    pub fn from_bytes(kid: impl Into<String>, secret: &[u8]) -> Result<Self, OfferReceiptError> {
        let signing_key = P256SigningKey::from_slice(secret)
            .map_err(|e| OfferReceiptError::Sign(e.to_string()))?;
        Ok(Self {
            kid: kid.into(),
            signing_key,
        })
    }

    /// Corresponding verifying key.
    #[must_use]
    pub fn verifying_key(&self) -> JwsPublicKey {
        JwsPublicKey::Es256(*self.signing_key.verifying_key())
    }
}

impl JwsSigner for Es256JwsSigner {
    fn kid(&self) -> &str {
        &self.kid
    }

    fn algorithm(&self) -> &'static str {
        "ES256"
    }

    fn sign(&self, signing_input: &[u8]) -> Result<String, OfferReceiptError> {
        use p256::ecdsa::signature::Signer;
        let sig: P256Signature = self.signing_key.sign(signing_input);
        Ok(b64url(sig.to_bytes().as_ref()))
    }
}

/// ES256K (secp256k1) JWS signer.
#[derive(Clone)]
pub struct Es256kJwsSigner {
    kid: String,
    signing_key: K256SigningKey,
}

impl std::fmt::Debug for Es256kJwsSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Es256kJwsSigner")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl Es256kJwsSigner {
    /// Builds a signer from a 32-byte secret.
    ///
    /// # Errors
    ///
    /// Invalid scalar.
    pub fn from_bytes(kid: impl Into<String>, secret: &[u8]) -> Result<Self, OfferReceiptError> {
        let signing_key = K256SigningKey::from_slice(secret)
            .map_err(|e| OfferReceiptError::Sign(e.to_string()))?;
        Ok(Self {
            kid: kid.into(),
            signing_key,
        })
    }

    /// Corresponding verifying key.
    #[must_use]
    pub fn verifying_key(&self) -> JwsPublicKey {
        JwsPublicKey::Es256k(*self.signing_key.verifying_key())
    }
}

impl JwsSigner for Es256kJwsSigner {
    fn kid(&self) -> &str {
        &self.kid
    }

    fn algorithm(&self) -> &'static str {
        "ES256K"
    }

    fn sign(&self, signing_input: &[u8]) -> Result<String, OfferReceiptError> {
        let sig: K256Signature = self.signing_key.sign(signing_input);
        Ok(b64url(sig.to_bytes().as_ref()))
    }
}

/// `EdDSA` (`Ed25519`) JWS signer.
#[derive(Clone)]
pub struct EddsaJwsSigner {
    kid: String,
    signing_key: EdSigningKey,
}

impl std::fmt::Debug for EddsaJwsSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EddsaJwsSigner")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl EddsaJwsSigner {
    /// Builds a signer from a 32-byte Ed25519 seed.
    ///
    /// # Errors
    ///
    /// Seed is not 32 bytes.
    pub fn from_bytes(kid: impl Into<String>, seed: &[u8]) -> Result<Self, OfferReceiptError> {
        let arr: [u8; 32] = seed
            .try_into()
            .map_err(|_| OfferReceiptError::Sign("Ed25519 seed must be 32 bytes".into()))?;
        Ok(Self {
            kid: kid.into(),
            signing_key: EdSigningKey::from_bytes(&arr),
        })
    }

    /// Corresponding verifying key.
    #[must_use]
    pub fn verifying_key(&self) -> JwsPublicKey {
        JwsPublicKey::Eddsa(self.signing_key.verifying_key())
    }
}

impl JwsSigner for EddsaJwsSigner {
    fn kid(&self) -> &str {
        &self.kid
    }

    fn algorithm(&self) -> &'static str {
        "EdDSA"
    }

    fn sign(&self, signing_input: &[u8]) -> Result<String, OfferReceiptError> {
        use ed25519_dalek::Signer;
        let sig = self.signing_key.sign(signing_input);
        Ok(b64url(&sig.to_bytes()))
    }
}

/// Create a JWS compact string over a canonicalized payload.
///
/// # Errors
///
/// Canonicalize or sign failure.
pub fn create_jws<T: Serialize>(
    payload: &T,
    signer: &dyn JwsSigner,
) -> Result<String, OfferReceiptError> {
    let header = serde_json::json!({
        "alg": signer.algorithm(),
        "kid": signer.kid(),
    });
    let header_b64 = b64url(serde_json::to_vec(&header)?.as_slice());
    let payload_b64 = b64url(canonicalize_serialize(payload)?.as_bytes());
    let signing_input = format!("{header_b64}.{payload_b64}");
    let sig_b64 = signer.sign(signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{sig_b64}"))
}

/// Extract JWS header without verification.
///
/// # Errors
///
/// [`OfferReceiptError::InvalidJws`].
pub fn extract_jws_header(jws: &str) -> Result<JwsHeader, OfferReceiptError> {
    let (header_b64, _, _) = split_jws(jws)?;
    let json = b64url_decode(header_b64)?;
    serde_json::from_slice(&json).map_err(OfferReceiptError::from)
}

/// Extract JWS payload without verification.
///
/// # Errors
///
/// [`OfferReceiptError::InvalidJws`].
pub fn extract_jws_payload<T: DeserializeOwned>(jws: &str) -> Result<T, OfferReceiptError> {
    let (_, payload_b64, _) = split_jws(jws)?;
    let json = b64url_decode(payload_b64)?;
    serde_json::from_slice(&json).map_err(OfferReceiptError::from)
}

/// Verify a compact JWS. When `key` is `None`, resolve from `kid`.
///
/// # Errors
///
/// Invalid JWS, missing key, or signature mismatch.
pub fn verify_jws<T: DeserializeOwned>(
    jws: &str,
    key: Option<&JwsPublicKey>,
) -> Result<T, OfferReceiptError> {
    let (header_b64, payload_b64, sig_b64) = split_jws(jws)?;
    let header = extract_jws_header(jws)?;
    let owned = if key.is_none() {
        let kid = header
            .kid
            .as_deref()
            .ok_or_else(|| OfferReceiptError::Verify("JWS header missing kid".into()))?;
        Some(public_key_from_kid(kid)?)
    } else {
        None
    };
    let key = key.or(owned.as_ref()).ok_or_else(|| {
        OfferReceiptError::Verify("No public key provided and JWS header missing kid".into())
    })?;
    let signing_input = format!("{header_b64}.{payload_b64}");
    let sig_bytes = b64url_decode(sig_b64)?;
    verify_sig(key, signing_input.as_bytes(), &sig_bytes)?;
    extract_jws_payload(jws)
}

/// Verify a JWS-signed offer.
///
/// # Errors
///
/// Format or signature failure.
pub fn verify_offer_signature_jws(
    offer: &SignedOffer,
    key: Option<&JwsPublicKey>,
) -> Result<super::OfferPayload, OfferReceiptError> {
    let SignedOffer::Jws { signature, .. } = offer else {
        return Err(OfferReceiptError::UnknownFormat("expected jws".into()));
    };
    verify_jws(signature, key)
}

/// Verify a JWS-signed receipt.
///
/// # Errors
///
/// Format or signature failure.
pub fn verify_receipt_signature_jws(
    receipt: &SignedReceipt,
    key: Option<&JwsPublicKey>,
) -> Result<super::ReceiptPayload, OfferReceiptError> {
    let SignedReceipt::Jws { signature } = receipt else {
        return Err(OfferReceiptError::UnknownFormat("expected jws".into()));
    };
    verify_jws(signature, key)
}

fn verify_sig(key: &JwsPublicKey, input: &[u8], sig: &[u8]) -> Result<(), OfferReceiptError> {
    use k256::ecdsa::signature::Verifier as K256Verify;
    use p256::ecdsa::signature::Verifier as P256Verify;
    match key {
        JwsPublicKey::Es256(vk) => {
            let sig = P256Signature::from_slice(sig)
                .map_err(|e| OfferReceiptError::Verify(e.to_string()))?;
            P256Verify::verify(vk, input, &sig)
                .map_err(|e| OfferReceiptError::Verify(e.to_string()))
        }
        JwsPublicKey::Es256k(vk) => {
            let sig = K256Signature::from_slice(sig)
                .map_err(|e| OfferReceiptError::Verify(e.to_string()))?;
            K256Verify::verify(vk, input, &sig)
                .map_err(|e| OfferReceiptError::Verify(e.to_string()))
        }
        JwsPublicKey::Eddsa(vk) => {
            let sig = EdSignature::from_slice(sig)
                .map_err(|e| OfferReceiptError::Verify(e.to_string()))?;
            vk.verify(input, &sig)
                .map_err(|e| OfferReceiptError::Verify(e.to_string()))
        }
    }
}

fn split_jws(jws: &str) -> Result<(&str, &str, &str), OfferReceiptError> {
    let mut parts = jws.split('.');
    let header = parts.next().ok_or(OfferReceiptError::InvalidJws)?;
    let payload = parts.next().ok_or(OfferReceiptError::InvalidJws)?;
    let sig = parts.next().ok_or(OfferReceiptError::InvalidJws)?;
    if parts.next().is_some() || header.is_empty() || payload.is_empty() || sig.is_empty() {
        return Err(OfferReceiptError::InvalidJws);
    }
    Ok((header, payload, sig))
}

pub(crate) fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn b64url_decode(s: &str) -> Result<Vec<u8>, OfferReceiptError> {
    URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|_| OfferReceiptError::InvalidJws)
}
