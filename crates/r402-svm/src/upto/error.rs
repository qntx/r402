//! Error types for the Solana `upto` payment-channel scheme.

use r402_protocol::{ErrorReason, FacilitatorError, VerificationError};

use super::channel::ChannelConfigError;

/// Facilitator / verify error codes for SVM `upto`.
pub mod codes {
    /// Client supplied the server-owned `voucherSignature` at verify / deposit.
    pub const UNEXPECTED_VOUCHER: &str = "invalid_upto_svm_payload_unexpected_voucher";
    /// Charge exceeds the signed ceiling.
    pub const SETTLEMENT_EXCEEDS_AMOUNT: &str =
        "invalid_upto_svm_payload_settlement_exceeds_amount";
    /// Partial charge with no voucher.
    pub const MISSING_VOUCHER: &str = "invalid_upto_svm_payload_missing_voucher";
    /// Amount field is not an unsigned integer.
    pub const PAYLOAD_AMOUNT: &str = "invalid_upto_svm_payload_amount";
    /// `payload.maxAmount != requirements.amount` at verify / deposit.
    pub const AMOUNT_MISMATCH: &str = "invalid_upto_svm_payload_amount_mismatch";
    /// `payload.deposit != payload.maxAmount`.
    pub const DEPOSIT_NOT_CEILING: &str = "invalid_upto_svm_payload_deposit_not_ceiling";
    /// Channel PDA seed (`openSlot` / nonce) is malformed.
    pub const CHANNEL_SEED: &str = "invalid_upto_svm_payload_channel_seed";
    /// Before `payload.validAfter`.
    pub const NOT_YET_ACTIVE: &str = "invalid_upto_svm_payload_not_yet_active";
    /// At or after `payload.expiresAt`.
    pub const EXPIRED: &str = "invalid_upto_svm_payload_expired";
    /// Requested channel lifetime exceeds the facilitator cap.
    pub const CHANNEL_LIFETIME_EXCEEDED: &str =
        "invalid_upto_svm_payload_channel_lifetime_exceeded";
    /// `expiresAt` exceeds `now + maxTimeoutSeconds` (+ skew).
    pub const EXPIRES_AT_MISMATCH: &str = "invalid_upto_svm_payload_expires_at_mismatch";
    /// Open transaction failed the acceptance policy.
    pub const OPEN_TRANSACTION: &str = "invalid_upto_svm_payload_open_transaction";
    /// Open channel PDA != `payload.channelId`.
    pub const CHANNEL_ID: &str = "invalid_upto_svm_payload_channel_id";
    /// Open salt != `payload.nonce`.
    pub const NONCE: &str = "invalid_upto_svm_payload_nonce";
    /// Open payer != `payload.from`.
    pub const PAYER_MISMATCH: &str = "invalid_upto_svm_payload_payer_mismatch";
    /// `payload.authorizedSigner` is not `extra.receiverAuthorizer`.
    pub const RECEIVER_AUTHORIZER: &str = "invalid_upto_svm_payload_receiver_authorizer";
    /// Voucher is not signed by the authorizer.
    pub const VOUCHER_SIGNATURE: &str = "invalid_upto_svm_payload_voucher_signature";
    /// Deposit settle targets an existing PDA.
    pub const CHANNEL_ALREADY_OPEN: &str = "invalid_upto_svm_channel_already_open";
    /// Onchain channel does not match the challenge.
    pub const CHANNEL_STATE: &str = "invalid_upto_svm_channel_state";
    /// Open broadcast (or pre-broadcast index) failed; safe to retry.
    pub const CHANNEL_BROADCAST: &str = "invalid_upto_svm_channel_broadcast";
    /// Pre-broadcast settlement simulation failed.
    pub const SETTLEMENT_SIMULATION: &str = "invalid_upto_svm_settlement_simulation";
    /// Requirements extra is unusable.
    pub const PAYMENT_REQUIREMENTS: &str = "invalid_upto_svm_payment_requirements";
}

/// [`ErrorReason`] for an SVM `upto` wire code.
#[must_use]
pub fn upto_reason(code: &str) -> ErrorReason {
    ErrorReason::from_wire(code)
}

impl From<ChannelConfigError> for VerificationError {
    fn from(err: ChannelConfigError) -> Self {
        Self::InvalidFormat(err.to_string())
    }
}

impl From<ChannelConfigError> for FacilitatorError {
    fn from(err: ChannelConfigError) -> Self {
        Self::Verification(VerificationError::from(err))
    }
}
