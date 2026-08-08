//! Path 2 smart-wallet verification (simulation + CPI outcome matching).
//!
//! Aligns with `scheme_exact_svm.md` §3.2–3.3 and the TypeScript reference
//! (`@x402/svm` `smartWalletVerification.ts`).
//!
//! Path 2 runs only when Path 1 rejects for a **recoverable layout** reason and
//! [`SolanaExactFacilitatorConfig::enable_smart_wallet_verification`] is set.

use r402_core::error::VerificationError;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;

use super::config::SolanaExactFacilitatorConfig;
use super::verify::{TransferRequirement, transfer_amount_meets_requirement};
use crate::exact::ATA_PROGRAM_PUBKEY;
use crate::exact::error::SolanaExactError;

/// SPL Token / Token-2022 `TransferChecked` instruction discriminant.
pub(super) const IX_TOKEN_TRANSFER_CHECKED: u8 = 12;

/// A `TransferChecked` observed at the top level or in a CPI trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedTransfer {
    /// Token program id.
    pub program_id: Pubkey,
    /// Transfer amount (base units).
    pub amount: u64,
    /// Mint.
    pub mint: Pubkey,
    /// Destination token account.
    pub destination: Pubkey,
    /// Authority (signer) of the transfer.
    pub authority: Pubkey,
}

/// Whether a Path 1 failure may fall through to Path 2.
///
/// Semantic failures (amount / mint / recipient / memo / self-spend / failed
/// simulation) MUST NOT fall through — matching the foundation TS path
/// selection rules.
#[must_use]
pub fn is_path1_layout_recoverable(err: &VerificationError) -> bool {
    match err {
        VerificationError::InvalidFormat(_) => true,
        VerificationError::SimulationFailed(msg) => is_recoverable_layout_message(msg),
        // Semantic and other failures — never Path 2.
        _ => false,
    }
}

fn is_recoverable_layout_message(msg: &str) -> bool {
    // Messages produced by SolanaExactError layout variants via Display.
    const NEEDLES: &[&str] = &[
        "Too few instructions",
        "Additional instructions not allowed",
        "Instruction count exceeds maximum",
        "Program not in allowed list",
        "Blocked program",
        "Invalid compute limit instruction",
        "Invalid compute price instruction",
        "Invalid token instruction",
        "Empty instruction",
        "Instruction at index",
        "No account at index",
        "CreateATA instruction not supported",
        "Fee payer included in instruction accounts",
    ];
    NEEDLES.iter().any(|n| msg.contains(n))
}

/// Parse a compiled `TransferChecked` instruction (program id + data + accounts).
///
/// Account order: `[source, mint, destination, authority, …]`.
#[must_use]
pub fn parse_transfer_checked(
    program_id: Pubkey,
    data: &[u8],
    accounts: &[Pubkey],
) -> Option<ObservedTransfer> {
    if data.first().copied()? != IX_TOKEN_TRANSFER_CHECKED {
        return None;
    }
    // layout: tag (1) + amount (u64 le) + decimals (u8)
    if data.len() < 1 + 8 + 1 {
        return None;
    }
    let amount_bytes: [u8; 8] = data.get(1..9)?.try_into().ok()?;
    let amount = u64::from_le_bytes(amount_bytes);
    if accounts.len() < 4 {
        return None;
    }
    Some(ObservedTransfer {
        program_id,
        amount,
        mint: *accounts.get(1)?,
        destination: *accounts.get(2)?,
        authority: *accounts.get(3)?,
    })
}

/// Extract `TransferChecked` instructions from the **top-level** message.
#[must_use]
pub fn extract_top_level_transfers(tx: &VersionedTransaction) -> Vec<ObservedTransfer> {
    let keys = tx.message.static_account_keys();
    let mut out = Vec::new();
    for ix in tx.message.instructions() {
        let Some(program_id) = keys.get(usize::from(ix.program_id_index)).copied() else {
            continue;
        };
        let is_token = program_id == spl_token::ID || program_id == spl_token_2022_interface::ID;
        if !is_token {
            continue;
        }
        let mut accounts = Vec::with_capacity(ix.accounts.len());
        for idx in &ix.accounts {
            if let Some(k) = keys.get(usize::from(*idx)).copied() {
                accounts.push(k);
            }
        }
        if let Some(t) = parse_transfer_checked(program_id, &ix.data, &accounts) {
            out.push(t);
        }
    }
    out
}

