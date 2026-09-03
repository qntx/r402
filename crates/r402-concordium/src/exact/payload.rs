//! Wire payload types for the Concordium `"exact"` payment scheme.

use std::collections::BTreeMap;

pub use r402_protocol::scheme::ExactScheme;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chain::ConcordiumAddress;

/// Extra fields for Concordium payment requirements.
///
/// Official extra is an open `Record<string, unknown>`; unknown keys are
/// ignored so `feePayer` still deserializes next to server-defined fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcordiumExtra {
    /// Facilitator sponsor account that pays transaction fees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_payer: Option<ConcordiumAddress>,
}

/// Credential-indexed signature map: `{ credIndex: { keyIndex: hex } }`.
pub type CredentialSignatureMap = BTreeMap<String, BTreeMap<String, String>>;

/// Sponsor slot in a V1 header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SponsorHeader {
    /// Facilitator account (official TS JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Alias used by some JSON shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Number of sponsor credential signatures.
    #[serde(default)]
    pub num_signatures: u64,
}

impl SponsorHeader {
    /// `address` if set, otherwise `account`.
    #[must_use]
    pub fn resolved_address(&self) -> Option<&str> {
        self.address
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.account.as_deref().filter(|s| !s.is_empty()))
    }
}

/// V1 transaction header as advertised on the x402 wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignableV1TransactionHeader {
    /// Sender account (`Base58Check`).
    #[serde(default)]
    pub sender: String,
    /// Account nonce / sequence number.
    #[serde(default)]
    pub nonce: u64,
    /// Unix expiry seconds. Official JSON uses a number.
    #[serde(default)]
    pub expiry: Value,
    /// Sender signature count.
    #[serde(default)]
    pub num_signatures: u64,
    /// Base energy amount (TS `executionEnergyAmount`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_energy_amount: Option<u64>,
    /// Sponsor identity.
    #[serde(default)]
    pub sponsor: Option<SponsorHeader>,
}

/// Native CCD or PLT payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SignableV1TransactionPayload {
    /// Native CCD simple transfer.
    #[serde(rename = "transfer")]
    Transfer {
        /// Recipient.
        #[serde(default, rename = "toAddress")]
        to_address: Option<String>,
        /// Amount in microCCD (string or number).
        #[serde(default)]
        amount: Value,
    },
    /// Native CCD transfer with memo.
    #[serde(rename = "transferWithMemo")]
    TransferWithMemo {
        /// Recipient.
        #[serde(default, rename = "toAddress")]
        to_address: Option<String>,
        /// Amount in microCCD.
        #[serde(default)]
        amount: Value,
        /// Optional memo hex.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memo: Option<String>,
    },
    /// PLT token update. `operations` is CBOR (hex on the wire we emit).
    #[serde(rename = "tokenUpdate")]
    TokenUpdate {
        /// Registered token symbol.
        #[serde(default, rename = "tokenId")]
        token_id: Option<String>,
        /// CBOR operations (hex string or JSON).
        #[serde(default)]
        operations: Value,
    },
}

impl SignableV1TransactionPayload {
    /// Wire `type` tag.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Transfer { .. } => "transfer",
            Self::TransferWithMemo { .. } => "transferWithMemo",
            Self::TokenUpdate { .. } => "tokenUpdate",
        }
    }
}

/// Sender and sponsor credential signatures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SignableV1Signatures {
    /// Client signatures.
    #[serde(default)]
    pub sender: CredentialSignatureMap,
    /// Empty from the client; facilitator fills on settle.
    #[serde(default)]
    pub sponsor: CredentialSignatureMap,
}

/// Partially-signed Concordium V1 sponsored transaction (x402 wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignableV1Transaction {
    /// Must be `1`.
    pub version: u64,
    /// Transaction header.
    pub header: SignableV1TransactionHeader,
    /// Transfer or token-update payload.
    pub payload: SignableV1TransactionPayload,
    /// Sender + sponsor signature maps.
    #[serde(default)]
    pub signatures: SignableV1Signatures,
}

