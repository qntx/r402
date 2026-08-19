//! Wire types and constants for EVM `batch-settlement`.
//!
//! Addresses match `specs/schemes/batch-settlement/scheme_batch_settlement_evm.md`
//! and the official TS/Go packages.

use alloy_primitives::{Address, B256, Bytes, address};
use r402_core::scheme::BatchSettlementScheme;
use serde::{Deserialize, Serialize};

use crate::asset::AssetTransferMethod;
use crate::chain::{ChecksummedAddress, TokenAmount};

/// CREATE2 address of `x402BatchSettlement` (all chains).
pub const BATCH_SETTLEMENT_ADDRESS: Address =
    address!("0x4020074e9dF2ce1deE5A9C1b5c3f541D02a10003");

/// ERC-3009 deposit collector.
pub const ERC3009_DEPOSIT_COLLECTOR_ADDRESS: Address =
    address!("0x4020806089470a89826cB9fB1f4059150b550004");

/// Permit2 deposit collector.
pub const PERMIT2_DEPOSIT_COLLECTOR_ADDRESS: Address =
    address!("0x4020425FAf3B746C082C2f942b4E5159887B0005");

/// Minimum withdraw delay (15 minutes).
pub const MIN_WITHDRAW_DELAY: u64 = 900;
/// Maximum withdraw delay (30 days).
pub const MAX_WITHDRAW_DELAY: u64 = 2_592_000;

/// EIP-712 domain name.
pub const BATCH_SETTLEMENT_DOMAIN_NAME: &str = "x402 Batch Settlement";
/// EIP-712 domain version.
pub const BATCH_SETTLEMENT_DOMAIN_VERSION: &str = "1";

/// Immutable channel configuration (on-chain identity preimage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfig {
    /// Client wallet (EOA or smart wallet).
    pub payer: Address,
    /// EOA for voucher signing, or zero for EIP-1271 via payer.
    pub payer_authorizer: Address,
    /// Server payment destination.
    pub receiver: Address,
    /// Authorizes claims/refunds.
    pub receiver_authorizer: Address,
    /// ERC-20 payment token.
    pub token: Address,
    /// Withdrawal delay in seconds (`900..=2_592_000`).
    pub withdraw_delay: u64,
    /// Differentiates channels with identical parameters.
    pub salt: B256,
}

/// Cumulative voucher fields on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherFields {
    /// `bytes32` channel id.
    pub channel_id: B256,
    /// Cumulative claim ceiling (decimal string via [`TokenAmount`]).
    pub max_claimable_amount: TokenAmount,
    /// EIP-712 voucher signature.
    pub signature: Bytes,
}

/// Server `PaymentRequirements.extra` for batch-settlement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSettlementExtra {
    /// Address that authorizes claims/refunds.
    pub receiver_authorizer: ChecksummedAddress,
    /// Withdrawal delay in seconds.
    pub withdraw_delay: u64,
    /// EIP-712 token domain name.
    pub name: String,
    /// EIP-712 token domain version.
    pub version: String,
    /// Deposit transfer method; default eip3009.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_transfer_method: Option<AssetTransferMethod>,
}

impl BatchSettlementExtra {
    /// Effective transfer method.
    #[must_use]
    pub const fn transfer_method(&self) -> AssetTransferMethod {
        match self.asset_transfer_method {
            Some(m) => m,
            None => AssetTransferMethod::Eip3009,
        }
    }
}

/// Local channel accounting snapshot (server/facilitator store).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelState {
    /// On-chain-style balance tracking (deposited − refunds − withdrawn).
    pub balance: TokenAmount,
    /// Cumulative claimed on-chain.
    pub total_claimed: TokenAmount,
    /// Server running total of actual charges (voucher base).
    pub charged_cumulative: TokenAmount,
    /// Cooperative refund nonce.
    pub refund_nonce: TokenAmount,
}

impl Default for ChannelState {
    fn default() -> Self {
        use alloy_primitives::U256;
        Self {
            balance: TokenAmount::from(U256::ZERO),
            total_claimed: TokenAmount::from(U256::ZERO),
            charged_cumulative: TokenAmount::from(U256::ZERO),
            refund_nonce: TokenAmount::from(U256::ZERO),
        }
    }
}

