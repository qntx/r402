//! Client extract/match helpers for signed offers and receipts.

use r402_protocol::payment::{PaymentRequired, PaymentRequirements, SettleResponse};

use super::create::{extract_offer_payload, extract_receipt_payload};
use super::{OFFER_RECEIPT, OfferPayload, OfferReceiptError, SignedOffer, SignedReceipt};

/// Signed offer with payload fields at the top level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedOffer {
    /// Canonical payload.
    pub payload: OfferPayload,
    /// Original signed offer.
    pub signed_offer: SignedOffer,
    /// `acceptIndex` hint.
    pub accept_index: Option<u32>,
}

/// Offers from `PaymentRequired.extensions["offer-receipt"].info.offers`.
#[must_use]
pub fn extract_offers_from_payment_required(
    payment_required: &PaymentRequired,
) -> Vec<SignedOffer> {
    let Some(entry) = payment_required.extensions.get(OFFER_RECEIPT) else {
        return Vec::new();
    };
    let value = entry.to_value();
    let offers = value
        .get("info")
        .and_then(|info| info.get("offers"))
        .or_else(|| value.get("offers"));
    let Some(offers) = offers else {
        return Vec::new();
    };
    serde_json::from_value(offers.clone()).unwrap_or_default()
}

/// Decode signed offers (JWS payload is base64-decoded, not verified).
///
/// # Errors
///
/// Invalid JWS payload JSON.
pub fn decode_signed_offers(
    offers: &[SignedOffer],
) -> Result<Vec<DecodedOffer>, OfferReceiptError> {
    offers
        .iter()
        .map(|offer| {
            Ok(DecodedOffer {
                payload: extract_offer_payload(offer)?,
                signed_offer: offer.clone(),
                accept_index: offer.accept_index(),
            })
        })
        .collect()
}

/// Match an offer to an `accepts[]` entry by payload fields (`acceptIndex` is a hint).
#[must_use]
pub fn find_accepts_object_from_signed_offer<'a>(
    payload: &OfferPayload,
    accept_index: Option<u32>,
    accepts: &'a [PaymentRequirements],
) -> Option<&'a PaymentRequirements> {
    if let Some(index) = accept_index
        && let Ok(idx) = usize::try_from(index)
        && let Some(hinted) = accepts.get(idx)
        && fields_match(hinted, payload)
    {
        return Some(hinted);
    }
    accepts.iter().find(|req| fields_match(req, payload))
}

fn fields_match(req: &PaymentRequirements, payload: &OfferPayload) -> bool {
    req.network.to_string() == payload.network
        && req.scheme.as_str() == payload.scheme
        && req.asset.as_str() == payload.asset
        && req.pay_to.as_str() == payload.pay_to
        && req.amount.as_str() == payload.amount
}

/// Receipt from `SettleResponse.extensions["offer-receipt"].info.receipt`.
#[must_use]
pub fn extract_receipt_from_settle_response(response: &SettleResponse) -> Option<SignedReceipt> {
    let (SettleResponse::Success { extensions, .. } | SettleResponse::Failure { extensions, .. }) =
        response
    else {
        return None;
    };
    let entry = extensions.get(OFFER_RECEIPT)?;
    let value = entry.to_value();
    let receipt = value
        .get("info")
        .and_then(|info| info.get("receipt"))
        .or_else(|| value.get("receipt"))?;
    serde_json::from_value(receipt.clone()).ok()
}

/// Payload-level receipt/offer match (no signature check).
#[must_use]
pub fn verify_receipt_matches_offer(
    receipt: &SignedReceipt,
    offer: &DecodedOffer,
    payer_addresses: &[&str],
    max_age_seconds: u64,
    now_unix: u64,
) -> bool {
    let Ok(payload) = extract_receipt_payload(receipt) else {
        return false;
    };
    let resource_ok = payload.resource_url == offer.payload.resource_url;
    let network_ok = payload.network == offer.payload.network;
    let payer_ok = payer_addresses
        .iter()
        .any(|addr| payload.payer.eq_ignore_ascii_case(addr));
    let issued_recently = now_unix.saturating_sub(payload.issued_at) < max_age_seconds;
    resource_ok && network_ok && payer_ok && issued_recently
}
