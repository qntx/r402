//! Configuration for the Solana exact scheme facilitator.
//!
//! Controls transaction verification behavior, including support for
//! additional instructions from third-party wallets like Phantom.

use serde::{Deserialize, Serialize};
use solana_pubkey::{Pubkey, pubkey};

use crate::chain::Address;
use crate::exact::{PHANTOM_LIGHTHOUSE_PROGRAM, SOLFLARE_LIGHTHOUSE_PROGRAM, SPL_MEMO_PROGRAM};

/// Official Path 1 instruction-count floor (`scheme_exact_svm.md` §3.1).
pub const MIN_INSTRUCTION_COUNT: usize = 3;

/// Official Path 1 instruction-count ceiling (Phantom third Lighthouse, #2097).
pub const MAX_INSTRUCTION_COUNT: usize = 7;

/// Clamp an operator-supplied instruction cap onto the official 3..=7 range.
#[must_use]
pub const fn clamp_max_instruction_count(n: usize) -> usize {
    if n < MIN_INSTRUCTION_COUNT {
        MIN_INSTRUCTION_COUNT
    } else if n > MAX_INSTRUCTION_COUNT {
        MAX_INSTRUCTION_COUNT
    } else {
        n
    }
}

fn deserialize_max_instruction_count<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(clamp_max_instruction_count(usize::deserialize(
        deserializer,
    )?))
}

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
    ///
    /// Default: 7 (official Path 1 cap: 2 compute budget + `TransferChecked` +
    /// memo + up to 3 Lighthouse). Values outside 3..=7 are clamped.
    #[serde(
        default = "default_max_instruction_count",
        deserialize_with = "deserialize_max_instruction_count"
    )]
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

    /// Enable Path 2 smart-wallet verification (spec §3.2).
    ///
    /// When `true`, Path 1 layout failures fall through to simulation-based
    /// CPI outcome matching (aligned with TS `enableSmartWalletVerification`).
    /// Default: `false` (Path 1 only), matching the foundation TypeScript
    /// facilitator default.
    #[serde(default)]
    pub enable_smart_wallet_verification: bool,

    /// Path 2 compute-unit ceiling. `None` uses [`DEFAULT_SMART_WALLET_MAX_COMPUTE_UNITS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_wallet_max_compute_units: Option<u32>,

    /// Path 2 priority-fee ceiling in microlamports.
    /// `None` uses [`DEFAULT_SMART_WALLET_MAX_PRIORITY_FEE_MICROLAMPORTS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_wallet_max_priority_fee_micro_lamports: Option<u64>,

    /// Path 2 top-level wallet-program allowlist. `None` uses
    /// [`DEFAULT_SMART_WALLET_ALLOWED_PROGRAMS`]. `ComputeBudget` and Memo are
    /// exempt. Empty `Some` admits no wallet programs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_wallet_allowed_programs: Option<Vec<Address>>,
}

/// Official Path 2 default compute-unit cap.
pub const DEFAULT_SMART_WALLET_MAX_COMPUTE_UNITS: u32 = 400_000;

/// Official Path 2 default priority-fee cap (microlamports).
pub const DEFAULT_SMART_WALLET_MAX_PRIORITY_FEE_MICROLAMPORTS: u64 = 50_000;

/// Official default Path 2 wallet programs (Squads / Swig / Governance / Metaplex / Lighthouse).
pub static DEFAULT_SMART_WALLET_ALLOWED_PROGRAMS: &[Pubkey] = &[
    pubkey!("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf"),
    pubkey!("SMRTzfY6DfH5ik3TKiyLFfXexV8uSG3d2UksSCYdunG"),
    pubkey!("SWiGmQedKzMz1tiTqoJCWeGDnGXfNBp2PkXLkpCAtQo"),
    pubkey!("swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB"),
    pubkey!("GovER5Lthms3bLBqWub97yVrMmEogzX7xNjdXpPPCVZw"),
    pubkey!("CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d"),
    pubkey!("L2TExMFKdjpN9kozasaurPirfHy9P8sbXoAN1qA3S95"),
];

const fn default_allow_additional_instructions() -> bool {
    true
}

const fn default_max_instruction_count() -> usize {
    MAX_INSTRUCTION_COUNT
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
            enable_smart_wallet_verification: false,
            smart_wallet_max_compute_units: None,
            smart_wallet_max_priority_fee_micro_lamports: None,
            smart_wallet_allowed_programs: None,
        }
    }
}