/// Select exactly one transfer matching requirements (spec §1.4).
///
/// # Errors
///
/// Returns [`VerificationError`] when no single matching transfer is found
/// or the fee payer would be the transfer authority.
pub fn match_required_transfer(
    transfers: &[ObservedTransfer],
    requirement: &TransferRequirement<'_>,
    fee_payer_signers: &[Pubkey],
) -> Result<ObservedTransfer, VerificationError> {
    // Self-spend protection: fee payer must not be authority.
    for t in transfers {
        if fee_payer_signers.contains(&t.authority) {
            return Err(SolanaExactError::FeePayerTransferringFunds.into());
        }
    }

    let expected_atas = expected_destination_atas(requirement.pay_to, requirement.asset);
    if expected_atas.is_empty() {
        return Err(VerificationError::InvalidFormat(
            "smart_wallet_cannot_derive_destination_ata".into(),
        ));
    }

    let required_mint = *requirement.asset.pubkey();
    let matching: Vec<_> = transfers
        .iter()
        .copied()
        .filter(|t| {
            t.mint == required_mint
                && expected_atas.contains(&t.destination)
                && transfer_amount_meets_requirement(t.amount, requirement.amount)
        })
        .collect();

    match matching.as_slice() {
        [] if transfers.is_empty() => Err(VerificationError::InvalidFormat(
            "smart_wallet_no_transfer_in_simulation".into(),
        )),
        [] => Err(VerificationError::InvalidFormat(
            "smart_wallet_transfer_mismatch".into(),
        )),
        [one] => Ok(*one),
        [first, ..] => Err(VerificationError::InvalidFormat(format!(
            "smart_wallet_multiple_matching_transfers (payer {})",
            first.authority
        ))),
    }
}

fn expected_destination_atas(
    pay_to: &crate::chain::Address,
    asset: &crate::chain::Address,
) -> Vec<Pubkey> {
    let owner = *pay_to.pubkey();
    let mint = *asset.pubkey();
    [spl_token::ID, spl_token_2022_interface::ID]
        .into_iter()
        .map(|token_program| {
            let (ata, _) = Pubkey::find_program_address(
                &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
                &ATA_PROGRAM_PUBKEY,
            );
            ata
        })
        .collect()
}

/// Operator-facing helper: Path 2 is enabled on this config.
#[must_use]
pub const fn smart_wallet_enabled(config: &SolanaExactFacilitatorConfig) -> bool {
    config.enable_smart_wallet_verification
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    reason = "unit tests use fixed-length fixture vectors"
)]
mod tests {
    use super::*;
    use crate::chain::Address;
    use solana_pubkey::Pubkey;

    fn req(amount: u64) -> (Address, Address, TransferRequirement<'static>) {
        // Fixed keys for ATA derivation stability in tests.
        let pay_to = Address::new(Pubkey::new_from_array([1u8; 32]));
        let asset = Address::new(Pubkey::new_from_array([2u8; 32]));
        // Leak for 'static in tests — unit tests only.
        let pay_to_static: &'static Address = Box::leak(Box::new(pay_to));
        let asset_static: &'static Address = Box::leak(Box::new(asset));
        let requirement = TransferRequirement {
            asset: asset_static,
            pay_to: pay_to_static,
            amount,
        };
        (*pay_to_static, *asset_static, requirement)
    }

    #[test]
    fn parse_transfer_checked_roundtrip_fields() {
        let program = spl_token::ID;
        let amount = 1_000_000u64;
        let mut data = vec![IX_TOKEN_TRANSFER_CHECKED];
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(6); // decimals
        let accounts = vec![
            Pubkey::new_from_array([10; 32]),
            Pubkey::new_from_array([11; 32]),
            Pubkey::new_from_array([12; 32]),
            Pubkey::new_from_array([13; 32]),
        ];
        let t = parse_transfer_checked(program, &data, &accounts).unwrap();
        assert_eq!(t.amount, amount);
        assert_eq!(t.mint, accounts[1]);
        assert_eq!(t.destination, accounts[2]);
        assert_eq!(t.authority, accounts[3]);
    }

    #[test]
    fn match_accepts_overpayment() {
        let (_pay_to, asset, requirement) = req(100);
        let atas = expected_destination_atas(requirement.pay_to, requirement.asset);
        let dest = *atas.first().expect("ATA derived");
        let t = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 150,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([9; 32]),
        };
        let matched = match_required_transfer(&[t], &requirement, &[]).unwrap();
        assert_eq!(matched.amount, 150);
    }

    #[test]
    fn match_rejects_underpayment() {
        let (_pay_to, asset, requirement) = req(100);
        let atas = expected_destination_atas(requirement.pay_to, requirement.asset);
        let dest = *atas.first().expect("ATA derived");
        let t = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 50,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([9; 32]),
        };
        let err = match_required_transfer(&[t], &requirement, &[]).unwrap_err();
        assert!(matches!(
            err,
            VerificationError::InvalidFormat(ref m) if m.contains("smart_wallet_transfer_mismatch")
        ));
    }

    #[test]
    fn layout_recoverable_classification() {
        assert!(!is_path1_layout_recoverable(
            &VerificationError::InvalidPaymentAmount
        ));
        assert!(is_path1_layout_recoverable(&VerificationError::SimulationFailed(
            "Program not in allowed list: Abc".into()
        )));
        assert!(!is_path1_layout_recoverable(&VerificationError::SimulationFailed(
            "some other sim failure".into()
        )));
        assert!(is_path1_layout_recoverable(&VerificationError::InvalidFormat(
            "Too few instructions".into()
        )));
    }
}
