//! Facilitator for EVM `auth-capture` (off-chain verify + on-chain authorize/charge).

use std::collections::HashMap;
use std::future::Future;

use alloy_primitives::{Bytes, Uint};
use alloy_provider::Provider;
use alloy_sol_types::{SolValue, sol};
use compact_str::CompactString;
use r402_facilitator::Facilitator;
use r402_protocol::error::{FacilitatorError, VerificationError};
use r402_protocol::network::ChainProvider;
use r402_protocol::payment as wire;
use r402_protocol::scheme::{AuthCaptureScheme, SchemeId};

use super::Eip155AuthCapture;
use super::payload::{
    AUTH_CAPTURE_ESCROW_ADDRESS, AuthCapturePayload, EIP3009_TOKEN_COLLECTOR_ADDRESS,
    PERMIT2_TOKEN_COLLECTOR_ADDRESS, v2,
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
    /// Constructs a facilitator from a chain provider.
    ///
    /// # Errors
    ///
    /// Currently infallible. [`Result`] so `try_new(provider)?` compiles.
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Result so try_new(provider)? compiles"
    )]
    pub const fn try_new(provider: P) -> Result<Self, FacilitatorError> {
        Ok(Self { provider })
    }

    fn parse_chain_id(&self) -> Result<u64, FacilitatorError> {
        self.provider
            .chain_id()
            .reference()
            .parse::<u64>()
            .map_err(|e| {
                FacilitatorError::Verification(VerificationError::InvalidFormat(e.to_string()))
            })
    }

    fn verify_sync(
        &self,
        request: wire::VerifyRequest,
    ) -> Result<wire::VerifyResponse, FacilitatorError> {
        let typed = v2::VerifyRequest::from_verify(request)?;
        let chain_id = self.parse_chain_id()?;
        let payer = verify_offchain(
            &typed.payment_payload,
            &typed.payment_requirements,
            chain_id,
        )?;
        Ok(wire::VerifyResponse::valid(format!("{payer:#x}")))
    }

    fn supported_sync(&self) -> wire::SupportedResponse {
        let chain_id = self.provider.chain_id();
        let kinds = vec![wire::SupportedPaymentKind::new(
            wire::V2.into(),
            AuthCaptureScheme.to_string(),
            chain_id.to_string(),
        )];
        let mut signers: HashMap<CompactString, Vec<CompactString>> = HashMap::with_capacity(1);
        let _ = signers.insert(
            Eip155AuthCapture.caip_family().into(),
            self.provider
                .signer_addresses()
                .into_iter()
                .map(CompactString::from)
                .collect(),
        );
        wire::SupportedResponse::new()
            .with_kinds(kinds)
            .with_signers(signers)
    }
}

fn uint120(value: alloy_primitives::U256) -> Result<Uint<120, 2>, FacilitatorError> {
    let as_u128 = u128::try_from(value).map_err(|_| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(
            "maxAmount exceeds uint120".into(),
        ))
    })?;
    Uint::<120, 2>::try_from(as_u128).map_err(|_| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(
            "maxAmount exceeds uint120".into(),
        ))
    })
}

fn uint48(value: u64) -> Result<Uint<48, 1>, FacilitatorError> {
    Uint::<48, 1>::try_from(value).map_err(|_| {
        FacilitatorError::Verification(VerificationError::InvalidFormat(
            "timestamp exceeds uint48".into(),
        ))
    })
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
    ) -> Result<wire::SettleResponse, FacilitatorError> {
        let typed = v2::SettleRequest::from_settle(request)?;
        let chain_id = self.parse_chain_id()?;
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
                let deadline = u64::try_from(p.permit2_authorization.deadline.0).map_err(|_| {
                    FacilitatorError::Verification(VerificationError::InvalidFormat(
                        "permit2 deadline exceeds u64".into(),
                    ))
                })?;
                let data = SolValue::abi_encode(&p.signature.as_ref());
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

        let payment_info = IAuthCaptureEscrow::PaymentInfo {
            operator: info.operator,
            payer: info.payer,
            receiver: info.receiver,
            token: info.token,
            maxAmount: uint120(info.max_amount)?,
            preApprovalExpiry: uint48(info.pre_approval_expiry)?,
            authorizationExpiry: uint48(info.authorization_expiry)?,
            refundExpiry: uint48(info.refund_expiry)?,
            minFeeBps: info.min_fee_bps,
            maxFeeBps: info.max_fee_bps,
            feeReceiver: info.fee_receiver,
            salt: info.salt,
        };

        let escrow = IAuthCaptureEscrow::new(AUTH_CAPTURE_ESCROW_ADDRESS, self.provider.inner());
        let amount = requirements.amount.0;
        let pending = escrow
            .authorize(payment_info, amount, collector, collector_data)
            .send()
            .await
            .map_err(|e| FacilitatorError::Onchain(format!("auth-capture authorize: {e}")))?;
        let receipt = pending.get_receipt().await.map_err(|e| {
            FacilitatorError::Onchain(format!("auth-capture authorize receipt: {e}"))
        })?;

        if !receipt.status() {
            return Err(FacilitatorError::Onchain(
                "auth-capture transaction reverted".into(),
            ));
        }

        let tx_hash = receipt.transaction_hash;
        Ok(wire::SettleResponse::Success {
            payer: format!("{payer:#x}").into(),
            transaction: format!("{tx_hash:#x}").into(),
            network: requirements.network.to_string().into(),
            amount: Some(requirements.amount.0.to_string().into()),
            extensions: wire::Extensions::new(),
            extension_responses: wire::Extensions::new(),
            extra: None,
        })
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<wire::SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(self.supported_sync()))
    }
}
