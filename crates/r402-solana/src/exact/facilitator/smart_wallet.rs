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
const ERR_FEE_PAYER_NOT_ISOLATED: &str = "invalid_exact_svm_smart_wallet_fee_payer_not_isolated";
const ERR_MALFORMED_COMPUTE_BUDGET: &str =
    "invalid_exact_svm_smart_wallet_malformed_compute_budget";
const ERR_MALFORMED_COMPUTE_LIMIT: &str = "invalid_exact_svm_smart_wallet_malformed_compute_limit";
const ERR_MALFORMED_COMPUTE_PRICE: &str = "invalid_exact_svm_smart_wallet_malformed_compute_price";
const ERR_COMPUTE_UNITS_TOO_HIGH: &str = "invalid_exact_svm_smart_wallet_compute_units_too_high";
const ERR_PRIORITY_FEE_TOO_HIGH: &str = "invalid_exact_svm_smart_wallet_priority_fee_too_high";
const ERR_UNSUPPORTED_COMPUTE_BUDGET: &str =
    "invalid_exact_svm_smart_wallet_unsupported_compute_budget_instruction";
const ERR_ALT_UNAVAILABLE: &str = "invalid_exact_svm_smart_wallet_alt_resolution_not_available";
const ERR_ALT_FAILED: &str = "invalid_exact_svm_smart_wallet_alt_resolution_failed";
const ERR_SIMULATION_FAILED: &str = "invalid_exact_svm_smart_wallet_simulation_failed";
const ERR_PROGRAM_NOT_ALLOWED: &str = "invalid_exact_svm_smart_wallet_program_not_allowed";
const ERR_VERIFICATION_UNAVAILABLE: &str =
    "invalid_exact_svm_smart_wallet_verification_not_available";
const IX_SET_COMPUTE_UNIT_LIMIT: u8 = 2;
const IX_SET_COMPUTE_UNIT_PRICE: u8 = 3;

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
        VerificationError::SimulationFailed(msg) => is_recoverable_layout_message(msg),
        VerificationError::Wire(reason) => is_recoverable_layout_message(reason.as_str()),
        _ => false,
    }
}

fn is_recoverable_layout_message(msg: &str) -> bool {
    // Official LAYOUT_RECOVERABLE_REASONS (scheme.ts): instruction count,
    // missing positional transfer, unknown extra ix, malformed compute budget.
    // Not: blocked program, CreateATA, fee-payer isolation, NoAccountAtIndex.
    const NEEDLES: &[&str] = &[
        "Too few instructions",
        "Additional instructions not allowed",
        "Instruction count exceeds maximum",
        "Program not in allowed list",
        "Invalid compute limit instruction",
        "Invalid compute price instruction",
        "Invalid token instruction",
        "Empty instruction",
        "Instruction at index",
        "invalid_exact_svm_payload_transaction_instructions_length",
        "invalid_exact_svm_payload_no_transfer_instruction",
        "invalid_exact_svm_payload_unknown_fourth_instruction",
        "invalid_exact_svm_payload_unknown_fifth_instruction",
        "invalid_exact_svm_payload_unknown_sixth_instruction",
        "invalid_exact_svm_payload_unknown_optional_instruction",
        "invalid_exact_svm_payload_transaction_instructions_compute_limit_instruction",
        "invalid_exact_svm_payload_transaction_instructions_compute_price_instruction",
    ];
    NEEDLES.iter().any(|n| msg.contains(n))
}

fn wire(code: &str) -> VerificationError {
    VerificationError::from_wire(code)
}

fn wire_detail(code: &str, detail: impl core::fmt::Display) -> VerificationError {
    VerificationError::from_wire(&format!("{code}: {detail}"))
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
    extract_top_level_transfers_with_keys(tx, tx.message.static_account_keys())
}

