//! Canonical 50-byte payment-channel voucher payload.
#![allow(clippy::indexing_slicing, reason = "fixed 50-byte voucher layout")]

use solana_pubkey::Pubkey;

/// Magic prefix of the signed voucher payload (`V\x01`).
pub const VOUCHER_MAGIC: [u8; 2] = [0x56, 0x01];

/// Fixed length of the signed voucher payload.
pub const VOUCHER_MESSAGE_SIZE: usize = 50;

/// Encode `magic(2) || channel_id(32) || u64le(amount) || i64le(expires_at)`.
#[must_use]
pub fn encode_voucher_message(
    channel_id: &Pubkey,
    cumulative_amount: u64,
    expires_at: i64,
) -> [u8; VOUCHER_MESSAGE_SIZE] {
    let mut out = [0u8; VOUCHER_MESSAGE_SIZE];
    out[0] = VOUCHER_MAGIC[0];
    out[1] = VOUCHER_MAGIC[1];
    out[2..34].copy_from_slice(channel_id.as_ref());
    out[34..42].copy_from_slice(&cumulative_amount.to_le_bytes());
    out[42..50].copy_from_slice(&expires_at.to_le_bytes());
    out
}

#[cfg(any(feature = "server", feature = "client", feature = "facilitator"))]
mod signed {
    use solana_pubkey::Pubkey;
    use solana_signature::Signature;
    use solana_signer::{Signer, SignerError};

    use super::{VOUCHER_MESSAGE_SIZE, encode_voucher_message};

    /// Sign the canonical voucher message with `authorizer`.
    ///
    /// # Errors
    ///
    /// Returns [`SignerError`] when the key cannot sign.
    pub fn sign_voucher<S: Signer + ?Sized>(
        authorizer: &S,
        channel_id: &Pubkey,
        cumulative_amount: u64,
        expires_at: i64,
    ) -> Result<Signature, SignerError> {
        let message = encode_voucher_message(channel_id, cumulative_amount, expires_at);
        authorizer.try_sign_message(&message)
    }

    /// Verify a base58 Ed25519 voucher signature.
    #[must_use]
    pub fn verify_voucher_signature(
        signature: &Signature,
        signer: &Pubkey,
        message: &[u8; VOUCHER_MESSAGE_SIZE],
    ) -> bool {
        signature.verify(signer.as_ref(), message)
    }
}

#[cfg(any(feature = "server", feature = "client", feature = "facilitator"))]
pub use signed::{sign_voucher, verify_voucher_signature};

#[cfg(test)]
mod tests {
    use solana_pubkey::Pubkey;

    use super::*;

    #[test]
    fn voucher_message_matches_cross_language_golden() {
        // USDC mainnet mint, amount 1_000_000, expires_at 4_102_444_800.
        let channel: Pubkey = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
            .parse()
            .unwrap();
        let message = encode_voucher_message(&channel, 1_000_000, 4_102_444_800);
        assert_eq!(
            hex::encode(message),
            "5601c6fa7af3bedbad3a3d65f36aabc97431b1bbe4c2d2f6e0e47ca60203452f5d6140420f0000000000005786f400000000"
        );
    }

    #[test]
    fn voucher_layout_fields() {
        let channel = Pubkey::from([0xAB; 32]);
        let message = encode_voucher_message(&channel, 1858, 1_893_456_000);
        assert_eq!(&message[0..2], &VOUCHER_MAGIC);
        assert_eq!(&message[2..34], channel.as_ref());
        assert_eq!(&message[34..42], &1858u64.to_le_bytes());
        assert_eq!(&message[42..50], &1_893_456_000i64.to_le_bytes());
    }
}
