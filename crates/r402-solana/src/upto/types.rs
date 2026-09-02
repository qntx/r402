//! Wire types for the Solana `upto` payment-channel scheme.

pub use r402_core::scheme::UptoScheme;
use serde::{Deserialize, Serialize};

/// Client authorization for SVM `upto`.
///
/// `voucher_signature` is server-owned and settle-only; verify rejects the
/// field if present on the client payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UptoSvmPayload {
    /// Payer wallet (base58).
    pub from: String,
    /// Signed ceiling (base units). MUST equal verification-phase `amount`.
    pub max_amount: String,
    /// Deadline (Unix seconds); signed into the onchain voucher.
    pub expires_at: i64,
    /// Activation time (Unix seconds).
    pub valid_after: i64,
    /// Unique per-authorization identifier (channel salt, decimal).
    pub nonce: String,
    /// Slot encoded in the open instruction (decimal).
    pub open_slot: String,
    /// Channel PDA (base58).
    pub channel_id: String,
    /// Onchain escrow ceiling (base units); MUST equal `max_amount`.
    pub deposit: String,
    /// Voucher signer; MUST equal `extra.receiverAuthorizer` (base58).
    pub authorized_signer: String,
    /// Base64 client-signed `open` transaction for the fee payer to broadcast.
    pub open_transaction: String,
    /// Base58 Ed25519 voucher signature by `authorized_signer`. Settle-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voucher_signature: Option<String>,
}

/// Extra keys written into `PaymentRequirements.extra` for SVM `upto`.
pub mod extra_keys {
    /// Facilitator fee payer / rent payer / zero-share payee.
    pub const FEE_PAYER: &str = "feePayer";
    /// Server hot key set as channel `authorized_signer`.
    pub const RECEIVER_AUTHORIZER: &str = "receiverAuthorizer";
    /// Channel `grace_period` in seconds.
    pub const WITHDRAW_DELAY: &str = "withdrawDelay";
    /// SPL Token or Token-2022 program id.
    pub const TOKEN_PROGRAM: &str = "tokenProgram";
    /// Seller memo for the open transaction.
    pub const MEMO: &str = "memo";
    /// Optional activation time (unix seconds).
    pub const VALID_AFTER: &str = "validAfter";
    /// Pre-fetched blockhash for the client open.
    pub const RECENT_BLOCKHASH: &str = "recentBlockhash";
    /// Last valid block height for `recentBlockhash`.
    pub const LAST_VALID_BLOCK_HEIGHT: &str = "lastValidBlockHeight";
    /// Recent slot the client MAY use as `openSlot`.
    pub const RECENT_SLOT: &str = "recentSlot";
}

/// Asset-transfer method advertised for SVM `upto`.
pub const ASSET_TRANSFER_METHOD_CHANNEL: &str = "channel";

/// JSON key attached by [`super::server::UptoSvmScheme`] on claim/cancel settle.
pub const VOUCHER_SIGNATURE_FIELD: &str = "voucherSignature";

/// Whether `payload` has the SVM `upto` payment-channel shape.
#[must_use]
pub fn is_upto_svm_payload(payload: &serde_json::Value) -> bool {
    let Some(obj) = payload.as_object() else {
        return false;
    };
    string_field(obj, "from")
        && string_field(obj, "maxAmount")
        && string_field(obj, "deposit")
        && string_field(obj, "channelId")
        && string_field(obj, "authorizedSigner")
        && string_field(obj, "openTransaction")
        && string_field(obj, "openSlot")
        && string_field(obj, "nonce")
        && obj.get("expiresAt").is_some_and(is_json_integer)
        && obj.get("validAfter").is_some_and(is_json_integer)
        && obj
            .get("voucherSignature")
            .is_none_or(serde_json::Value::is_string)
}

fn string_field(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> bool {
    obj.get(key).is_some_and(serde_json::Value::is_string)
}

fn is_json_integer(value: &serde_json::Value) -> bool {
    value.is_i64() || value.is_u64()
}

/// Typed wire aliases for SVM `upto`.
pub mod v2 {
    use r402_core::wire;
    use r402_core::wire::U64String;

    use super::{UptoScheme, UptoSvmPayload};
    use crate::chain::Address;

    /// Verify request using the upto Solana payment scheme.
    pub type VerifyRequest = wire::TypedVerifyRequest<2, PaymentPayload, PaymentRequirements>;
    /// Settle request (same structure as verify).
    pub type SettleRequest = VerifyRequest;
    /// Payment payload with embedded requirements and upto channel data.
    pub type PaymentPayload = wire::PaymentPayload<PaymentRequirements, UptoSvmPayload>;
    /// Payment requirements with Solana address types.
    pub type PaymentRequirements =
        wire::PaymentRequirements<UptoScheme, U64String, Address, serde_json::Value>;
}
