//! Path 2 smart-wallet verification (simulation + CPI outcome matching).
//!
//! Aligns with `scheme_exact_svm.md` §3.2–3.4 and the TypeScript reference
//! (`@x402/svm` `smartWalletVerification.ts`).
//!
//! Path 2 runs only when Path 1 rejects for a **recoverable layout** reason and
//! [`SolanaExactFacilitatorConfig::enable_smart_wallet_verification`] is set.

use std::str::FromStr;
use std::time::Duration;

use r402_protocol::VerificationError;
use solana_client::rpc_response::{
    ParsedInstruction, UiInnerInstructions, UiInstruction, UiLoadedAddresses, UiParsedInstruction,
};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

use super::config::{MAX_INSTRUCTION_COUNT, MIN_INSTRUCTION_COUNT, SolanaExactFacilitatorConfig};
use super::verify::{TransferRequirement, transfer_amount_meets_requirement};
use crate::chain::provider::SolanaChainProviderLike;
use crate::exact::ATA_PROGRAM_PUBKEY;
use crate::exact::error::SolanaExactError;

const ERR_CANNOT_DERIVE_ATA: &str = "invalid_exact_svm_smart_wallet_cannot_derive_destination_ata";
const ERR_NO_TRANSFER: &str = "invalid_exact_svm_smart_wallet_no_transfer_in_simulation";
const ERR_TRANSFER_MISMATCH: &str = "invalid_exact_svm_smart_wallet_transfer_mismatch";
const ERR_MULTIPLE: &str = "invalid_exact_svm_smart_wallet_multiple_matching_transfers";

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
            ERR_CANNOT_DERIVE_ATA.into(),
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
        [] if transfers.is_empty() => Err(VerificationError::InvalidFormat(ERR_NO_TRANSFER.into())),
        [] => Err(VerificationError::InvalidFormat(
            ERR_TRANSFER_MISMATCH.into(),
        )),
        [one] => Ok(*one),
        _ => Err(VerificationError::InvalidFormat(ERR_MULTIPLE.into())),
    }
}

fn is_token_program(program_id: &Pubkey) -> bool {
    *program_id == spl_token::ID || *program_id == spl_token_2022_interface::ID
}

/// Static keys plus simulation-loaded ALT addresses (writable, then readonly).
#[must_use]
pub(super) fn resolved_account_keys(
    tx: &VersionedTransaction,
    loaded: &UiLoadedAddresses,
) -> Vec<Pubkey> {
    let mut keys = tx.message.static_account_keys().to_vec();
    for addr in loaded.writable.iter().chain(&loaded.readonly) {
        if let Ok(pk) = Pubkey::from_str(addr) {
            keys.push(pk);
        }
    }
    keys
}

/// Extract `TransferChecked` from simulation / confirmed-tx inner instructions.
///
/// Handles compiled (base58 + account indices), jsonParsed, and partially
/// decoded RPC shapes — official `extractTransfersFromInnerInstructions`.
#[must_use]
pub fn extract_transfers_from_inner_instructions(
    inner_instructions: &[UiInnerInstructions],
    account_keys: &[Pubkey],
) -> Vec<ObservedTransfer> {
    let mut out = Vec::new();
    for group in inner_instructions {
        for ix in &group.instructions {
            if let Some(t) = extract_one_inner(ix, account_keys) {
                out.push(t);
            }
        }
    }
    out
}

fn extract_one_inner(ix: &UiInstruction, account_keys: &[Pubkey]) -> Option<ObservedTransfer> {
    match ix {
        UiInstruction::Compiled(compiled) => {
            let program_id = *account_keys.get(usize::from(compiled.program_id_index))?;
            if !is_token_program(&program_id) {
                return None;
            }
            let data = bs58::decode(&compiled.data).into_vec().ok()?;
            let mut accounts = Vec::with_capacity(compiled.accounts.len());
            for idx in &compiled.accounts {
                accounts.push(*account_keys.get(usize::from(*idx))?);
            }
            parse_transfer_checked(program_id, &data, &accounts)
        }
        UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) => {
            parse_parsed_transfer_checked(parsed)
        }
        UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(partial)) => {
            let program_id = Pubkey::from_str(&partial.program_id).ok()?;
            if !is_token_program(&program_id) {
                return None;
            }
            let data = bs58::decode(&partial.data).into_vec().ok()?;
            let accounts: Option<Vec<Pubkey>> = partial
                .accounts
                .iter()
                .map(|a| Pubkey::from_str(a).ok())
                .collect();
            parse_transfer_checked(program_id, &data, &accounts?)
        }
    }
}