impl SolanaExactFacilitatorConfig {
    /// Enables Path 2 smart-wallet verification (CPI / simulation outcome path).
    #[must_use]
    pub const fn with_smart_wallet_verification(mut self, enabled: bool) -> Self {
        self.enable_smart_wallet_verification = enabled;
        self
    }

    /// Path 2 compute-unit cap (official default `400_000`).
    #[must_use]
    pub const fn path2_max_compute_units(&self) -> u32 {
        match self.smart_wallet_max_compute_units {
            Some(n) => n,
            None => DEFAULT_SMART_WALLET_MAX_COMPUTE_UNITS,
        }
    }

    /// Path 2 priority-fee cap in microlamports (official default `50_000`).
    #[must_use]
    pub const fn path2_max_priority_fee_micro_lamports(&self) -> u64 {
        match self.smart_wallet_max_priority_fee_micro_lamports {
            Some(n) => n,
            None => DEFAULT_SMART_WALLET_MAX_PRIORITY_FEE_MICROLAMPORTS,
        }
    }

    /// Path 2 wallet-program allowlist (`ComputeBudget` and Memo are exempt).
    #[must_use]
    pub fn path2_allowed_programs(&self) -> Vec<Pubkey> {
        self.smart_wallet_allowed_programs.as_ref().map_or_else(
            || DEFAULT_SMART_WALLET_ALLOWED_PROGRAMS.to_vec(),
            |programs| programs.iter().map(|a| *a.pubkey()).collect(),
        )
    }

    /// Operator Path 2 limits must be usable as limits (official `assertSmartWalletLimits`).
    ///
    /// # Errors
    ///
    /// Returns a message when a configured ceiling is below its minimum.
    pub fn assert_smart_wallet_limits(&self) -> Result<(), String> {
        if let Some(units) = self.smart_wallet_max_compute_units
            && units < 1
        {
            return Err(format!(
                "smartWalletMaxComputeUnits must be a safe integer >= 1, received {units}"
            ));
        }
        Ok(())
    }

    /// Official 3..=7 cap, even if [`Self::max_instruction_count`] was mutated.
    #[must_use]
    pub const fn effective_max_instruction_count(&self) -> usize {
        clamp_max_instruction_count(self.max_instruction_count)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_instruction_count_is_7() {
        assert_eq!(
            SolanaExactFacilitatorConfig::default().max_instruction_count,
            7
        );
    }

    #[test]
    fn clamp_rejects_opening_the_official_cap() {
        assert_eq!(clamp_max_instruction_count(8), 7);
        assert_eq!(clamp_max_instruction_count(2), 3);
        assert_eq!(clamp_max_instruction_count(7), 7);
        assert_eq!(clamp_max_instruction_count(3), 3);
    }

    #[test]
    fn serde_clamps_max_instruction_count() {
        let high: SolanaExactFacilitatorConfig =
            serde_json::from_value(serde_json::json!({ "maxInstructionCount": 8 })).unwrap();
        assert_eq!(high.max_instruction_count, 7);
        let low: SolanaExactFacilitatorConfig =
            serde_json::from_value(serde_json::json!({ "maxInstructionCount": 1 })).unwrap();
        assert_eq!(low.max_instruction_count, 3);
    }

    #[test]
    fn effective_max_clamps_mutated_field() {
        let config = SolanaExactFacilitatorConfig {
            max_instruction_count: 8,
            ..SolanaExactFacilitatorConfig::default()
        };
        assert_eq!(config.effective_max_instruction_count(), 7);
    }

    #[test]
    fn path2_defaults_match_official() {
        assert_eq!(DEFAULT_SMART_WALLET_MAX_COMPUTE_UNITS, 400_000);
        assert_eq!(DEFAULT_SMART_WALLET_MAX_PRIORITY_FEE_MICROLAMPORTS, 50_000);
        assert_eq!(DEFAULT_SMART_WALLET_ALLOWED_PROGRAMS.len(), 7);
        let config = SolanaExactFacilitatorConfig::default();
        assert_eq!(config.path2_max_compute_units(), 400_000);
        assert!(config.assert_smart_wallet_limits().is_ok());
        let bad = SolanaExactFacilitatorConfig {
            smart_wallet_max_compute_units: Some(0),
            ..SolanaExactFacilitatorConfig::default()
        };
        assert!(bad.assert_smart_wallet_limits().is_err());
    }
}
