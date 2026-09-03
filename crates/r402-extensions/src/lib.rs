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
#[cfg(feature = "ext-eip2612")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-eip2612")))]
pub mod eip2612;
#[cfg(feature = "ext-erc20-approval")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-erc20-approval")))]
pub mod erc20;
#[cfg(feature = "ext-payment-id")]
#[cfg_attr(docsrs, doc(cfg(feature = "ext-payment-id")))]
pub mod payment_id;
#[cfg(feature = "siwx")]
#[cfg_attr(docsrs, doc(cfg(feature = "siwx")))]
pub mod siwx;

#[cfg(feature = "ext-bazaar")]
pub use bazaar::{
    BAZAAR_KEY, BazaarBodyMethod, BazaarBodyType, BazaarExtension, BazaarQueryMethod,
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
#[cfg(feature = "ext-payment-id")]
pub use payment_id::{
    DEFAULT_CAPACITY, DEFAULT_MAX_AGE, Duplicate, ID_MAX_LEN, ID_MIN_LEN, PAYMENT_IDENTIFIER_KEY,
    PaymentIdError, PaymentIdentifierConfig, PaymentIdentifierExtension, validate_id_format,
};
#[cfg(feature = "siwx")]
pub use siwx::{
    InMemoryPaidAddressStore, PaidAddressStore, SIWX_KEY, SiwxChain, SiwxError, SiwxExtension,
    SiwxOrigin, SiwxOriginError, SiwxProof, SiwxProofError,
};

#[cfg(test)]
mod _dev_deps {
    use alloy_signer as _;
    use alloy_signer_local as _;
    use ed25519_dalek as _;
    use tokio as _;
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