fn parse_parsed_transfer_checked(parsed: &ParsedInstruction) -> Option<ObservedTransfer> {
    let program_id = Pubkey::from_str(&parsed.program_id).ok()?;
    if !is_token_program(&program_id) {
        return None;
    }
    if parsed.parsed.get("type")?.as_str()? != "transferChecked" {
        return None;
    }
    let info = parsed.parsed.get("info")?;
    let amount = info
        .get("tokenAmount")
        .and_then(|token_amount| token_amount.get("amount"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| info.get("amount").and_then(serde_json::Value::as_str))
        .and_then(|s| s.parse().ok())?;
    let mint = Pubkey::from_str(info.get("mint")?.as_str()?).ok()?;
    let destination = Pubkey::from_str(info.get("destination")?.as_str()?).ok()?;
    let authority = info
        .get("authority")
        .or_else(|| info.get("owner"))
        .and_then(serde_json::Value::as_str)
        .and_then(|s| Pubkey::from_str(s).ok())?;
    Some(ObservedTransfer {
        program_id,
        amount,
        mint,
        destination,
        authority,
    })
}

/// Path 1 positional layout: 3–7 ixs with `TransferChecked` at index 2.
#[must_use]
pub fn has_static_transfer_layout(tx: &VersionedTransaction) -> bool {
    let instructions = tx.message.instructions();
    if instructions.len() < MIN_INSTRUCTION_COUNT || instructions.len() > MAX_INSTRUCTION_COUNT {
        return false;
    }
    let keys = tx.message.static_account_keys();
    let Some(ix) = instructions.get(2) else {
        return false;
    };
    let Some(program_id) = keys.get(usize::from(ix.program_id_index)).copied() else {
        return false;
    };
    if !is_token_program(&program_id) {
        return false;
    }
    let mut accounts = Vec::with_capacity(ix.accounts.len());
    for idx in &ix.accounts {
        let Some(key) = keys.get(usize::from(*idx)).copied() else {
            return false;
        };
        accounts.push(key);
    }
    parse_transfer_checked(program_id, &ix.data, &accounts).is_some()
}

/// How post-settle TOCTOU verified the on-chain transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostSettlementMethod {
    /// Confirmed-tx inner instructions contained a matching `TransferChecked`.
    InnerInstructions,
    /// Destination ATA balance increased by at least the required amount.
    BalanceDelta,
    /// Neither method could decide (RPC lag / missing caps).
    Unverified,
}

/// Result of [`verify_post_settlement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostSettlementCheck {
    /// Whether the required transfer was confirmed on-chain.
    pub verified: bool,
    /// Which method produced [`Self::verified`].
    pub method: PostSettlementMethod,
}

/// Post-settle TOCTOU check (spec §3.4).
///
/// Prefers confirmed inner instructions. Falls back to ATA balance delta when
/// the RPC has not indexed inners. `unverified` is a settle Failure.
pub async fn verify_post_settlement<P: SolanaChainProviderLike>(
    provider: &P,
    signature: &Signature,
    requirement: &TransferRequirement<'_>,
    fee_payer_signers: &[Pubkey],
    balance_before: Option<u64>,
    known_destination: Option<Pubkey>,
) -> PostSettlementCheck {
    for attempt in 0u32..3 {
        if let Ok(Some(inner)) = provider.get_confirmed_inner_instructions(signature).await {
            return PostSettlementCheck {
                verified: post_settle_inners_match(
                    &inner,
                    requirement,
                    fee_payer_signers,
                    known_destination,
                ),
                method: PostSettlementMethod::InnerInstructions,
            };
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt + 1))).await;
        }
    }

    let Some(before) = balance_before else {
        return PostSettlementCheck {
            verified: false,
            method: PostSettlementMethod::Unverified,
        };
    };

    if let Some(dest) = known_destination {
        return match provider.get_token_account_balance(&dest).await {
            Ok(Some(after)) => PostSettlementCheck {
                verified: after.saturating_sub(before) >= requirement.amount,
                method: PostSettlementMethod::BalanceDelta,
            },
            Ok(None) | Err(_) => PostSettlementCheck {
                verified: false,
                method: PostSettlementMethod::Unverified,
            },
        };
    }

    let atas = expected_destination_atas(requirement.pay_to, requirement.asset);
    let mut any = false;
    for ata in atas {
        if let Ok(Some(after)) = provider.get_token_account_balance(&ata).await {
            any = true;
            if after.saturating_sub(before) >= requirement.amount {
                return PostSettlementCheck {
                    verified: true,
                    method: PostSettlementMethod::BalanceDelta,
                };
            }
        }
    }
    if any {
        PostSettlementCheck {
            verified: false,
            method: PostSettlementMethod::BalanceDelta,
        }
    } else {
        PostSettlementCheck {
            verified: false,
            method: PostSettlementMethod::Unverified,
        }
    }
}

