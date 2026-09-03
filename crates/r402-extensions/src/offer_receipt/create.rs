//! Offer and receipt construction.

use std::time::{SystemTime, UNIX_EPOCH};

use super::eip712::{Eip712DigestSigner, sign_offer_eip712, sign_receipt_eip712};
use super::jws::{JwsSigner, create_jws, extract_jws_payload};
use super::{
    OfferInput, OfferPayload, OfferReceiptError, ReceiptInput, ReceiptPayload, SignedOffer,
    SignedReceipt,
};

/// Default offer validity in seconds (official `x402ResourceServer`).
pub const DEFAULT_OFFER_VALIDITY_SECONDS: u64 = 300;

/// Current offer/receipt payload schema version.
pub const EXTENSION_VERSION: u64 = 1;

fn unix_now() -> Result<u64, OfferReceiptError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| OfferReceiptError::Clock)
}

fn create_offer_payload(
    resource_url: &str,
    input: &OfferInput,
) -> Result<OfferPayload, OfferReceiptError> {
    let now = unix_now()?;
    let validity = input
        .offer_validity_seconds
        .unwrap_or(DEFAULT_OFFER_VALIDITY_SECONDS);
    Ok(OfferPayload {
        version: EXTENSION_VERSION,
        resource_url: resource_url.to_owned(),
        scheme: input.scheme.clone(),
        network: input.network.clone(),
        asset: input.asset.clone(),
        pay_to: input.pay_to.clone(),
        amount: input.amount.clone(),
        valid_until: now.saturating_add(validity),
    })
}

/// Create a JWS-signed offer.
///
/// # Errors
///
/// Clock or sign failure.
pub fn create_offer_jws(
    resource_url: &str,
    input: &OfferInput,
    signer: &dyn JwsSigner,
) -> Result<SignedOffer, OfferReceiptError> {
    let payload = create_offer_payload(resource_url, input)?;
    let jws = create_jws(&payload, signer)?;
    Ok(SignedOffer::Jws {
        accept_index: Some(input.accept_index),
        signature: jws,
    })
}

/// Create an EIP-712-signed offer.
///
/// # Errors
///
/// Clock or sign failure.
pub fn create_offer_eip712(
    resource_url: &str,
    input: &OfferInput,
    signer: &dyn Eip712DigestSigner,
) -> Result<SignedOffer, OfferReceiptError> {
    let payload = create_offer_payload(resource_url, input)?;
    let signature = sign_offer_eip712(&payload, signer)?;
    Ok(SignedOffer::Eip712 {
        accept_index: Some(input.accept_index),
        payload,
        signature,
    })
}

/// Extract the offer payload (no signature check).
///
/// # Errors
///
/// Invalid JWS payload JSON.
pub fn extract_offer_payload(offer: &SignedOffer) -> Result<OfferPayload, OfferReceiptError> {
    match offer {
        SignedOffer::Jws { signature, .. } => extract_jws_payload(signature),
        SignedOffer::Eip712 { payload, .. } => Ok(payload.clone()),
    }
}

/// Create a JWS-signed receipt. Omits `transaction` when unset.
///
/// # Errors
///
/// Clock or sign failure.
pub fn create_receipt_jws(
    input: &ReceiptInput,
    signer: &dyn JwsSigner,
) -> Result<SignedReceipt, OfferReceiptError> {
    let payload = ReceiptPayload {
        version: EXTENSION_VERSION,
        network: input.network.clone(),
        resource_url: input.resource_url.clone(),
        payer: input.payer.clone(),
        issued_at: unix_now()?,
        transaction: input.transaction.clone().filter(|s| !s.is_empty()),
    };
    let jws = create_jws(&payload, signer)?;
    Ok(SignedReceipt::Jws { signature: jws })
}

/// Create an EIP-712-signed receipt. Empty `transaction` when unset.
///
/// # Errors
///
/// Clock or sign failure.
pub fn create_receipt_eip712(
    input: &ReceiptInput,
    signer: &dyn Eip712DigestSigner,
) -> Result<SignedReceipt, OfferReceiptError> {
    let payload = ReceiptPayload {
        version: EXTENSION_VERSION,
        network: input.network.clone(),
        resource_url: input.resource_url.clone(),
        payer: input.payer.clone(),
        issued_at: unix_now()?,
        transaction: Some(input.transaction.clone().unwrap_or_default()),
    };
    let signature = sign_receipt_eip712(&payload, signer)?;
    Ok(SignedReceipt::Eip712 { payload, signature })
}

/// Extract the receipt payload (no signature check).
///
/// # Errors
///
/// Invalid JWS payload JSON.
pub fn extract_receipt_payload(
    receipt: &SignedReceipt,
) -> Result<ReceiptPayload, OfferReceiptError> {
    match receipt {
        SignedReceipt::Jws { signature } => extract_jws_payload(signature),
        SignedReceipt::Eip712 { payload, .. } => Ok(payload.clone()),
    }
}
