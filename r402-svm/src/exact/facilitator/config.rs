//! Configuration for the Solana exact scheme facilitator.
//!
//! Controls transaction verification behavior, including support for
//! additional instructions from third-party wallets like Phantom.

use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

use crate::chain::Address;
use crate::exact::PHANTOM_LIGHTHOUSE_PROGRAM;

/// Configuration for Solana Exact Facilitator (shared by V1 and V2).
///
/// Controls transaction verification behavior, including support for
/// additional instructions from third-party wallets like Phantom.
///
/// By default, the Phantom Lighthouse program is allowed to support
/// Phantom wallet users on mainnet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaExactFacilitatorConfig {
    /// Allow additional instructions beyond the required ones.
    /// Default: true (to support Phantom Lighthouse)
    #[serde(default = "default_allow_additional_instructions")]
    pub allow_additional_instructions: bool,

    /// Maximum number of instructions allowed in a transaction.
    /// Default: 10
    #[serde(default = "default_max_instruction_count")]
    pub max_instruction_count: usize,

    /// Explicitly allowed program IDs for additional instructions.
    /// Only checked if `allow_additional_instructions` is true.
    ///
    /// Default: [Phantom Lighthouse program]
    ///
    /// SECURITY: If this list is empty and `allow_additional_instructions` is true,
    /// ALL additional instructions will be rejected. You must explicitly whitelist
    /// the programs you want to allow.
    #[serde(default = "default_allowed_program_ids")]
    pub allowed_program_ids: Vec<Address>,

    /// Blocked program IDs (always rejected, takes precedence over allowed).
    #[serde(default)]
    pub blocked_program_ids: Vec<Address>,

    /// SECURITY: Require fee payer is NOT present in any instruction's accounts.
    /// Default: true - strongly recommended to keep this enabled
    #[serde(default = "default_require_fee_payer_not_in_instructions")]
    pub require_fee_payer_not_in_instructions: bool,
}

const fn default_allow_additional_instructions() -> bool {
    true
}

const fn default_max_instruction_count() -> usize {
    10
}

fn default_allowed_program_ids() -> Vec<Address> {
    vec![Address::new(*PHANTOM_LIGHTHOUSE_PROGRAM)]
}

const fn default_require_fee_payer_not_in_instructions() -> bool {
    true
}

impl Default for SolanaExactFacilitatorConfig {
    fn default() -> Self {
        Self {
            allow_additional_instructions: default_allow_additional_instructions(),
            max_instruction_count: default_max_instruction_count(),
            allowed_program_ids: default_allowed_program_ids(),
            blocked_program_ids: Vec::new(),
            require_fee_payer_not_in_instructions: default_require_fee_payer_not_in_instructions(),
        }
    }
}

impl SolanaExactFacilitatorConfig {
    /// Check if a program ID is in the blocked list.
    #[must_use]
    pub fn is_blocked(&self, program_id: &Pubkey) -> bool {
        self.blocked_program_ids
            .iter()
            .any(|addr| addr.pubkey() == program_id)
    }

    /// Check if a program ID is in the allowed list.
    ///
    /// SECURITY: If the allowed list is empty, NO programs are allowed.
    #[must_use]
    pub fn is_allowed(&self, program_id: &Pubkey) -> bool {
        self.allowed_program_ids
            .iter()
            .any(|addr| addr.pubkey() == program_id)
    }
}