/// Top-level transfers using a resolved account-key list (static + ALTs).
#[must_use]
pub(super) fn extract_top_level_transfers_with_keys(
    tx: &VersionedTransaction,
    keys: &[Pubkey],
) -> Vec<ObservedTransfer> {
    let mut out = Vec::new();
    for ix in tx.message.instructions() {
        let Some(program_id) = keys.get(usize::from(ix.program_id_index)).copied() else {
            continue;
        };
        if !is_token_program(&program_id) {
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
        return Err(wire(ERR_CANNOT_DERIVE_ATA));
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
        [] if transfers.is_empty() => Err(wire(ERR_NO_TRANSFER)),
        [] => Err(wire(ERR_TRANSFER_MISMATCH)),
        [one] => Ok(*one),
        _ => Err(wire(ERR_MULTIPLE)),
    }
}

fn is_token_program(program_id: &Pubkey) -> bool {
    *program_id == spl_token::ID || *program_id == spl_token_2022_interface::ID
}

/// Official Path 2 constructor/runtime cap check.
pub(super) fn assert_path2_provider<P: SolanaChainProviderLike>(
    provider: &P,
    config: &SolanaExactFacilitatorConfig,
) -> Result<(), VerificationError> {
    if !provider.supports_smart_wallet() {
        return Err(wire(ERR_VERIFICATION_UNAVAILABLE));
    }
    if let Err(msg) = config.assert_smart_wallet_limits() {
        return Err(VerificationError::InvalidFormat(msg));
    }
    Ok(())
}

/// Top-level programs except `ComputeBudget` and Memo must be in the Path 2 allowlist.
pub(super) fn assert_path2_program_allowlist(
    tx: &VersionedTransaction,
    config: &SolanaExactFacilitatorConfig,
) -> Result<(), VerificationError> {
    let allowed = config.path2_allowed_programs();
    let keys = tx.message.static_account_keys();
    let compute = solana_compute_budget_interface::ID;
    for ix in tx.message.instructions() {
        let Some(program_id) = keys.get(usize::from(ix.program_id_index)).copied() else {
            continue;
        };
        if program_id == compute || program_id == crate::exact::SPL_MEMO_PROGRAM {
            continue;
        }
        if !allowed.contains(&program_id) {
            return Err(wire_detail(ERR_PROGRAM_NOT_ALLOWED, program_id));
        }
    }
    Ok(())
}

/// Resolve static + ALT keys. Empty lookups skip RPC.
pub(super) async fn resolve_loaded_account_keys<P: SolanaChainProviderLike>(
    provider: &P,
    tx: &VersionedTransaction,
) -> Result<Vec<Pubkey>, VerificationError> {
    let mut keys = tx.message.static_account_keys().to_vec();
    let Some(lookups) = tx.message.address_table_lookups() else {
        return Ok(keys);
    };
    if lookups.is_empty() {
        return Ok(keys);
    }
    if !provider.supports_smart_wallet() {
        return Err(wire(ERR_ALT_UNAVAILABLE));
    }
    let table_addrs: Vec<Pubkey> = lookups.iter().map(|lookup| lookup.account_key).collect();
    let tables = provider
        .fetch_address_lookup_tables(&table_addrs)
        .await
        .map_err(|e| wire_detail(ERR_ALT_FAILED, e))?;
    let mut writable = Vec::new();
    let mut readonly = Vec::new();
    for lookup in lookups {
        let table = tables
            .get(&lookup.account_key)
            .ok_or_else(|| wire(ERR_ALT_FAILED))?;
        for idx in &lookup.writable_indexes {
            let addr = table
                .get(usize::from(*idx))
                .copied()
                .ok_or_else(|| wire(ERR_ALT_FAILED))?;
            writable.push(addr);
        }
        for idx in &lookup.readonly_indexes {
            let addr = table
                .get(usize::from(*idx))
                .copied()
                .ok_or_else(|| wire(ERR_ALT_FAILED))?;
            readonly.push(addr);
        }
    }
    keys.extend(writable);
    keys.extend(readonly);
    Ok(keys)
}

/// Spec §2.1: fee payer is neither a program id nor an instruction account.
pub(super) fn assert_fee_payer_isolated_from_keys(
    tx: &VersionedTransaction,
    keys: &[Pubkey],
    fee_payer: Pubkey,
) -> Result<(), VerificationError> {
    for ix in tx.message.instructions() {
        let program_id = keys
            .get(usize::from(ix.program_id_index))
            .copied()
            .ok_or_else(|| wire(ERR_FEE_PAYER_NOT_ISOLATED))?;
        if program_id == fee_payer {
            return Err(wire_detail(
                ERR_FEE_PAYER_NOT_ISOLATED,
                format!("fee payer {fee_payer} invoked as program"),
            ));
        }
        for idx in &ix.accounts {
            let account = keys
                .get(usize::from(*idx))
                .copied()
                .ok_or_else(|| wire(ERR_FEE_PAYER_NOT_ISOLATED))?;
            if account == fee_payer {
                return Err(wire_detail(
                    ERR_FEE_PAYER_NOT_ISOLATED,
                    format!(
                        "fee payer {fee_payer} appears in instruction accounts (program: {program_id})"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Official Path 2 compute-budget caps (only types 2 and 3).
pub(super) fn validate_path2_compute_budget(
    tx: &VersionedTransaction,
    keys: &[Pubkey],
    config: &SolanaExactFacilitatorConfig,
) -> Result<(), VerificationError> {
    let max_cu = config.path2_max_compute_units();
    let max_fee = config.path2_max_priority_fee_micro_lamports();
    let compute = solana_compute_budget_interface::ID;
    for ix in tx.message.instructions() {
        let Some(program_id) = keys.get(usize::from(ix.program_id_index)).copied() else {
            continue;
        };
        if program_id != compute {
            continue;
        }
        let data = ix.data.as_slice();
        if data.is_empty() {
            return Err(wire(ERR_MALFORMED_COMPUTE_BUDGET));
        }
        match data.first().copied() {
            Some(IX_SET_COMPUTE_UNIT_LIMIT) => {
                if data.len() < 5 {
                    return Err(wire(ERR_MALFORMED_COMPUTE_LIMIT));
                }
                let units = u32::from_le_bytes(
                    data.get(1..5)
                        .and_then(|b| b.try_into().ok())
                        .ok_or_else(|| wire(ERR_MALFORMED_COMPUTE_LIMIT))?,
                );
                if units > max_cu {
                    return Err(wire_detail(
                        ERR_COMPUTE_UNITS_TOO_HIGH,
                        format!("{units} exceeds max {max_cu}"),
                    ));
                }
            }
            Some(IX_SET_COMPUTE_UNIT_PRICE) => {
                if data.len() < 9 {
                    return Err(wire(ERR_MALFORMED_COMPUTE_PRICE));
                }
                let micro = u64::from_le_bytes(
                    data.get(1..9)
                        .and_then(|b| b.try_into().ok())
                        .ok_or_else(|| wire(ERR_MALFORMED_COMPUTE_PRICE))?,
                );
                if micro > max_fee {
                    return Err(wire_detail(
                        ERR_PRIORITY_FEE_TOO_HIGH,
                        format!("{micro} exceeds max {max_fee}"),
                    ));
                }
            }
            Some(other) => {
                return Err(wire_detail(
                    ERR_UNSUPPORTED_COMPUTE_BUDGET,
                    format!("type {other}"),
                ));
            }
            None => return Err(wire(ERR_MALFORMED_COMPUTE_BUDGET)),
        }
    }
    Ok(())
}

pub(super) const fn simulation_failed_code() -> &'static str {
    ERR_SIMULATION_FAILED
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
        assert_eq!(
            r402_protocol::AsPaymentProblem::as_payment_problem(&err)
                .reason()
                .as_str(),
            ERR_TRANSFER_MISMATCH
        );
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
        assert!(!is_path1_layout_recoverable(
            &VerificationError::InvalidFormat("Too few instructions".into())
        ));
        assert!(!is_path1_layout_recoverable(
            &VerificationError::SimulationFailed("Blocked program in transaction: Abc".into())
        ));
        assert!(is_path1_layout_recoverable(
            &VerificationError::SimulationFailed("Too few instructions in transaction".into())
        ));
        assert!(!is_path1_layout_recoverable(
            &VerificationError::SimulationFailed(
                "Fee payer included in instruction accounts".into()
            )
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

    fn tx_with_ixs(ixs: &[solana_transaction::Instruction]) -> VersionedTransaction {
        use solana_message::{Message, VersionedMessage};
        use solana_signature::Signature as Sig;
        let payer = Pubkey::new_from_array([1u8; 32]);
        VersionedTransaction {
            signatures: vec![Sig::default()],
            message: VersionedMessage::Legacy(Message::new(ixs, Some(&payer))),
        }
    }

    #[test]
    fn path2_rejects_unsupported_compute_budget_type() {
        let mut data = vec![1u8]; // RequestHeapFrame
        data.extend_from_slice(&32u32.to_le_bytes());
        let tx = tx_with_ixs(&[solana_transaction::Instruction {
            program_id: solana_compute_budget_interface::ID,
            accounts: Vec::new(),
            data,
        }]);
        let keys = tx.message.static_account_keys().to_vec();
        let err =
            validate_path2_compute_budget(&tx, &keys, &SolanaExactFacilitatorConfig::default())
                .unwrap_err();
        assert!(
            r402_protocol::AsPaymentProblem::as_payment_problem(&err)
                .reason()
                .as_str()
                .contains(ERR_UNSUPPORTED_COMPUTE_BUDGET)
        );
    }

    #[test]
    fn path2_rejects_compute_units_above_cap() {
        let mut data = vec![IX_SET_COMPUTE_UNIT_LIMIT];
        data.extend_from_slice(&500_000u32.to_le_bytes());
        let tx = tx_with_ixs(&[solana_transaction::Instruction {
            program_id: solana_compute_budget_interface::ID,
            accounts: Vec::new(),
            data,
        }]);
        let keys = tx.message.static_account_keys().to_vec();
        let err =
            validate_path2_compute_budget(&tx, &keys, &SolanaExactFacilitatorConfig::default())
                .unwrap_err();
        assert!(
            r402_protocol::AsPaymentProblem::as_payment_problem(&err)
                .reason()
                .as_str()
                .contains(ERR_COMPUTE_UNITS_TOO_HIGH)
        );
    }

    #[test]
    fn path2_allowlist_rejects_unknown_wallet_program() {
        let wallet = Pubkey::new_from_array([8u8; 32]);
        let tx = tx_with_ixs(&[solana_transaction::Instruction {
            program_id: wallet,
            accounts: Vec::new(),
            data: vec![1],
        }]);
        let err = assert_path2_program_allowlist(&tx, &SolanaExactFacilitatorConfig::default())
            .unwrap_err();
        let reason = r402_protocol::AsPaymentProblem::as_payment_problem(&err)
            .reason()
            .as_str()
            .to_owned();
        assert!(reason.contains(ERR_PROGRAM_NOT_ALLOWED));
        assert!(reason.contains(&wallet.to_string()));
    }

    #[test]
    fn match_required_transfer_wire_reason_is_not_invalid_payload() {
        let (_pay_to, asset, requirement) = req(100);
        let dest = expected_destination_atas(requirement.pay_to, requirement.asset)[0];
        let t = ObservedTransfer {
            program_id: spl_token::ID,
            amount: 50,
            mint: *asset.pubkey(),
            destination: dest,
            authority: Pubkey::new_from_array([9; 32]),
        };
        let err = match_required_transfer(&[t], &requirement, &[]).unwrap_err();
        assert_eq!(
            r402_protocol::AsPaymentProblem::as_payment_problem(&err)
                .reason()
                .as_str(),
            ERR_TRANSFER_MISMATCH
        );
        assert_ne!(
            r402_protocol::AsPaymentProblem::as_payment_problem(&err)
                .reason()
                .as_str(),
            "invalid_payload"
        );
    }
}