fn post_settle_inners_match(
    inner: &[UiInnerInstructions],
    requirement: &TransferRequirement<'_>,
    fee_payer_signers: &[Pubkey],
    known_destination: Option<Pubkey>,
) -> bool {
    let transfers = extract_transfers_from_inner_instructions(inner, &[]);
    let expected = known_destination.map_or_else(
        || expected_destination_atas(requirement.pay_to, requirement.asset),
        |dest| vec![dest],
    );
    let mint = *requirement.asset.pubkey();
    transfers.iter().any(|t| {
        t.mint == mint
            && expected.contains(&t.destination)
            && transfer_amount_meets_requirement(t.amount, requirement.amount)
            && !fee_payer_signers.contains(&t.authority)
    })
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
    use solana_pubkey::Pubkey;

    use super::*;
    use crate::chain::Address;

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
        assert!(is_path1_layout_recoverable(
            &VerificationError::SimulationFailed("Program not in allowed list: Abc".into())
        ));
        assert!(!is_path1_layout_recoverable(
            &VerificationError::SimulationFailed("some other sim failure".into())
        ));
        assert!(is_path1_layout_recoverable(
            &VerificationError::InvalidFormat("Too few instructions".into())
        ));
    }

    fn transfer_data(amount: u64) -> Vec<u8> {
        let mut data = vec![IX_TOKEN_TRANSFER_CHECKED];
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(6);
        data
    }

    fn parsed_inner(t: &ObservedTransfer) -> Vec<UiInnerInstructions> {
        vec![UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Parsed(UiParsedInstruction::Parsed(
                ParsedInstruction {
                    program: "spl-token".into(),
                    program_id: t.program_id.to_string(),
                    parsed: serde_json::json!({
                        "type": "transferChecked",
                        "info": {
                            "mint": t.mint.to_string(),
                            "destination": t.destination.to_string(),
                            "authority": t.authority.to_string(),
                            "tokenAmount": { "amount": t.amount.to_string() }
                        }
                    }),
                    stack_height: None,
                },
            ))],
        }]
    }

    #[test]
    fn extract_compiled_inner_transfer_checked() {
        let program = spl_token::ID;
        let accounts = [
            Pubkey::new_from_array([10; 32]),
            Pubkey::new_from_array([11; 32]),
            Pubkey::new_from_array([12; 32]),
            Pubkey::new_from_array([13; 32]),
        ];
        let mut keys = vec![program];
        keys.extend_from_slice(&accounts);
        let data = bs58::encode(transfer_data(1_000)).into_string();
        let inner = vec![UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Compiled(
                solana_client::rpc_response::UiCompiledInstruction {
                    program_id_index: 0,
                    accounts: vec![1, 2, 3, 4],
                    data,
                    stack_height: None,
                },
            )],
        }];
        let extracted = extract_transfers_from_inner_instructions(&inner, &keys);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].amount, 1_000);
        assert_eq!(extracted[0].mint, accounts[1]);
        assert_eq!(extracted[0].destination, accounts[2]);
        assert_eq!(extracted[0].authority, accounts[3]);
    }

    #[test]
    fn extract_parsed_inner_transfer_checked() {
        let t = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 42,
            mint: Pubkey::new_from_array([2; 32]),
            destination: Pubkey::new_from_array([3; 32]),
            authority: Pubkey::new_from_array([4; 32]),
        };
        let extracted = extract_transfers_from_inner_instructions(&parsed_inner(&t), &[]);
        assert_eq!(extracted, vec![t]);
    }

    #[test]
    fn path2_matches_cpi_only_transfer() {
        let (_pay_to, asset, requirement) = req(100);
        let dest = expected_destination_atas(requirement.pay_to, requirement.asset)[0];
        let t = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 100,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([9; 32]),
        };
        let transfers = extract_transfers_from_inner_instructions(&parsed_inner(&t), &[]);
        let matched = match_required_transfer(&transfers, &requirement, &[]).unwrap();
        assert_eq!(matched.destination, dest);
        assert_eq!(matched.amount, 100);
    }

    #[test]
    fn post_settle_inner_match_and_miss() {
        let (_pay_to, asset, requirement) = req(100);
        let dest = expected_destination_atas(requirement.pay_to, requirement.asset)[0];
        let t = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 100,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([9; 32]),
        };
        assert!(post_settle_inners_match(
            &parsed_inner(&t),
            &requirement,
            &[],
            Some(dest)
        ));
        let miss = ObservedTransfer {
            destination: Pubkey::new_from_array([8; 32]),
            ..t
        };
        assert!(!post_settle_inners_match(
            &parsed_inner(&miss),
            &requirement,
            &[],
            Some(dest)
        ));
        assert!(!post_settle_inners_match(
            &[],
            &requirement,
            &[],
            Some(dest)
        ));
    }

    #[test]
    fn has_static_layout_requires_positional_transfer_checked() {
        use solana_message::{Message, VersionedMessage};
        use solana_signature::Signature as Sig;
        use solana_transaction::Instruction;

        let payer = Pubkey::new_from_array([1u8; 32]);
        let short = VersionedTransaction {
            signatures: vec![Sig::default()],
            message: VersionedMessage::Legacy(Message::new(
                &[Instruction {
                    program_id: Pubkey::new_from_array([9; 32]),
                    accounts: Vec::new(),
                    data: vec![0],
                }],
                Some(&payer),
            )),
        };
        assert!(!has_static_transfer_layout(&short));
    }
}
