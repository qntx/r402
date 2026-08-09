//! Facilitator for EVM `auth-capture` (off-chain verify + on-chain authorize/charge).

use std::future::Future;

use alloy_primitives::Bytes;
use alloy_provider::Provider;
use alloy_sol_types::sol;
use r402_core::chain::ChainProvider;
use r402_core::error::{FacilitatorError, VerificationError};
use r402_core::facilitator::{DynFacilitator, Facilitator};
use r402_core::scheme::SchemeBuilder;
use r402_core::wire::{self, Extensions, SettleResponse, SupportedPaymentKind, SupportedResponse};

use super::Eip155AuthCapture;
use super::types::v2;
use super::types::{
    AUTH_CAPTURE_ESCROW_ADDRESS, AuthCapturePayload, EIP3009_TOKEN_COLLECTOR_ADDRESS,
    PERMIT2_TOKEN_COLLECTOR_ADDRESS,
};
use super::verify::{reconstruct_payment_info, verify_offchain};
use crate::chain::Eip155MetaTransactionProvider;

sol! {
    #[sol(rpc)]
    interface IAuthCaptureEscrow {
        struct PaymentInfo {
            address operator;
            address payer;
            address receiver;
            address token;
            uint120 maxAmount;
            uint48 preApprovalExpiry;
            uint48 authorizationExpiry;
            uint48 refundExpiry;
            uint16 minFeeBps;
            uint16 maxFeeBps;
            address feeReceiver;
            uint256 salt;
        }

        function authorize(
            PaymentInfo paymentInfo,
            uint256 amount,
            address tokenCollector,
            bytes collectorData
        ) external;

        function charge(
            PaymentInfo paymentInfo,
            uint256 amount,
            address tokenCollector,
            bytes collectorData
        ) external;
    }
}

/// On-chain auth-capture facilitator bound to an EIP-155 provider.
pub struct Eip155AuthCaptureFacilitator<P> {
    provider: P,
}

impl<P> std::fmt::Debug for Eip155AuthCaptureFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip155AuthCaptureFacilitator")
            .finish_non_exhaustive()
    }
}

impl<P> Eip155AuthCaptureFacilitator<P>
where
    P: ChainProvider,
{
    /// Wraps a chain provider.
    #[must_use]
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }

    fn verify_sync(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let typed = v2::VerifyRequest::from_verify(request)?;
        let chain_id = self
            .provider
            .chain_id()
            .reference()
            .parse::<u64>()
            .map_err(|e| {
                FacilitatorError::Verification(VerificationError::InvalidFormat(e.to_string()))
            })?;
        let payer = verify_offchain(
            &typed.payment_payload,
            &typed.payment_requirements,
            chain_id,
        )?;
        Ok(wire::VerifyResponse::valid(format!("{payer:#x}")))
    }

    fn supported_sync(&self) -> SupportedResponse {
        let network = self.provider.chain_id().to_string();
        let kind = SupportedPaymentKind::new(2, "auth-capture", network);
        SupportedResponse::new().with_kinds(vec![kind])
    }
}

impl<P> SchemeBuilder<P> for Eip155AuthCapture
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync + 'static,
{
    fn build(
        &self,
        provider: P,
        _config: Option<serde_json::Value>,
    ) -> Result<Box<dyn DynFacilitator>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(Eip155AuthCaptureFacilitator::new(provider)))
    }
}

impl<P> Facilitator for Eip155AuthCaptureFacilitator<P>
where
    P: Eip155MetaTransactionProvider + ChainProvider + Send + Sync,
    P::Inner: Provider,
{
    fn verify(
        &self,
        request: wire::VerifyRequest,
    ) -> impl Future<Output = Result<wire::VerifyResponse, FacilitatorError>> + Send {
        std::future::ready(self.verify_sync(request))
    }

    async fn settle(
        &self,
        request: wire::SettleRequest,
    ) -> Result<SettleResponse, FacilitatorError> {
        let typed = v2::SettleRequest::from_settle(request)?;
        let chain_id = self
            .provider
            .chain_id()
            .reference()
            .parse::<u64>()
            .map_err(|e| {
                FacilitatorError::Verification(VerificationError::InvalidFormat(e.to_string()))
            })?;
        let payload = &typed.payment_payload;
        let requirements = &typed.payment_requirements;
        let payer = verify_offchain(payload, requirements, chain_id)?;

        let extra = requirements.extra.as_ref().ok_or_else(|| {
            FacilitatorError::Verification(VerificationError::InvalidFormat("missing extra".into()))
        })?;

        let (pre_approval, collector, collector_data) = match &payload.payload {
            AuthCapturePayload::Eip3009(p) => (
                p.authorization.valid_before.as_secs(),
                EIP3009_TOKEN_COLLECTOR_ADDRESS,
                p.signature.clone(),
            ),
            AuthCapturePayload::Permit2(p) => {
                let deadline: u64 = p
                    .permit2_authorization
                    .deadline
                    .0
                    .try_into()
                    .unwrap_or(u64::MAX);
                let data = alloy_sol_types::SolValue::abi_encode(&p.signature.as_ref());
                (deadline, PERMIT2_TOKEN_COLLECTOR_ADDRESS, Bytes::from(data))
            }
        };

        let info = reconstruct_payment_info(
            requirements,
            extra,
            payer,
            payload.payload.salt(),
            pre_approval,
        );

        let max_amount_u120 = u128::try_from(info.max_amount).unwrap_or(u128::MAX);
        let payment_info = IAuthCaptureEscrow::PaymentInfo {
            operator: info.operator,
            payer: info.payer,
            receiver: info.receiver,
            token: info.token,
            maxAmount: alloy_primitives::Uint::from(max_amount_u120),
            preApprovalExpiry: alloy_primitives::Uint::from(info.pre_approval_expiry),
            authorizationExpiry: alloy_primitives::Uint::from(info.authorization_expiry),
            refundExpiry: alloy_primitives::Uint::from(info.refund_expiry),
            minFeeBps: info.min_fee_bps,
            maxFeeBps: info.max_fee_bps,
            feeReceiver: info.fee_receiver,
            salt: info.salt,
        };

        let escrow = IAuthCaptureEscrow::new(AUTH_CAPTURE_ESCROW_ADDRESS, self.provider.inner());
        let amount = requirements.amount.0;
        let receipt = if extra.auto_capture() {
            let pending = escrow
                .charge(payment_info, amount, collector, collector_data)
                .send()
                .await
                .map_err(|e| FacilitatorError::Onchain(format!("auth-capture charge: {e}")))?;
            pending.get_receipt().await.map_err(|e| {
                FacilitatorError::Onchain(format!("auth-capture charge receipt: {e}"))
            })?
        } else {
            let pending = escrow
                .authorize(payment_info, amount, collector, collector_data)
                .send()
                .await
                .map_err(|e| FacilitatorError::Onchain(format!("auth-capture authorize: {e}")))?;
            pending.get_receipt().await.map_err(|e| {
                FacilitatorError::Onchain(format!("auth-capture authorize receipt: {e}"))
            })?
        };

        if !receipt.status() {
            return Err(FacilitatorError::Onchain(
                "auth-capture transaction reverted".into(),
            ));
        }

        let tx_hash = receipt.transaction_hash;
        Ok(SettleResponse::Success {
            payer: format!("{payer:#x}").into(),
            transaction: format!("{tx_hash:#x}").into(),
            network: requirements.network.to_string().into(),
            amount: Some(requirements.amount.0.to_string().into()),
            extensions: Extensions::new(),
        })
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(self.supported_sync()))
    }
}
