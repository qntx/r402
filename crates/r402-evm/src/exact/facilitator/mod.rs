//! Facilitator-side payment verification and settlement for EIP-155 exact scheme.
//!
//! This module implements the facilitator logic for verifying and settling
//! EVM exact payments on EVM chains. It currently supports EIP-3009
//! (`transferWithAuthorization`) and routes based on [`ExactPayload`] variants.
//!
//! Key capabilities:
//! - Signature verification (EOA, EIP-1271, EIP-6492)
//! - Balance and amount validation
//! - EIP-712 domain construction
//! - On-chain settlement with gas management
//! - Smart wallet deployment for counterfactual signatures

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use alloy_primitives::{Address, B256, Bytes, TxHash, U256, hex};
use alloy_provider::Provider;
use compact_str::CompactString;
use r402_extensions::BuilderCodeFacilitatorExtension;
use r402_facilitator::{
    Duplicate, Facilitator, InMemoryPendingSettlementStore, PendingSettlementStore, SettlementCache,
};
use r402_protocol::error::{AsPaymentProblem, ErrorReason, FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment as wire;
use r402_protocol::payment::UnixTimestamp;
use r402_protocol::scheme::{ExactScheme, SchemeId};

use crate::chain::Eip155MetaTransactionProvider;
use crate::exact::{Eip155Exact, ExactPayload, payload};

/// Awaits a future, optionally instrumenting it with a tracing span.
macro_rules! traced {
    ($fut:expr, $span:expr) => {{
        #[cfg(feature = "telemetry")]
        {
            use tracing::Instrument;
            $fut.instrument($span).await
        }
        #[cfg(not(feature = "telemetry"))]
        {
            $fut.await
        }
    }};
}

/// Creates a tracing span for a `transferWithAuthorization` call.
///
/// All EIP-3009 call sites share the same set of span fields; this macro
/// avoids repeating the field list at every call site.
#[allow(
    unused_macros,
    reason = "expanded only when traced! keeps the span (telemetry)"
)]
macro_rules! transfer_span {
    ($name:expr, $call:expr $(, $key:ident = $val:expr)*) => {{
        #[cfg(feature = "telemetry")]
        {
            tracing::info_span!($name,
                from = %$call.from,
                to = %$call.to,
                value = %$call.value,
                valid_after = %$call.valid_after,
                valid_before = %$call.valid_before,
                nonce = %$call.nonce,
                signature = %$call.signature,
                token_contract = %$call.contract_address,
                $($key = $val,)*
                otel.kind = "client",
            )
        }
        #[cfg(not(feature = "telemetry"))]
        {
            let _ = &$call;
            ()
        }
    }};
}

pub(crate) mod contract;
pub(crate) mod eip6492;
mod settle;
pub(crate) mod signature;
mod verify;

use eip6492::TRANSFER_EVENT_MISMATCH;
use settle::{ExpectedTransfer, reconcile_pending_receipt, settle_payment, settle_permit2_payment};
use verify::{verify_payment, verify_permit2_payment};

pub(crate) use verify::{permit2_allowance_gate, permit2_extension_covers_allowance};

use crate::error::Eip155ExactError;

/// A fully specified ERC-3009 authorization payload for EVM settlement.
#[derive(Debug)]
pub struct Eip3009Payment {
    /// Authorized sender (`from`) — EOA or smart wallet.
    pub from: Address,
    /// Authorized recipient (`to`).
    pub to: Address,
    /// Transfer amount (token units).
    pub value: U256,
    /// Not valid before this timestamp (inclusive).
    pub valid_after: UnixTimestamp,
    /// Not valid at/after this timestamp (exclusive).
    pub valid_before: UnixTimestamp,
    /// Unique 32-byte nonce (prevents replay).
    pub nonce: B256,
    /// Raw signature bytes (EIP-1271 or EIP-6492-wrapped).
    pub signature: Bytes,
}

