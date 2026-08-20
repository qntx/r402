//! Hiero transaction inspection and client-side transfer construction.

use std::collections::BTreeMap;
use std::str::FromStr;

use base64::Engine;
use hedera::{AccountId, AnyTransaction, Hbar, TokenId, TransferTransaction};

use super::types::{hedera_account_ids_equal, is_hbar_asset};

/// A single signed transfer entry. Positive values credit, negative debit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HederaTransferEntry {
    /// Account that is credited or debited.
    pub account_id: String,
    /// Amount in tinybars or token atomic units (signed decimal string).
    pub amount: String,
}

/// Parsed transaction details used by facilitator verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedHederaTransaction {
    /// Hiero transaction type name (`TransferTransaction` or other).
    pub transaction_type: String,
    /// Display form of the transaction id (`0.0.n@seconds.nanos`).
    pub transaction_id: String,
    /// `transactionId.accountId` — the fee payer at the network level.
    pub transaction_id_account_id: String,
    /// `true` when the body is not a `TransferTransaction`.
    pub has_non_transfer_operations: bool,
    /// Native HBAR transfers.
    pub hbar_transfers: Vec<HederaTransferEntry>,
    /// HTS transfers grouped by token id.
    pub token_transfers: BTreeMap<String, Vec<HederaTransferEntry>>,
}

/// Decode and inspect a base64 Hiero transaction.
///
/// # Errors
///
/// Returns an error when the bytes are not a Hiero transaction or the
/// transaction id is missing.
pub fn inspect_hedera_transaction(
    transaction_base64: &str,
) -> Result<InspectedHederaTransaction, String> {
    let bytes = Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        transaction_base64,
    )
    .map_err(|e| e.to_string())?;
    inspect_hedera_transaction_bytes(&bytes)
}

/// Inspect already-decoded Hiero transaction bytes.
///
/// # Errors
///
/// Returns an error when the bytes are not a Hiero transaction or the
/// transaction id is missing.
pub fn inspect_hedera_transaction_bytes(
    bytes: &[u8],
) -> Result<InspectedHederaTransaction, String> {
    let any = AnyTransaction::from_bytes(bytes).map_err(|e| e.to_string())?;
    let Some(transaction_id) = any.get_transaction_id() else {
        return Err("invalid_hedera_transaction_metadata".to_owned());
    };
    let transaction_id_account_id = transaction_id.account_id.to_string();
    if transaction_id_account_id.is_empty() {
        return Err("invalid_hedera_transaction_metadata".to_owned());
    }
    let transaction_id_str = transaction_id.to_string();

    match any.downcast::<TransferTransaction>() {
        Ok(tx) => {
            let hbar_transfers = normalize_hbar_transfers(&tx);
            let mut token_transfers = normalize_token_transfers(&tx);
            for (token_id, nfts) in tx.get_nft_transfers() {
                if nfts.is_empty() {
                    continue;
                }
                let _ = token_transfers.entry(token_id.to_string()).or_default();
            }
            Ok(InspectedHederaTransaction {
                transaction_type: "TransferTransaction".to_owned(),
                transaction_id: transaction_id_str,
                transaction_id_account_id,
                has_non_transfer_operations: false,
                hbar_transfers,
                token_transfers,
            })
        }
        Err(_) => Ok(InspectedHederaTransaction {
            transaction_type: "Other".to_owned(),
            transaction_id: transaction_id_str,
            transaction_id_account_id,
            has_non_transfer_operations: true,
            hbar_transfers: Vec::new(),
            token_transfers: BTreeMap::new(),
        }),
    }
}

fn normalize_hbar_transfers(tx: &TransferTransaction) -> Vec<HederaTransferEntry> {
    tx.get_hbar_transfers()
        .into_iter()
        .map(|(account_id, amount)| HederaTransferEntry {
            account_id: account_id.to_string(),
            amount: amount.to_tinybars().to_string(),
        })
        .collect()
}

