//! Protocol extension implementations for the x402 payment protocol.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::panic,
        reason = "unit tests panic on assertion failure"
    )
)]

#[cfg(feature = "ext-bazaar")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-bazaar")))]
pub mod bazaar;
#[cfg(feature = "ext-builder-code")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-builder-code")))]
pub mod builder_code;
#[cfg(feature = "ext-eip2612")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-eip2612")))]
pub mod eip2612;
#[cfg(feature = "ext-erc20-approval")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-erc20-approval")))]
pub mod erc20;
#[cfg(feature = "ext-offer-receipt")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-offer-receipt")))]
pub mod offer_receipt;
#[cfg(feature = "ext-payment-id")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-payment-id")))]
pub mod payment_id;
#[cfg(feature = "siwx")]
#[cfg_attr(docsrs, doc(cfg(feature = "siwx")))]
pub mod siwx;

#[cfg(feature = "ext-bazaar-discovery")]
pub use bazaar::discovery::{
    BazaarDiscoveryClient, BazaarDiscoveryError, DiscoveryPagination, DiscoveryResource,
    DiscoveryResourcesResponse, ListDiscoveryResourcesParams, SearchDiscoveryResourcesParams,
    SearchDiscoveryResourcesResponse, SearchPagination, with_bazaar,
};
#[cfg(feature = "ext-bazaar")]
pub use bazaar::{
    BAZAAR_KEY, BazaarBodyMethod, BazaarBodyType, BazaarExtension, BazaarQueryMethod,
};
#[cfg(feature = "ext-builder-code")]
pub use builder_code::{
    BUILDER_CODE, BuilderCodeClient, BuilderCodeData, BuilderCodeError, BuilderCodeExtension,
    BuilderCodeFacilitatorConfig, BuilderCodeFacilitatorExtension, ERC_8021_MARKER,
    ERC_8021_MARKER_HEX, MAX_CLIENT_SERVICE_CODES, MAX_ECHOED_SERVICE_CODES,
    MAX_FACILITATOR_SERVICE_CODES, MAX_SERVER_SERVICE_CODES, MAX_SERVICE_CODES, SCHEMA_2_ID,
    declare_builder_code_extension, encode_builder_code_suffix, is_valid_builder_code,
    parse_builder_code_suffix_from_calldata,
};
#[cfg(feature = "ext-eip2612")]
pub use eip2612::{
    EIP2612_GAS_SPONSORING_KEY, EIP2612_GAS_SPONSORING_VERSION, Eip2612GasSponsoringExtension,
};
#[cfg(feature = "ext-erc20-approval")]
pub use erc20::{
    ERC20_APPROVAL_GAS_SPONSORING_KEY, ERC20_APPROVAL_GAS_SPONSORING_VERSION,
    Erc20ApprovalGasSponsoringExtension,
};
#[cfg(feature = "ext-offer-receipt")]
pub use offer_receipt::{
    DEFAULT_OFFER_VALIDITY_SECONDS, DecodedOffer, EXTENSION_VERSION, EddsaJwsSigner,
    Eip712DigestSigner, Eip712OfferReceiptIssuer, Eip712Verification, Es256JwsSigner,
    Es256kJwsSigner, JwsOfferReceiptIssuer, JwsPublicKey, JwsSigner, OFFER_RECEIPT, OfferInput,
    OfferPayload, OfferReceiptError, OfferReceiptExtension, OfferReceiptIssuer, ReceiptInput,
    ReceiptPayload, SignatureFormat, SignedOffer, SignedReceipt, canonicalize,
    create_eip712_offer_receipt_issuer, create_jws, create_jws_offer_receipt_issuer,
    create_offer_eip712, create_offer_jws, create_receipt_eip712, create_receipt_jws,
    declare_offer_receipt_extension, decode_signed_offers, extract_jws_header, extract_jws_payload,
    extract_offer_payload, extract_offers_from_payment_required,
    extract_receipt_from_settle_response, extract_receipt_payload,
    find_accepts_object_from_signed_offer, hash_offer_typed_data, hash_receipt_typed_data,
    verify_offer_signature_eip712, verify_offer_signature_jws, verify_receipt_matches_offer,
    verify_receipt_signature_eip712, verify_receipt_signature_jws,
};
#[cfg(feature = "ext-payment-id")]
pub use payment_id::{
    DEFAULT_CAPACITY, DEFAULT_MAX_AGE, Duplicate, ID_MAX_LEN, ID_MIN_LEN, PAYMENT_IDENTIFIER_KEY,
    PaymentIdError, PaymentIdentifierConfig, PaymentIdentifierExtension, validate_id_format,
};
#[cfg(feature = "siwx")]
pub use siwx::{
    DEFAULT_STATEMENT, InMemoryPaidAddressStore, PaidAddressStore, SIWX_HTTP_HEADER, SIWX_KEY,
    SiwxChain, SiwxClientExtension, SiwxError, SiwxExtension, SiwxOrigin, SiwxOriginError,
    SiwxProof, SiwxProofError, SiwxSigner,
};

#[cfg(test)]
mod _dev_deps {
    use alloy_signer_local as _;
    #[cfg(not(feature = "siwx"))]
    use http as _;
    use tokio as _;
    use wiremock as _;
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-extensions",
            "package name must match the crate directory"
        );
    }
}