/// A fully specified Permit2 authorization payload for EVM settlement.
#[derive(Debug)]
pub struct Permit2Payment {
    /// Signer / owner address.
    pub from: Address,
    /// Destination address for funds.
    pub to: Address,
    /// Token contract address.
    pub token: Address,
    /// Permitted amount (token units).
    pub amount: U256,
    /// Must be the `x402Permit2Proxy` address.
    pub spender: Address,
    /// Unique nonce (uint256).
    pub nonce: U256,
    /// Signature expires after this unix timestamp.
    pub deadline: U256,
    /// Payment invalid before this timestamp.
    pub valid_after: U256,
    /// EIP-712 signature bytes.
    pub signature: Bytes,
    /// Buyer-signed EIP-2612 permit when `eip2612GasSponsoring` is attached.
    pub eip2612: Option<crate::eip2612::Eip2612SignedPermit>,
}

/// Facilitator for EIP-155 exact scheme payments.
///
/// Supports both EIP-3009 and Permit2 transfer methods. The transfer method
/// is determined by the [`ExactPayload`] variant in the payment payload.
///
/// Maintains a [`SettlementCache`] keyed by `chain_id:nonce_hex` so that
/// concurrent or replayed `/settle` calls within the on-chain replay
/// window return [`VerificationError::DuplicateSettlement`] before the
/// transaction is broadcast a second time. EIP-3009 and Permit2 nonces
/// are already unique per buyer + token + chain, so this cache mirrors
/// the underlying chain's protection but reacts faster than RPC
/// confirmation.
pub struct Eip155ExactFacilitator<P> {
    provider: P,
    /// Grace period (in seconds) applied to time-window checks to tolerate
    /// clock drift between the facilitator and the blockchain network.
    clock_skew_tolerance: u64,
    /// Settle-time deduplication cache (see struct-level docs).
    settlement_cache: SettlementCache,
    /// Factories the facilitator will call for undeployed EIP-6492 wallets.
    eip6492_allowed_factories: Vec<Address>,
    /// Broadcast-but-unconfirmed settlement hashes keyed by signature hex.
    pending: Arc<dyn PendingSettlementStore>,
    /// Whether `erc20ApprovalGasSponsoring` is registered on this facilitator.
    erc20_approval_enabled: bool,
    /// Optional ERC-8021 builder-code suffix at settle (`w` + echoed `a`/`s`).
    builder_code: Option<BuilderCodeFacilitatorExtension>,
}

impl<P> Eip155ExactFacilitator<P> {
    /// Constructs a facilitator from a chain provider.
    ///
    /// Uses [`crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS`] (6 s, one EVM
    /// block, aligned with Go's `Permit2DeadlineBuffer`) for time-window
    /// validation, and a default [`SettlementCache`] with the spec-recommended
    /// 2-minute TTL.
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub fn try_new(provider: P) -> Result<Self, FacilitatorError> {
        Ok(Self::with_settlement_cache(
            provider,
            SettlementCache::new(),
        ))
    }

    /// Creates a facilitator with a caller-supplied [`SettlementCache`].
    ///
    /// Use this when you want to share a cache across worker tasks (e.g.
    /// behind a load-balancer that pins by buyer address) or to plug in a
    /// custom backend (Redis, etc.) by wrapping its primitives in the same
    /// shape.
    pub fn with_settlement_cache(provider: P, settlement_cache: SettlementCache) -> Self {
        Self {
            provider,
            clock_skew_tolerance: crate::EVM_DEFAULT_CLOCK_SKEW_TOLERANCE_SECS,
            settlement_cache,
            eip6492_allowed_factories: Vec::new(),
            pending: Arc::new(InMemoryPendingSettlementStore::new()),
            erc20_approval_enabled: false,
            builder_code: None,
        }
    }

    /// Allowlist of EIP-6492 factories. Default empty (fail-closed).
    #[must_use]
    pub fn with_eip6492_allowed_factories(mut self, factories: Vec<Address>) -> Self {
        self.eip6492_allowed_factories = factories;
        self
    }

    /// Override the pending-settlement store (retry reconcile).
    #[must_use]
    pub fn with_pending_store(mut self, store: Arc<dyn PendingSettlementStore>) -> Self {
        self.pending = store;
        self
    }

    /// Register `erc20ApprovalGasSponsoring` on this facilitator (official `getExtension`).
    #[must_use]
    pub const fn with_erc20_approval_gas_sponsoring(mut self) -> Self {
        self.erc20_approval_enabled = true;
        self
    }

