//! Buyer register, select, and `spendControls` for the x402 payment protocol.

#![cfg_attr(docsrs, feature(doc_cfg))]

mod candidate;
mod extension;
mod hooks;
mod policy;
mod register;
mod select;
mod spend;

pub use candidate::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
pub use extension::{ClientExtension, DynClientExtension};
pub use hooks::{
    BoxFuture, ClientHooks, CreatedPayment, DynClientHooks, FailureRecovery, HookDecision,
    PaymentCreationContext, PaymentResponseContext, PaymentResponseResult,
};
pub use policy::{MaxAmountPolicy, NetworkPolicy, PaymentPolicy, SchemePolicy};
pub use register::PaymentClient;
pub use select::{FirstMatch, MaxAmount, PaymentSelector, PreferChain};
pub use spend::{AllowedAssets, MaxAmountPerPayment, SpendControlAsset, SpendControls};
