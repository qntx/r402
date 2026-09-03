//! Canonical Uniswap Permit2 address, wire permissions, and client auto-approve.

use alloy_primitives::{Address, address};
use serde::{Deserialize, Serialize};

use crate::chain::TokenAmount;

/// Canonical Uniswap Permit2 contract address (same on all EVM chains via CREATE2).
pub const PERMIT2_ADDRESS: Address = address!("0x000000000022D473030F116dDEE9F6B43aC78BA3");

/// Permit2 token permissions — which token and how much.
///
/// Part of the `PermitWitnessTransferFrom` message structure that gets signed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Permit2TokenPermissions {
    /// Token contract address.
    pub token: Address,
    /// Amount in smallest unit as decimal string (e.g., `"1000000"` for 1 USDC).
    pub amount: TokenAmount,
}

#[cfg(feature = "client")]
use std::future::Future;
#[cfg(feature = "client")]
use std::pin::Pin;

#[cfg(feature = "client")]
use alloy_primitives::{Bytes, U256};
#[cfg(feature = "client")]
use alloy_sol_types::{SolCall, sol};
#[cfg(feature = "client")]
use r402_protocol::error::ClientError;

/// Object-safe future for [`Permit2Approver::estimate_fees_per_gas`].
#[cfg(feature = "client")]
type GasSponsoringFeeFut<'a> =
    Pin<Box<dyn Future<Output = Result<(u128, u128), ClientError>> + Send + 'a>>;

/// Abstraction for on-chain interactions needed by the Permit2 auto-approve flow.
///
/// Implement this trait to enable automatic Permit2 allowance management.
/// When an approver is provided via a scheme client builder, the client will:
///
/// 1. **Before each Permit2 payment**, call [`check_permit2_allowance`](Self::check_permit2_allowance)
///    to query the current ERC-20 allowance granted to the canonical Permit2 contract.
/// 2. **If the allowance is insufficient**, call [`approve_permit2`](Self::approve_permit2)
///    to send an on-chain `approve(Permit2, MAX)` transaction.
/// 3. **Proceed with normal Permit2 EIP-712 signing** once allowance is confirmed.
#[cfg(feature = "client")]
pub trait Permit2Approver: Send + Sync {
    /// Queries the current ERC-20 allowance that `owner` has granted to the
    /// canonical Permit2 contract for the given `token`.
    fn check_permit2_allowance(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>>;

    /// Sends an ERC-20 `approve(PERMIT2_ADDRESS, MAX_UINT256)` transaction
    /// for `token` on behalf of `owner`, and waits for on-chain confirmation.
    fn approve_permit2(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>>;

    /// Whether this approver can read EIP-2612 nonces and tx metadata.
    ///
    /// Official `trySign*` no-ops without `readContract`. Default `false`.
    fn supports_gas_sponsoring_rpc(&self) -> bool {
        false
    }

    /// Reads `token.nonces(owner)` for an EIP-2612 permit.
    fn eip2612_nonce(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>> {
        let _ = (token, owner);
        Box::pin(async {
            Err(ClientError::PreConditionFailed(
                "eip2612 nonce read is not supported by this approver".into(),
            ))
        })
    }

    /// Reads `eth_getTransactionCount` for ERC-20 approval gas sponsoring.
    fn transaction_count(
        &self,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<u64, ClientError>> + Send + '_>> {
        let _ = owner;
        Box::pin(async {
            Err(ClientError::PreConditionFailed(
                "transaction count is not supported by this approver".into(),
            ))
        })
    }

    /// Estimates EIP-1559 fees `(max_fee_per_gas, max_priority_fee_per_gas)`.
    fn estimate_fees_per_gas(&self) -> GasSponsoringFeeFut<'_> {
        Box::pin(async {
            Err(ClientError::PreConditionFailed(
                "fee estimation is not supported by this approver".into(),
            ))
        })
    }
}

/// Built-in [`Permit2Approver`] backed by an Alloy provider.
#[cfg(all(feature = "client", feature = "client-provider"))]
pub(crate) struct BuiltinPermit2Approver<P> {
    pub(crate) provider: P,
}

#[cfg(all(feature = "client", feature = "client-provider"))]
impl<P> Permit2Approver for BuiltinPermit2Approver<P>
where
    P: alloy_provider::Provider + Send + Sync,
{
    fn check_permit2_allowance(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let (_addr, calldata) = permit2_allowance_calldata(token, owner);
            let tx = alloy_rpc_types_eth::TransactionRequest::default()
                .to(token)
                .input(calldata.into());
            let result = self.provider.call(tx).await.map_err(|e| {
                ClientError::PreConditionFailed(format!("Permit2 allowance check failed: {e}"))
            })?;
            Ok(U256::from_be_slice(&result))
        })
    }