fn normalize_token_transfers(
    tx: &TransferTransaction,
) -> BTreeMap<String, Vec<HederaTransferEntry>> {
    tx.get_token_transfers()
        .into_iter()
        .map(|(token_id, accounts)| {
            let entries = accounts
                .into_iter()
                .map(|(account_id, amount)| HederaTransferEntry {
                    account_id: account_id.to_string(),
                    amount: amount.to_string(),
                })
                .collect();
            (token_id.to_string(), entries)
        })
        .collect()
}

/// Sums a transfer list and returns the net amount.
#[must_use]
pub fn sum_transfers(transfers: &[HederaTransferEntry]) -> i128 {
    transfers.iter().map(|e| parse_i128(&e.amount)).sum()
}

/// Returns account ids with a positive net receipt.
#[must_use]
pub fn get_positive_receivers(transfers: &[HederaTransferEntry]) -> Vec<String> {
    let mut net: BTreeMap<String, i128> = BTreeMap::new();
    for entry in transfers {
        *net.entry(entry.account_id.clone()).or_insert(0) += parse_i128(&entry.amount);
    }
    net.into_iter()
        .filter(|(_, value)| *value > 0)
        .map(|(account_id, _)| account_id)
        .collect()
}

fn parse_i128(s: &str) -> i128 {
    s.parse().unwrap_or(0)
}

/// Transfer list for the requested asset, or `None` when the payload
/// transfers a different set of assets.
#[must_use]
pub fn asset_transfers<'a>(
    inspected: &'a InspectedHederaTransaction,
    asset: &str,
) -> Option<&'a [HederaTransferEntry]> {
    if is_hbar_asset(asset) {
        if !inspected.token_transfers.is_empty() {
            return None;
        }
        return Some(inspected.hbar_transfers.as_slice());
    }
    if inspected.token_transfers.len() != 1 {
        return None;
    }
    inspected.token_transfers.get(asset).map(Vec::as_slice)
}

/// Debited senders and the absolute amount each is spending.
#[must_use]
pub fn infer_payers(transfers: &[HederaTransferEntry]) -> Vec<(String, String)> {
    let mut debited: BTreeMap<String, i128> = BTreeMap::new();
    for entry in transfers {
        let amount = parse_i128(&entry.amount);
        if amount < 0 {
            *debited.entry(entry.account_id.clone()).or_insert(0) += amount;
        }
    }
    debited
        .into_iter()
        .map(|(account_id, amount)| (account_id, (-amount).to_string()))
        .collect()
}

/// Returns `true` when `account_id` has a negative entry in `transfers`.
#[must_use]
pub fn has_negative_transfer(transfers: &[HederaTransferEntry], account_id: &str) -> bool {
    transfers.iter().any(|entry| {
        hedera_account_ids_equal(&entry.account_id, account_id) && parse_i128(&entry.amount) < 0
    })
}

/// Builds a Hiero `Client` for a CAIP-2 network.
///
/// # Errors
///
/// Returns an error when `network` is not a supported Hedera CAIP-2 id, or
/// when `node_url` is not a valid custom network map.
pub fn create_hedera_client(
    network: &str,
    node_url: Option<&str>,
) -> Result<hedera::Client, String> {
    if let Some(node_url) = node_url {
        let mut map = std::collections::HashMap::new();
        let _ = map.insert(
            node_url.to_owned(),
            AccountId::from_str("0.0.3").map_err(|e| e.to_string())?,
        );
        return hedera::Client::for_network(map).map_err(|e| e.to_string());
    }
    match network {
        "hedera:mainnet" => Ok(hedera::Client::for_mainnet()),
        "hedera:testnet" => Ok(hedera::Client::for_testnet()),
        other => Err(format!("Unsupported Hedera network: {other}")),
    }
}