    /// Appends an ERC-8021 Schema 2 suffix (`w`) on settlement transactions.
    #[must_use]
    pub fn with_builder_code(mut self, extension: BuilderCodeFacilitatorExtension) -> Self {
        self.builder_code = Some(extension);
        self
    }

    fn data_suffix(&self, extensions: &wire::Extensions) -> Vec<u8> {
        self.builder_code
            .as_ref()
            .and_then(|ext| ext.build_data_suffix(extensions, 2))
            .unwrap_or_default()
    }

    /// Sets a custom clock-skew tolerance (in seconds) for time-window checks.
    ///
    /// A larger value is more lenient toward clock drift between the facilitator
    /// and the chain; a value of `0` enforces exact-time boundaries.
    #[must_use]
    pub const fn with_clock_skew_tolerance(mut self, seconds: u64) -> Self {
        self.clock_skew_tolerance = seconds;
        self
    }

    /// Builds a stable settlement-cache key for the EIP-3009 path.
    ///
    /// `B256` formats as `0x`-prefixed lowercase hex, matching the
    /// `eip155:<chain>:<nonce>` convention used by the Go SDK.
    fn eip3009_cache_key(&self, nonce: B256) -> String
    where
        P: ChainProvider,
    {
        format!("{}:{nonce}", self.provider.chain_id())
    }

    /// Builds a stable settlement-cache key for the Permit2 path.
    ///
    /// Permit2 nonces are `uint256`; the `:permit2:` infix discriminates
    /// from EIP-3009 32-byte nonces so the two namespaces never collide
    /// on a value that happens to be representable in both.
    fn permit2_cache_key(&self, nonce: U256) -> String
    where
        P: ChainProvider,
    {
        format!("{}:permit2:{nonce:#x}", self.provider.chain_id())
    }
}

impl<P> Eip155ExactFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    async fn reconcile_pending(
        &self,
        payload: &payload::v2::PaymentPayload,
        pending_key: &str,
        cached: CompactString,
        network: CompactString,
        amount: CompactString,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let hash: TxHash = cached.parse().map_err(|e| {
            FacilitatorError::Onchain(format!("invalid pending settlement hash: {e}"))
        })?;
        let expected = expected_transfer(payload);
        let payer = payload.payload.sender();
        match reconcile_pending_receipt(self.provider.inner(), hash, expected).await {
            Ok(confirmed) => {
                self.pending.delete(pending_key);
                Ok(settle_success(payer, confirmed, network, amount))
            }
            Err(err) => self.map_settle_error(err, pending_key, payer, network),
        }
    }

    async fn broadcast_and_confirm(
        &self,
        payload: &payload::v2::PaymentPayload,
        requirements: &payload::v2::PaymentRequirements,
        pending_key: &str,
        network: CompactString,
        amount: CompactString,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        match &payload.payload {
            ExactPayload::Eip3009(eip3009) => {
                let (contract, payment, eip712_domain) = match verify::assert_valid_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    eip3009,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await
                {
                    Ok(prepared) => prepared,
                    Err(err) => {
                        return self.map_settle_error(
                            err,
                            pending_key,
                            payload.payload.sender(),
                            network,
                        );
                    }
                };
                let payer = payment.from;
                let suffix = self.data_suffix(&payload.extensions);
                match settle_payment(
                    &self.provider,
                    &contract,
                    &payment,
                    &eip712_domain,
                    &self.eip6492_allowed_factories,
                    &suffix,
                )
                .await
                {
                    Ok(tx_hash) => {
                        self.pending.delete(pending_key);
                        Ok(settle_success(payer, tx_hash, network, amount))
                    }
                    Err(err) => self.map_settle_error(err, pending_key, payer, network),
                }
            }
            ExactPayload::Permit2(permit2) => {
                let (_erc20, payment, _eip712_domain) = match verify::assert_valid_permit2_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    permit2,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                    self.erc20_approval_enabled,
                )
                .await
                {
                    Ok(prepared) => prepared,
                    Err(err) => {
                        return self.map_settle_error(
                            err,
                            pending_key,
                            payload.payload.sender(),
                            network,
                        );
                    }
                };
                let payer = payment.from;
                let suffix = self.data_suffix(&payload.extensions);
                match settle_permit2_payment(&self.provider, &payment, &suffix).await {
                    Ok(tx_hash) => {
                        self.pending.delete(pending_key);
                        Ok(settle_success(payer, tx_hash, network, amount))
                    }
                    Err(err) => self.map_settle_error(err, pending_key, payer, network),
                }
            }
        }
    }

    fn map_settle_error(
        &self,
        err: Eip155ExactError,
        pending_key: &str,
        payer: Address,
        network: CompactString,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        match err {
            Eip155ExactError::ReceiptWait { hash, .. } => {
                self.pending
                    .set(pending_key, CompactString::from(hash.to_string()));
                Ok(settle_failure(
                    ErrorReason::SettlementPending,
                    Some(payer),
                    hash.to_string(),
                    network,
                ))
            }
            Eip155ExactError::TransferEventMismatch(hash) => {
                self.pending.delete(pending_key);
                Ok(settle_failure(
                    ErrorReason::from_wire(TRANSFER_EVENT_MISMATCH),
                    Some(payer),
                    hash.to_string(),
                    network,
                ))
            }
            Eip155ExactError::TransactionReverted(hash) => {
                self.pending.delete(pending_key);
                Ok(settle_failure(
                    ErrorReason::from_wire("invalid_exact_evm_transaction_failed"),
                    Some(payer),
                    hash.to_string(),
                    network,
                ))
            }
            Eip155ExactError::PaymentVerification(e) => Ok(settle_failure(
                e.as_payment_problem().reason(),
                Some(payer),
                "",
                network,
            )),
            other => Err(other.into()),
        }
    }
}

