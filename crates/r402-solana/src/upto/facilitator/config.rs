//! Configuration for the Solana `upto` facilitator.
#![allow(
    unreachable_pub,
    missing_copy_implementations,
    reason = "config types are used by the parent facilitator module"
)]

use serde::{Deserialize, Serialize};

use crate::SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS;
use crate::upto::payment_channels::{
    DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS, DEFAULT_SETTLE_COMPUTE_UNIT_LIMIT,
    OPEN_MAX_COMPUTE_UNIT_LIMIT,
};

/// Default facilitator `maxChannelLifetimeSecs` (1 hour).
pub const DEFAULT_MAX_CHANNEL_LIFETIME_SECS: u64 = 3_600;

/// Client/facilitator clock skew allowance for `expiresAt` checks.
pub const EXPIRES_AT_CLOCK_SKEW_SECS: i64 = 60;

/// Optional configuration for the upto SVM facilitator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaUptoFacilitatorConfig {
    /// Max channel lifetime (seconds) accepted at verify/deposit.
    #[serde(default = "default_max_channel_lifetime_secs")]
    pub max_channel_lifetime_secs: u64,
    /// Maximum compute unit price in microlamports accepted on the open.
    #[serde(default = "default_max_priority_fee")]
    pub max_priority_fee_micro_lamports: u64,
    /// Maximum compute unit limit accepted on the open.
    #[serde(default = "default_max_compute_units")]
    pub max_compute_units: u32,
    /// Optional ceiling on required signatures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_required_signatures: Option<usize>,
    /// `SetComputeUnitPrice` for facilitator-submitted settlement txs.
    #[serde(default = "default_settle_price")]
    pub compute_unit_price_micro_lamports: u64,
    /// `SetComputeUnitLimit` for facilitator-submitted settlement txs.
    #[serde(default = "default_settle_cu")]
    pub settle_compute_unit_limit: u32,
}

const fn default_max_channel_lifetime_secs() -> u64 {
    DEFAULT_MAX_CHANNEL_LIFETIME_SECS
}

const fn default_max_priority_fee() -> u64 {
    SOLANA_MAX_COMPUTE_UNIT_PRICE_MICROLAMPORTS
}

const fn default_max_compute_units() -> u32 {
    OPEN_MAX_COMPUTE_UNIT_LIMIT
}

const fn default_settle_price() -> u64 {
    DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS
}

const fn default_settle_cu() -> u32 {
    DEFAULT_SETTLE_COMPUTE_UNIT_LIMIT
}

impl Default for SolanaUptoFacilitatorConfig {
    fn default() -> Self {
        Self {
            max_channel_lifetime_secs: default_max_channel_lifetime_secs(),
            max_priority_fee_micro_lamports: default_max_priority_fee(),
            max_compute_units: default_max_compute_units(),
            max_required_signatures: None,
            compute_unit_price_micro_lamports: default_settle_price(),
            settle_compute_unit_limit: default_settle_cu(),
        }
    }
}