/// Builds a partially signed HBAR or HTS `TransferTransaction`.
///
/// # Errors
///
/// Returns an error when identifiers or amounts are invalid, or when the
/// transaction cannot be frozen or signed.
#[allow(
    clippy::too_many_arguments,
    clippy::large_types_passed_by_value,
    reason = "Hiero AccountId is Copy but includes optional key material"
)]
pub fn create_partially_signed_transfer(
    payer: AccountId,
    private_key: &hedera::PrivateKey,
    fee_payer: AccountId,
    pay_to: AccountId,
    asset: &str,
    amount: i64,
    network: &str,
    node_url: Option<&str>,
) -> Result<String, String> {
    if amount <= 0 {
        return Err("amount must be greater than zero".to_owned());
    }
    let mut tx = TransferTransaction::new();
    if is_hbar_asset(asset) {
        let _ = tx
            .hbar_transfer(payer, Hbar::from_tinybars(-amount))
            .hbar_transfer(pay_to, Hbar::from_tinybars(amount));
    } else {
        let token_id = TokenId::from_str(asset).map_err(|e| e.to_string())?;
        let _ = tx
            .token_transfer(token_id, payer, -amount)
            .token_transfer(token_id, pay_to, amount);
    }
    let _ = tx.transaction_id(hedera::TransactionId::generate(fee_payer));
    // Avoid Client::for_* here: constructing a managed network requires a
    // Tokio runtime even when only freezing bytes.
    if let Some(node_url) = node_url {
        let client = create_hedera_client(network, Some(node_url))?;
        tx.freeze_with(&client).map_err(|e| e.to_string())?;
    } else {
        let _ = tx.node_account_ids([AccountId::new(0, 0, 3)]);
        tx.freeze().map_err(|e| e.to_string())?;
    }
    let _ = tx.sign(private_key.clone());
    let bytes = tx.to_bytes().map_err(|e| e.to_string())?;
    Ok(Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        bytes,
    ))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test assertions"
)]
mod tests {
    use super::*;
    use hedera::{AccountId, Hbar, PrivateKey, TokenId, TopicCreateTransaction, TransactionId};

    fn transfer_b64(args: (&str, &str, &str, &str, &str)) -> String {
        let (fee_payer, payer, pay_to, asset, amount) = args;
        let amount: i64 = amount.parse().unwrap();
        let mut tx = TransferTransaction::new();
        if asset == "0.0.0" {
            let _ = tx
                .hbar_transfer(
                    AccountId::from_str(payer).unwrap(),
                    Hbar::from_tinybars(-amount),
                )
                .hbar_transfer(
                    AccountId::from_str(pay_to).unwrap(),
                    Hbar::from_tinybars(amount),
                );
        } else {
            let token = TokenId::from_str(asset).unwrap();
            let _ = tx
                .token_transfer(token, AccountId::from_str(payer).unwrap(), -amount)
                .token_transfer(token, AccountId::from_str(pay_to).unwrap(), amount);
        }
        let _ = tx
            .transaction_id(TransactionId::generate(
                AccountId::from_str(fee_payer).unwrap(),
            ))
            .node_account_ids([AccountId::new(0, 0, 3)]);
        tx.freeze().unwrap();
        Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            tx.to_bytes().unwrap(),
        )
    }

    #[test]
    fn inspects_token_transfer() {
        let b64 = transfer_b64(("0.0.5001", "0.0.9001", "0.0.7001", "0.0.6001", "1000"));
        let inspected = inspect_hedera_transaction(&b64).unwrap();
        assert_eq!(inspected.transaction_type, "TransferTransaction");
        assert_eq!(inspected.transaction_id_account_id, "0.0.5001");
        assert!(!inspected.has_non_transfer_operations);
        let transfers = asset_transfers(&inspected, "0.0.6001").unwrap();
        assert_eq!(sum_transfers(transfers), 0);
        let payers = infer_payers(transfers);
        assert_eq!(payers.len(), 1);
        assert_eq!(payers[0].0, "0.0.9001");
        assert_eq!(payers[0].1, "1000");
    }

    #[test]
    fn inspects_non_transfer() {
        let mut tx = TopicCreateTransaction::new();
        let _ = tx
            .transaction_id(TransactionId::generate(
                AccountId::from_str("0.0.5001").unwrap(),
            ))
            .node_account_ids([AccountId::new(0, 0, 3)]);
        let key = PrivateKey::generate_ed25519();
        let _ = tx.submit_key(key.public_key());
        tx.freeze().unwrap();
        let b64 = Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            tx.to_bytes().unwrap(),
        );
        let inspected = inspect_hedera_transaction(&b64).unwrap();
        assert!(inspected.has_non_transfer_operations);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(inspect_hedera_transaction("not-a-valid-hedera-transaction").is_err());
    }
}
