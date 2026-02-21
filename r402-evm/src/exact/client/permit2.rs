//! Permit2 auto-approve support for the EIP-155 exact client.
//!
//! This module provides the [`Permit2Approver`] trait for automatic ERC-20
//! allowance management, along with ABI calldata helpers for custom
//! implementations.

use std::future::Future;
use std::pin::Pin;

use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{SolCall, sol};
use r402::scheme::ClientError;

use crate::exact::PERMIT2_ADDRESS;

/// Abstraction for on-chain interactions needed by the Permit2 auto-approve flow.
///
/// Implement this trait to enable automatic Permit2 allowance management
/// in [`Eip155ExactClient`](super::Eip155ExactClient). When an approver is provided via
/// [`Eip155ExactClientBuilder::approver`](super::Eip155ExactClientBuilder::approver), the client will:
///
/// 1. **Before each Permit2 payment**, call [`check_permit2_allowance`](Self::check_permit2_allowance)
///    to query the current ERC-20 allowance granted to the canonical Permit2 contract.
/// 2. **If the allowance is insufficient**, call [`approve_permit2`](Self::approve_permit2)
///    to send an on-chain `approve(Permit2, MAX_UINT256)` transaction.
/// 3. **Proceed with normal Permit2 EIP-712 signing** once allowance is confirmed.
///
/// This eliminates the manual approve step that Permit2 otherwise requires,
/// giving users the same zero-friction experience as EIP-3009 tokens (like USDC).
///
/// # Gas Costs
///
/// The `approve` transaction costs approximately 46,000 gas (~$0.01–0.10
/// depending on the chain). This cost is borne by the token owner (the
/// signing wallet) and only occurs **once per token** — subsequent payments
/// reuse the existing unlimited allowance.
///
/// # Calldata Helpers
///
/// If you are building a custom implementation, the helper functions
/// [`permit2_allowance_calldata`] and [`permit2_approval_calldata`] generate
/// the raw ABI-encoded calldata for the underlying `eth_call` / `eth_sendTransaction`.
///
/// # Example
///
/// ```ignore
/// use r402_evm::exact::client::{Eip155ExactClient, Permit2Approver};
///
/// struct MyProvider { /* RPC + signer */ }
///
/// impl Permit2Approver for MyProvider {
///     fn check_permit2_allowance(&self, token: Address, owner: Address)
///         -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>>
///     {
///         Box::pin(async move { /* eth_call: token.allowance(owner, PERMIT2) */ todo!() })
///     }
///
///     fn approve_permit2(&self, token: Address, owner: Address)
///         -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>>
///     {
///         Box::pin(async move { /* send tx: token.approve(PERMIT2, MAX) */ todo!() })
///     }
/// }
///
/// let client = Eip155ExactClient::builder(signer)
///     .approver(MyProvider { /* ... */ })
///     .build();
/// ```
pub trait Permit2Approver: Send + Sync {
    /// Queries the current ERC-20 allowance that `owner` has granted to the
    /// canonical Permit2 contract for the given `token`.
    ///
    /// Equivalent to `token.allowance(owner, PERMIT2_ADDRESS)` via `eth_call`
    /// (read-only, no gas cost).
    fn check_permit2_allowance(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>>;

    /// Sends an ERC-20 `approve(PERMIT2_ADDRESS, MAX_UINT256)` transaction
    /// for `token` on behalf of `owner`, and waits for on-chain confirmation.
    ///
    /// This is a **write** operation that costs gas. Implementations should
    /// ensure the transaction is sent from the `owner` address.
    fn approve_permit2(
        &self,
        token: Address,
        owner: Address,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>>;
}

/// Built-in [`Permit2Approver`] backed by an Alloy provider.
///
/// Created automatically when calling
/// [`Eip155ExactClientBuilder::provider`](super::Eip155ExactClientBuilder::provider).
/// Users never need to interact with this type directly.
#[cfg(feature = "client-provider")]
pub(super) struct BuiltinPermit2Approver<P> {
    pub(super) provider: P,
}

#[cfg(feature = "client-provider")]
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
}

sol! {
    /// Minimal ERC-20 interface for client-side allowance checks and approvals.
    #[allow(missing_docs)]
    interface IPermit2Approval {
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
    }
}

/// Returns the ABI-encoded calldata for checking a token's Permit2 allowance.
///
/// The returned tuple `(token_address, calldata)` can be used with any EVM
/// provider's `eth_call` to check whether `owner` has approved the canonical
/// Permit2 contract to spend their tokens.
///
/// # Automatic Alternative
///
/// If you use [`Eip155ExactClient::builder`](super::Eip155ExactClient::builder) with a [`Permit2Approver`],
/// allowance checks and approvals are handled automatically. This function
/// is primarily useful for custom [`Permit2Approver`] implementations.
///
/// Mirrors Go SDK's `GetPermit2AllowanceReadParams`.
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
///
/// The returned tuple `(token_address, calldata)` represents a transaction
/// the user must send (paying gas) before using the Permit2 payment flow.
///
/// # Automatic Alternative
///
/// If you use [`Eip155ExactClient::builder`](super::Eip155ExactClient::builder) with a [`Permit2Approver`],
/// this approval is sent automatically when needed. This function is
/// primarily useful for custom [`Permit2Approver`] implementations.
///
/// Mirrors Go SDK's `CreatePermit2ApprovalTxData`.
#[must_use]
pub fn permit2_approval_calldata(token: Address) -> (Address, Bytes) {
    let call = IPermit2Approval::approveCall {
        spender: PERMIT2_ADDRESS,
        amount: U256::MAX,
    };
    (token, call.abi_encode().into())
}