    fn approve_permit2(
        &self,
        token: Address,
        _owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>> {
        Box::pin(async move {
            let calldata = IPermit2Approval::approveCall {
                spender: PERMIT2_ADDRESS,
                amount: U256::MAX,
            }
            .abi_encode();
            let tx = alloy_rpc_types_eth::TransactionRequest::default()
                .to(token)
                .input(calldata.into());
            let pending = self.provider.send_transaction(tx).await.map_err(|e| {
                ClientError::PreConditionFailed(format!("Permit2 approve tx failed: {e}"))
            })?;
            let receipt = pending.get_receipt().await.map_err(|e| {
                ClientError::PreConditionFailed(format!("Permit2 approve receipt failed: {e}"))
            })?;
            if !receipt.status() {
                return Err(ClientError::PreConditionFailed(
                    "Permit2 approve transaction reverted".into(),
                ));
            }
            Ok(())
        })
    }

    fn supports_gas_sponsoring_rpc(&self) -> bool {
        true
    }

    fn eip2612_nonce(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let call = IEip2612Nonces::noncesCall { owner };
            let tx = alloy_rpc_types_eth::TransactionRequest::default()
                .to(token)
                .input(call.abi_encode().into());
            let result = self.provider.call(tx).await.map_err(|e| {
                ClientError::PreConditionFailed(format!("EIP-2612 nonce read failed: {e}"))
            })?;
            Ok(U256::from_be_slice(&result))
        })
    }

    fn transaction_count(
        &self,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<u64, ClientError>> + Send + '_>> {
        Box::pin(async move {
            self.provider
                .get_transaction_count(owner)
                .await
                .map_err(|e| {
                    ClientError::PreConditionFailed(format!("transaction count failed: {e}"))
                })
        })
    }

    fn estimate_fees_per_gas(&self) -> GasSponsoringFeeFut<'_> {
        Box::pin(async move {
            Ok(self.provider.estimate_eip1559_fees().await.map_or(
                (
                    crate::erc20_approval::DEFAULT_MAX_FEE_PER_GAS,
                    crate::erc20_approval::DEFAULT_MAX_PRIORITY_FEE_PER_GAS,
                ),
                |fees| (fees.max_fee_per_gas, fees.max_priority_fee_per_gas),
            ))
        })
    }
}

#[cfg(feature = "client")]
sol! {
    /// Minimal ERC-20 interface for client-side allowance checks and approvals.
    #[allow(missing_docs, reason = "sol! generated interface")]
    interface IPermit2Approval {
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
    }

    /// EIP-2612 `nonces(address)` used when signing a gas-sponsored permit.
    #[allow(missing_docs, reason = "sol! generated interface")]
    interface IEip2612Nonces {
        function nonces(address owner) external view returns (uint256);
    }
}

/// Returns the ABI-encoded calldata for checking a token's Permit2 allowance.
#[cfg(feature = "client")]
#[must_use]
pub fn permit2_allowance_calldata(token: Address, owner: Address) -> (Address, Bytes) {
    let call = IPermit2Approval::allowanceCall {
        owner,
        spender: PERMIT2_ADDRESS,
    };
    (token, call.abi_encode().into())
}

/// Returns the ABI-encoded calldata for approving the canonical Permit2
/// contract to spend an unlimited amount of `token`.
#[cfg(feature = "client")]
#[must_use]
pub fn permit2_approval_calldata(token: Address) -> (Address, Bytes) {
    let call = IPermit2Approval::approveCall {
        spender: PERMIT2_ADDRESS,
        amount: U256::MAX,
    };
    (token, call.abi_encode().into())
}
