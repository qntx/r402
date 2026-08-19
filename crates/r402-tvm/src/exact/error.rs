//! Error types and wire `invalidReason` codes for the TON `"exact"` scheme.

use compact_str::CompactString;
use r402_core::error::ErrorReason;
use r402_core::wire::VerifyResponse;

/// Wire `invalidReason` for an unsupported x402 version.
pub const ERR_EXACT_TVM_UNSUPPORTED_VERSION: &str = "unsupported_version";
/// Wire `invalidReason` for a scheme other than `"exact"`.
pub const ERR_EXACT_TVM_UNSUPPORTED_SCHEME: &str = "unsupported_scheme";
/// Wire `invalidReason` for an unknown TVM network.
pub const ERR_EXACT_TVM_UNSUPPORTED_NETWORK: &str = "unsupported_network";
/// Wire `invalidReason` when payload and requirements networks differ.
pub const ERR_EXACT_TVM_NETWORK_MISMATCH: &str = "network_mismatch";
/// Wire `invalidReason` when `settlementBoc` / `asset` are missing.
pub const ERR_EXACT_TVM_INVALID_PAYLOAD: &str = "invalid_exact_tvm_payload";
/// Wire `invalidReason` when the settlement BoC is not an internal message.
pub const ERR_EXACT_TVM_INVALID_SETTLEMENT_BOC: &str = "invalid_exact_tvm_payload_settlement_boc";
/// Wire `invalidReason` when the body is not W5 `internal_signed`.
pub const ERR_EXACT_TVM_INVALID_W5_MESSAGE: &str =
    "invalid_exact_tvm_payload_w5_internal_signed_request";
/// Wire `invalidReason` when W5 actions are not one bounceable jetton send.
pub const ERR_EXACT_TVM_INVALID_W5_ACTIONS: &str = "invalid_exact_tvm_payload_w5_actions";
/// Wire `invalidReason` when the jetton transfer body is malformed.
pub const ERR_EXACT_TVM_INVALID_JETTON_TRANSFER: &str = "invalid_exact_tvm_payload_jetton_transfer";
/// Wire `invalidReason` when the Ed25519 signature does not verify.
pub const ERR_EXACT_TVM_INVALID_SIGNATURE: &str = "invalid_exact_tvm_payload_invalid_signature";
/// Wire `invalidReason` when the wallet code hash is not W5R1.
pub const ERR_EXACT_TVM_INVALID_CODE_HASH: &str = "invalid_exact_tvm_payload_invalid_code_hash";
/// Wire `invalidReason` when the jetton minter does not match requirements.
pub const ERR_EXACT_TVM_INVALID_ASSET: &str = "invalid_exact_tvm_payload_asset_mismatch";
/// Wire `invalidReason` when `payTo` / owner does not match.
pub const ERR_EXACT_TVM_INVALID_RECIPIENT: &str = "invalid_exact_tvm_payload_recipient_mismatch";
/// Wire `invalidReason` when the jetton amount does not match.
pub const ERR_EXACT_TVM_INVALID_AMOUNT: &str = "invalid_exact_tvm_payload_amount_mismatch";
/// Wire `invalidReason` when W5 `signatureAllowed` is false.
pub const ERR_EXACT_TVM_INVALID_SIGNATURE_MODE: &str =
    "invalid_exact_tvm_payload_signature_mode_mismatch";
/// Wire `invalidReason` when the signed seqno is not the on-chain seqno.
pub const ERR_EXACT_TVM_INVALID_SEQNO: &str = "invalid_exact_tvm_payload_seqno_mismatch";
/// Wire `invalidReason` when the signed `walletId` does not match state.
pub const ERR_EXACT_TVM_INVALID_WALLET_ID: &str = "invalid_exact_tvm_payload_wallet_id_mismatch";
/// Wire `invalidReason` when W5 extensions are present on an undeployed wallet.
pub const ERR_EXACT_TVM_INVALID_EXTENSIONS_DICT: &str =
    "invalid_exact_tvm_payload_extensions_dict_mismatch";
/// Wire `invalidReason` when attached TON exceeds the sponsored cap.
pub const ERR_EXACT_TVM_TON_AMOUNT_TOO_HIGH: &str =
    "invalid_exact_tvm_payload_attached_ton_amount_too_high";
/// Wire `invalidReason` when the payer account is frozen.
pub const ERR_EXACT_TVM_ACCOUNT_FROZEN: &str = "account_frozen";
/// Wire `invalidReason` when `validUntil` is in the past.
pub const ERR_EXACT_TVM_INVALID_UNTIL_EXPIRED: &str =
    "invalid_exact_tvm_payload_valid_until_expired";
/// Wire `invalidReason` when `validUntil` exceeds `maxTimeoutSeconds`.
pub const ERR_EXACT_TVM_VALID_UNTIL_TOO_FAR: &str = "invalid_exact_tvm_payload_valid_until_too_far";
/// Wire `invalidReason` when the payer jetton balance is too low.
pub const ERR_EXACT_TVM_INSUFFICIENT_BALANCE: &str = "insufficient_balance";
/// Wire `invalidReason` when the facilitator TON balance is below the floor.
pub const ERR_EXACT_TVM_FACILITATOR_INSUFFICIENT_BALANCE: &str = "facilitator_insufficient_balance";
/// Wire `invalidReason` for a replayed settlement BoC.
pub const ERR_EXACT_TVM_DUPLICATE_SETTLEMENT: &str = "duplicate_settlement";
/// Wire `invalidReason` when emulation or trace checks fail.
pub const ERR_EXACT_TVM_SIMULATION_FAILED: &str = "simulation_failed";
/// Wire `invalidReason` when Highload send or confirmation fails.
pub const ERR_EXACT_TVM_TRANSACTION_FAILED: &str = "transaction_failed";

/// A verify failure carrying a wire `invalidReason` and optional payer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvmInvalid {
    /// Wire `invalidReason`.
    pub reason: ErrorReason,
    /// Optional human-readable message.
    pub message: Option<CompactString>,
    /// Payer address, attached after the settlement BoC is parsed.
    pub payer: Option<CompactString>,
}

impl TvmInvalid {
    /// Constructs an invalid outcome with no payer.
    #[must_use]
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason: ErrorReason::from_wire(reason),
            message: None,
            payer: None,
        }
    }

    /// Constructs an invalid outcome from a possibly non-static reason.
    #[must_use]
    pub fn from_reason(reason: impl AsRef<str>) -> Self {
        Self {
            reason: ErrorReason::from_wire(reason.as_ref()),
            message: None,
            payer: None,
        }
    }

    /// Attaches a cryptographically attributed payer.
    #[must_use]
    pub fn with_payer(mut self, payer: impl Into<CompactString>) -> Self {
        self.payer = Some(payer.into());
        self
    }

    /// Attaches a human-readable message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<CompactString>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Converts to a wire [`VerifyResponse`].
    #[must_use]
    pub fn into_response(self) -> VerifyResponse {
        match self.message {
            Some(message) => VerifyResponse::invalid_with_message(self.payer, self.reason, message),
            None => VerifyResponse::invalid(self.payer, self.reason),
        }
    }
}

/// Convenience: builds [`TvmInvalid`] from a static reason code.
#[must_use]
pub fn invalid(reason: &'static str) -> TvmInvalid {
    TvmInvalid::new(reason)
}
