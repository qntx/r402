//! EIP-712 offer/receipt hashing, signing, and recovery.

use alloy_primitives::{Address, B256, Signature, U256};
use alloy_sol_types::{SolStruct, eip712_domain};

use super::{OfferPayload, OfferReceiptError, ReceiptPayload, SignedOffer, SignedReceipt};

alloy_sol_types::sol! {
    struct Offer {
        uint256 version;
        string resourceUrl;
        string scheme;
        string network;
        string asset;
        string payTo;
        string amount;
        uint256 validUntil;
    }

    struct Receipt {
        uint256 version;
        string network;
        string resourceUrl;
        string payer;
        uint256 issuedAt;
        string transaction;
    }
}

/// Signs an EIP-712 digest (32-byte hash).
pub trait Eip712DigestSigner: Send + Sync {
    /// Sign `digest` and return a 65-byte Ethereum signature.
    ///
    /// # Errors
    ///
    /// [`OfferReceiptError::Sign`].
    fn sign_digest(&self, digest: B256) -> Result<Signature, OfferReceiptError>;
}

impl<S> Eip712DigestSigner for S
where
    S: alloy_signer::SignerSync + Send + Sync,
{
    fn sign_digest(&self, digest: B256) -> Result<Signature, OfferReceiptError> {
        self.sign_hash_sync(&digest)
            .map_err(|e| OfferReceiptError::Sign(e.to_string()))
    }
}

/// Recovered EIP-712 signer plus payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eip712Verification<T> {
    /// Recovered signer address.
    pub signer: Address,
    /// Verified payload.
    pub payload: T,
}

/// EIP-712 digest for an offer (`name: "x402 offer"`, `chainId: 1`).
#[must_use]
pub fn hash_offer_typed_data(payload: &OfferPayload) -> B256 {
    let domain = eip712_domain! {
        name: "x402 offer",
        version: "1",
        chain_id: 1u64,
    };
    let offer = Offer {
        version: U256::from(payload.version),
        resourceUrl: payload.resource_url.clone(),
        scheme: payload.scheme.clone(),
        network: payload.network.clone(),
        asset: payload.asset.clone(),
        payTo: payload.pay_to.clone(),
        amount: payload.amount.clone(),
        validUntil: U256::from(payload.valid_until),
    };
    offer.eip712_signing_hash(&domain)
}

/// EIP-712 digest for a receipt (`name: "x402 receipt"`, `chainId: 1`).
#[must_use]
pub fn hash_receipt_typed_data(payload: &ReceiptPayload) -> B256 {
    let domain = eip712_domain! {
        name: "x402 receipt",
        version: "1",
        chain_id: 1u64,
    };
    let receipt = Receipt {
        version: U256::from(payload.version),
        network: payload.network.clone(),
        resourceUrl: payload.resource_url.clone(),
        payer: payload.payer.clone(),
        issuedAt: U256::from(payload.issued_at),
        transaction: payload.transaction.clone().unwrap_or_default(),
    };
    receipt.eip712_signing_hash(&domain)
}

/// Sign an offer digest.
///
/// # Errors
///
/// Signer failure.
pub(super) fn sign_offer_eip712(
    payload: &OfferPayload,
    signer: &dyn Eip712DigestSigner,
) -> Result<String, OfferReceiptError> {
    Ok(encode_eth_signature(
        signer.sign_digest(hash_offer_typed_data(payload))?,
    ))
}

/// Sign a receipt digest.
///
/// # Errors
///
/// Signer failure.
pub(super) fn sign_receipt_eip712(
    payload: &ReceiptPayload,
    signer: &dyn Eip712DigestSigner,
) -> Result<String, OfferReceiptError> {
    Ok(encode_eth_signature(
        signer.sign_digest(hash_receipt_typed_data(payload))?,
    ))
}

/// Recover the EIP-712 offer signer. Does not check authorization.
///
/// # Errors
///
/// Format or recover failure.
pub fn verify_offer_signature_eip712(
    offer: &SignedOffer,
) -> Result<Eip712Verification<OfferPayload>, OfferReceiptError> {
    let SignedOffer::Eip712 {
        payload, signature, ..
    } = offer
    else {
        return Err(OfferReceiptError::UnknownFormat("expected eip712".into()));
    };
    let hash = hash_offer_typed_data(payload);
    let sig = parse_eth_signature(signature)?;
    let signer = sig
        .recover_address_from_prehash(&hash)
        .map_err(|e| OfferReceiptError::Verify(e.to_string()))?;
    Ok(Eip712Verification {
        signer,
        payload: payload.clone(),
    })
}

/// Recover the EIP-712 receipt signer. Does not check authorization.
///
/// # Errors
///
/// Format or recover failure.
pub fn verify_receipt_signature_eip712(
    receipt: &SignedReceipt,
) -> Result<Eip712Verification<ReceiptPayload>, OfferReceiptError> {
    let SignedReceipt::Eip712 { payload, signature } = receipt else {
        return Err(OfferReceiptError::UnknownFormat("expected eip712".into()));
    };
    let hash = hash_receipt_typed_data(payload);
    let sig = parse_eth_signature(signature)?;
    let signer = sig
        .recover_address_from_prehash(&hash)
        .map_err(|e| OfferReceiptError::Verify(e.to_string()))?;
    Ok(Eip712Verification {
        signer,
        payload: payload.clone(),
    })
}

fn parse_eth_signature(sig: &str) -> Result<Signature, OfferReceiptError> {
    let hex = sig.strip_prefix("0x").unwrap_or(sig);
    let bytes = hex::decode(hex).map_err(|e| OfferReceiptError::Verify(e.to_string()))?;
    Signature::from_raw(&bytes).map_err(|e| OfferReceiptError::Verify(e.to_string()))
}

fn encode_eth_signature(sig: Signature) -> String {
    format!("0x{}", hex::encode(sig.as_bytes()))
}