/// Deposit authorization: exactly one of eip3009 / permit2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositErc3009Authorization {
    /// Unix lower bound.
    pub valid_after: TokenAmount,
    /// Unix upper bound.
    pub valid_before: TokenAmount,
    /// EIP-3009 nonce.
    pub salt: B256,
    /// Signature.
    pub signature: Bytes,
}

/// Permit2 deposit authorization with channel witness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositPermit2Authorization {
    /// Payer.
    pub from: Address,
    /// Token permissions.
    pub permitted: DepositTokenPermissions,
    /// Permit2 deposit collector.
    pub spender: Address,
    /// Permit2 nonce.
    pub nonce: TokenAmount,
    /// Deadline.
    pub deadline: TokenAmount,
    /// Channel-bound witness.
    pub witness: DepositWitness,
    /// Signature.
    pub signature: Bytes,
}

/// Token + amount for deposit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositTokenPermissions {
    /// Token.
    pub token: Address,
    /// Amount.
    pub amount: TokenAmount,
}

/// Deposit witness binding a Permit2 transfer to a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositWitness {
    /// Channel id.
    pub channel_id: B256,
}

/// Deposit authorization envelope (untagged: permit2 first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepositAuthorization {
    /// Permit2 path.
    Permit2 {
        /// Nested permit2 auth.
        #[serde(rename = "permit2Authorization")]
        permit2_authorization: DepositPermit2Authorization,
    },
    /// EIP-3009 path.
    Erc3009 {
        /// Nested erc3009 auth.
        #[serde(rename = "erc3009Authorization")]
        erc3009_authorization: DepositErc3009Authorization,
    },
}

/// Discriminated payment payload for request path (client → server).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BatchSettlementPayload {
    /// First deposit + initial voucher.
    Deposit {
        /// Channel config.
        channel_config: ChannelConfig,
        /// Opening voucher.
        voucher: VoucherFields,
        /// Deposit amount + authorization (boxed: largest variant fields).
        deposit: Box<DepositBody>,
    },
    /// Subsequent cumulative voucher.
    Voucher {
        /// Channel config.
        channel_config: ChannelConfig,
        /// Signed voucher.
        voucher: VoucherFields,
    },
    /// Cooperative refund request.
    Refund {
        /// Channel config.
        channel_config: ChannelConfig,
        /// Zero-charge voucher (max = charged cumulative).
        voucher: VoucherFields,
        /// Optional refund amount.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<TokenAmount>,
    },
}

/// Deposit amount + auth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositBody {
    /// Deposit amount.
    pub amount: TokenAmount,
    /// Token authorization.
    pub authorization: DepositAuthorization,
}

impl BatchSettlementPayload {
    /// Channel config shared by all variants.
    #[must_use]
    pub const fn channel_config(&self) -> &ChannelConfig {
        match self {
            Self::Deposit { channel_config, .. }
            | Self::Voucher { channel_config, .. }
            | Self::Refund { channel_config, .. } => channel_config,
        }
    }

    /// Voucher fields shared by all variants.
    #[must_use]
    pub const fn voucher(&self) -> &VoucherFields {
        match self {
            Self::Deposit { voucher, .. }
            | Self::Voucher { voucher, .. }
            | Self::Refund { voucher, .. } => voucher,
        }
    }
}

/// Wire aliases.
pub mod v2 {
    use r402_core::wire as proto_v2;

    use super::{BatchSettlementExtra, BatchSettlementPayload, BatchSettlementScheme};
    use crate::chain::{ChecksummedAddress, TokenAmount};

    /// Typed verify request.
    pub type VerifyRequest = proto_v2::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;
    /// Typed settle request.
    pub type SettleRequest = VerifyRequest;
    /// Payment payload.
    pub type PaymentPayload = proto_v2::PaymentPayload<PaymentRequirements, BatchSettlementPayload>;
    /// Payment requirements.
    pub type PaymentRequirements = proto_v2::PaymentRequirements<
        BatchSettlementScheme,
        TokenAmount,
        ChecksummedAddress,
        BatchSettlementExtra,
    >;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            BATCH_SETTLEMENT_ADDRESS.to_checksum(None),
            "0x4020074e9dF2ce1deE5A9C1b5c3f541D02a10003"
        );
    }
}
