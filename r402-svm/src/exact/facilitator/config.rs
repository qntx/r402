//! Configuration for the Solana exact scheme facilitator.
//!
//! Controls transaction verification behavior, including support for
//! additional instructions from third-party wallets like Phantom.

use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

use crate::chain::Address;
use crate::exact::{PHANTOM_LIGHTHOUSE_PROGRAM, SOLFLARE_LIGHTHOUSE_PROGRAM, SPL_MEMO_PROGRAM};

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
    /// Default: 6 (per x402 v2 spec §SVM exact scheme recommendation:
    /// 1 `TransferChecked` + 2 compute budget + 1 optional memo +
    /// 1-2 optional lighthouse instructions).
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
}

const fn default_allow_additional_instructions() -> bool {
    true
}

const fn default_max_instruction_count() -> usize {
    6
}

fn default_allowed_program_ids() -> Vec<Address> {
    vec![
        Address::new(*PHANTOM_LIGHTHOUSE_PROGRAM),
        Address::new(*SOLFLARE_LIGHTHOUSE_PROGRAM),
        Address::new(SPL_MEMO_PROGRAM),
    ]
}

impl Default for SolanaExactFacilitatorConfig {
    fn default() -> Self {
        Self {
            allow_additional_instructions: default_allow_additional_instructions(),
            max_instruction_count: default_max_instruction_count(),
            allowed_program_ids: default_allowed_program_ids(),
            blocked_program_ids: Vec::new(),
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