fn expected_transfer(payload: &payload::v2::PaymentPayload) -> ExpectedTransfer {
    match &payload.payload {
        ExactPayload::Eip3009(eip3009) => ExpectedTransfer {
            token: payload.accepted.asset.into(),
            from: eip3009.authorization.from,
            to: eip3009.authorization.to,
            value: eip3009.authorization.value.into(),
        },
        ExactPayload::Permit2(permit2) => ExpectedTransfer {
            token: permit2.permit2_authorization.permitted.token,
            from: permit2.permit2_authorization.from,
            to: permit2.permit2_authorization.witness.to,
            value: permit2.permit2_authorization.permitted.amount.into(),
        },
    }
}

fn settle_success(
    payer: Address,
    hash: TxHash,
    network: CompactString,
    amount: CompactString,
) -> wire::SettleResponse {
    wire::SettleResponse::Success {
        payer: Some(payer.to_string().into()),
        transaction: hash.to_string().into(),
        network,
        amount: Some(amount),
        extensions: wire::Extensions::new(),
        extension_responses: wire::Extensions::new(),
        extra: None,
    }
}

fn settle_failure(
    reason: ErrorReason,
    payer: Option<Address>,
    transaction: impl Into<CompactString>,
    network: CompactString,
) -> wire::SettleResponse {
    wire::SettleResponse::Failure {
        reason,
        message: None,
        payer: payer.map(|p| p.to_string().into()),
        transaction: transaction.into(),
        network,
        extensions: wire::Extensions::new(),
        extension_responses: wire::Extensions::new(),
        extra: None,
    }
}

fn should_release_cache(outcome: &Result<wire::SettleResponse, FacilitatorError>) -> bool {
    match outcome {
        Ok(wire::SettleResponse::Success { .. }) => false,
        Ok(wire::SettleResponse::Failure { transaction, .. }) => transaction.is_empty(),
        Ok(_) | Err(_) => true,
    }
}

impl<P> std::fmt::Debug for Eip155ExactFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155ExactFacilitator")
            .finish_non_exhaustive()
    }
}