/// x402 V2 Concordium exact payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactConcordiumPayload {
    /// Partially-signed V1 sponsored transaction.
    pub signed_transaction: SignableV1Transaction,
    /// Optional sender, ignored by facilitators (header.sender wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
}

/// On-chain outcome after finalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionInfo {
    /// Transaction hash (hex).
    pub tx_hash: String,
    /// `pending` / `committed` / `finalized` / `failed`.
    pub status: TransactionStatus,
    /// Sender account.
    pub sender: String,
    /// Transfer recipient when known.
    pub recipient: Option<String>,
    /// Transfer amount in atomic units.
    pub amount: Option<String>,
    /// `"CCD"` or PLT symbol.
    pub asset: Option<String>,
}

/// Finalization status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    /// Not yet committed.
    Pending,
    /// In a block, not finalized.
    Committed,
    /// `ConcordiumBFT` finalized.
    Finalized,
    /// Rejected on-chain.
    Failed,
}

impl TransactionStatus {
    /// Wire string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Committed => "committed",
            Self::Finalized => "finalized",
            Self::Failed => "failed",
        }
    }
}

/// Wire format type aliases.
pub mod v2 {
    use compact_str::CompactString;
    use r402_protocol::payment::{
        PaymentPayload as WirePayload, PaymentRequirements as WireReqs, TypedVerifyRequest,
    };

    use super::{ConcordiumExtra, ExactConcordiumPayload, ExactScheme};

    /// Type alias for verify requests.
    pub type VerifyRequest = TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;

    /// Type alias for settle requests.
    pub type SettleRequest = VerifyRequest;

    /// Type alias for payment payloads.
    pub type PaymentPayload = WirePayload<PaymentRequirements, ExactConcordiumPayload>;

    /// Type alias for payment requirements.
    pub type PaymentRequirements =
        WireReqs<ExactScheme, CompactString, CompactString, ConcordiumExtra>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_keeps_fee_payer_beside_unknown_fields() {
        let json = serde_json::json!({
            "feePayer": "3UrcxPQeYywasrPcYUcqhvFu3SB2vBBDjj7TsaRQ431vGiczYp",
            "customField": "customValue",
            "anotherField": 42
        });
        let extra: ConcordiumExtra = serde_json::from_value(json).expect("open extra");
        assert_eq!(
            extra.fee_payer.map(|a| a.to_string()).as_deref(),
            Some("3UrcxPQeYywasrPcYUcqhvFu3SB2vBBDjj7TsaRQ431vGiczYp")
        );
        let empty = serde_json::json!({});
        assert!(serde_json::from_value::<ConcordiumExtra>(empty).is_ok());
    }

    #[test]
    fn payload_camel_case_signed_transaction() {
        let payload = ExactConcordiumPayload {
            signed_transaction: SignableV1Transaction {
                version: 1,
                header: SignableV1TransactionHeader {
                    sender: "3kBx2h5Y2veb4hZvAE2c1Zr6DYJwWbPr9xQJJBPWyFnXHF9UuN".to_owned(),
                    nonce: 42,
                    expiry: serde_json::json!(1_700_000_300),
                    num_signatures: 1,
                    execution_energy_amount: None,
                    sponsor: Some(SponsorHeader {
                        account: Some(
                            "4FmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW".to_owned(),
                        ),
                        address: None,
                        num_signatures: 1,
                    }),
                },
                payload: SignableV1TransactionPayload::Transfer {
                    to_address: Some(
                        "4FmiTW2L4RvCsSVTjFAavYvrgnPLGNj43eiwPYmbhNqtAcMbWW".to_owned(),
                    ),
                    amount: serde_json::json!("1000000"),
                },
                signatures: SignableV1Signatures {
                    sender: BTreeMap::from([(
                        "0".to_owned(),
                        BTreeMap::from([("0".to_owned(), "a1b2c3".to_owned())]),
                    )]),
                    sponsor: BTreeMap::new(),
                },
            },
            sender: None,
        };
        let json = serde_json::to_value(&payload).expect("json");
        assert!(json.get("signedTransaction").is_some());
        assert!(
            json.get("signedTransaction")
                .and_then(|t| t.get("version"))
                .is_some()
        );
    }
}