impl<P> Facilitator for Eip155ExactFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
    Eip155ExactError: From<P::Error>,
{
    async fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let request = payload::v2::VerifyRequest::from_verify(request)?;
        let payload = &request.payment_payload;
        let requirements = &request.payment_requirements;
        match &payload.payload {
            ExactPayload::Eip3009(eip3009) => {
                let (contract, payment, eip712_domain) = verify::assert_valid_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    eip3009,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                )
                .await?;
                let payer = verify_payment(
                    self.provider.inner(),
                    &contract,
                    &payment,
                    &eip712_domain,
                    &self.eip6492_allowed_factories,
                )
                .await?;
                Ok(wire::VerifyResponse::valid(payer.to_string()))
            }
            ExactPayload::Permit2(permit2) => {
                let (_erc20, payment, eip712_domain) = verify::assert_valid_permit2_payment(
                    self.provider.inner(),
                    self.provider.chain(),
                    permit2,
                    payload,
                    requirements,
                    self.clock_skew_tolerance,
                    self.erc20_approval_enabled,
                )
                .await?;
                let payer =
                    verify_permit2_payment(self.provider.inner(), &payment, &eip712_domain).await?;
                Ok(wire::VerifyResponse::valid(payer.to_string()))
            }
        }
    }

    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let request = payload::v2::SettleRequest::from_settle(request)?;
        let payload = &request.payment_payload;
        let requirements = &request.payment_requirements;
        let pending_key = hex::encode_prefixed(payload.payload.signature());
        let network: CompactString = payload.accepted.network.to_string().into();
        let amount: CompactString = requirements.amount.0.to_string().into();

        if let Some(cached) = self.pending.get(&pending_key) {
            self.pending.delete(&pending_key);
            return self
                .reconcile_pending(payload, &pending_key, cached, network, amount)
                .await;
        }

        let cache_key = match &payload.payload {
            ExactPayload::Eip3009(eip3009) => self.eip3009_cache_key(eip3009.authorization.nonce),
            ExactPayload::Permit2(permit2) => {
                self.permit2_cache_key(permit2.permit2_authorization.nonce.into())
            }
        };
        if self.settlement_cache.reserve(cache_key.clone()) == Duplicate::Yes {
            return Err(VerificationError::DuplicateSettlement.into());
        }

        let outcome = self
            .broadcast_and_confirm(payload, requirements, &pending_key, network, amount)
            .await;
        if should_release_cache(&outcome) {
            self.settlement_cache.release(&cache_key);
        }
        outcome
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        use compact_str::CompactString;

        let chain_id = self.provider.chain_id();
        let kinds = vec![wire::SupportedPaymentKind::new(
            wire::V2.into(),
            ExactScheme.to_string(),
            chain_id.to_string(),
        )];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            Eip155Exact.caip_family().into(),
            self.provider
                .signer_addresses()
                .into_iter()
                .map(CompactString::from)
                .collect(),
        );
        std::future::ready(Ok(wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes};
    use compact_str::CompactString;
    use r402_extensions::{
        BUILDER_CODE, BuilderCodeFacilitatorExtension, parse_builder_code_suffix_from_calldata,
    };
    use r402_protocol::payment::ExtensionEntry;
    use serde_json::json;

    use super::*;
    use crate::chain::MetaTransaction;

    fn facilitator_with_w() -> Eip155ExactFacilitator<()> {
        Eip155ExactFacilitator::with_settlement_cache((), SettlementCache::new()).with_builder_code(
            BuilderCodeFacilitatorExtension::new()
                .with_builder_code("bc_myfacilitator")
                .expect("valid facilitator builder code"),
        )
    }

    #[test]
    fn settle_calldata_carries_builder_code_suffix() {
        let fac = facilitator_with_w();
        let mut extensions = wire::Extensions::new();
        extensions.insert(
            BUILDER_CODE,
            ExtensionEntry::info(json!({ "a": "bc_myapp", "s": ["bc_client"] })),
        );
        let suffix = fac.data_suffix(&extensions);
        assert!(
            !suffix.is_empty(),
            "configured facilitator must emit a suffix"
        );
        let calldata = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]);
        let tx = MetaTransaction::new(Address::ZERO, calldata, 1).with_data_suffix(&suffix);
        assert!(
            tx.calldata.as_ref().ends_with(&suffix),
            "settlement calldata must end with the ERC-8021 suffix"
        );
        let parsed = parse_builder_code_suffix_from_calldata(tx.calldata.as_ref())
            .expect("calldata must parse as Schema 2 suffix");
        assert_eq!(parsed.a.as_deref(), Some("bc_myapp"));
        assert_eq!(parsed.w.as_deref(), Some("bc_myfacilitator"));
        assert_eq!(
            parsed
                .s
                .iter()
                .map(CompactString::as_str)
                .collect::<Vec<_>>(),
            vec!["bc_client"]
        );
    }
}
