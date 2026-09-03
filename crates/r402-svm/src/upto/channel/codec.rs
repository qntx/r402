//! Borsh-compatible encoding of payment-channel `open` arguments.
#![allow(
    clippy::indexing_slicing,
    reason = "fixed-width borsh layout after length checks"
)]

use solana_pubkey::Pubkey;

use super::program::BASIS_POINTS_DENOMINATOR;

/// Distribution recipient share in basis points (`10_000` = 100%).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSplit {
    /// Recipient address (base58).
    pub recipient: String,
    /// Share in basis points (`10_000` = 100%).
    pub bps: u16,
}

impl ChannelSplit {
    /// Full (100%) share assigned to `recipient`.
    #[must_use]
    pub fn full(recipient: impl Into<String>) -> Self {
        Self {
            recipient: recipient.into(),
            bps: BASIS_POINTS_DENOMINATOR,
        }
    }
}

/// Borsh-encoded arguments of the `open` instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenArgs {
    /// Channel-derivation salt.
    pub salt: u64,
    /// Escrow deposit (authorized ceiling).
    pub deposit: u64,
    /// Forced-close grace period in seconds.
    pub grace_period: u32,
    /// Slot the open is anchored to.
    pub open_slot: u64,
    /// Distribution recipients sealed into the channel.
    pub recipients: Vec<ChannelSplit>,
}

/// Encode `open` args in program (borsh) order.
///
/// # Errors
///
/// Returns an error when a recipient is not a valid base58 pubkey.
pub fn encode_open_args(args: &OpenArgs) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::with_capacity(32 + args.recipients.len() * 34);
    out.extend_from_slice(&args.salt.to_le_bytes());
    out.extend_from_slice(&args.deposit.to_le_bytes());
    out.extend_from_slice(&args.grace_period.to_le_bytes());
    out.extend_from_slice(&args.open_slot.to_le_bytes());
    out.extend(encode_distribution_entries(&args.recipients)?);
    Ok(out)
}

/// Decode `open` args, rejecting truncated data and trailing bytes.
///
/// # Errors
///
/// Returns [`CodecError`] when the buffer is truncated or the recipient
/// section length does not match the declared count.
pub fn decode_open_args(data: &[u8]) -> Result<OpenArgs, CodecError> {
    const FIXED: usize = 8 + 8 + 4 + 8 + 4;
    if data.len() < FIXED {
        return Err(CodecError::OpenArgsTruncated(data.len()));
    }
    let salt = u64_at(data, 0);
    let deposit = u64_at(data, 8);
    let grace_period = u32::from_le_bytes(
        data[16..20]
            .try_into()
            .map_err(|_| CodecError::OpenArgsTruncated(data.len()))?,
    );
    let open_slot = u64_at(data, 20);
    let count = u32::from_le_bytes(
        data[28..32]
            .try_into()
            .map_err(|_| CodecError::OpenArgsTruncated(data.len()))?,
    );
    let rest = &data[32..];
    let expected = usize::try_from(count)
        .unwrap_or(usize::MAX)
        .saturating_mul(34);
    if rest.len() != expected {
        return Err(CodecError::RecipientSection {
            actual: rest.len(),
            expected,
            count,
        });
    }
    let mut recipients = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let offset = i * 34;
        let pk_bytes: [u8; 32] = rest[offset..offset + 32]
            .try_into()
            .map_err(|_| CodecError::OpenArgsTruncated(data.len()))?;
        let bps = u16::from_le_bytes(
            rest[offset + 32..offset + 34]
                .try_into()
                .map_err(|_| CodecError::OpenArgsTruncated(data.len()))?,
        );
        recipients.push(ChannelSplit {
            recipient: Pubkey::from(pk_bytes).to_string(),
            bps,
        });
    }
    Ok(OpenArgs {
        salt,
        deposit,
        grace_period,
        open_slot,
        recipients,
    })
}

pub(crate) fn encode_distribution_entries(splits: &[ChannelSplit]) -> Result<Vec<u8>, CodecError> {
    let count = u32::try_from(splits.len()).map_err(|_| CodecError::TooManyRecipients)?;
    let mut out = Vec::with_capacity(4 + splits.len() * 34);
    out.extend_from_slice(&count.to_le_bytes());
    for split in splits {
        let recipient: Pubkey = split
            .recipient
            .parse()
            .map_err(|_| CodecError::InvalidRecipient(split.recipient.clone()))?;
        out.extend_from_slice(recipient.as_ref());
        out.extend_from_slice(&split.bps.to_le_bytes());
    }
    Ok(out)
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

/// Open-args codec failures.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// Buffer shorter than the fixed prefix.
    #[error("open args truncated: {0} bytes")]
    OpenArgsTruncated(usize),
    /// Recipient section length does not match the declared count.
    #[error(
        "open args recipient section is {actual} bytes, expected {expected} for {count} recipients"
    )]
    RecipientSection {
        /// Observed remaining bytes.
        actual: usize,
        /// Expected remaining bytes.
        expected: usize,
        /// Declared recipient count.
        count: u32,
    },
    /// Recipient is not a valid base58 pubkey.
    #[error("invalid distribution recipient {0}")]
    InvalidRecipient(String),
    /// Recipient count does not fit in u32.
    #[error("too many distribution recipients")]
    TooManyRecipients,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_args_round_trip() {
        let args = OpenArgs {
            salt: 42,
            deposit: 10_000,
            grace_period: 900,
            open_slot: 341_000_000,
            recipients: vec![ChannelSplit::full(
                "SysvarRent111111111111111111111111111111111",
            )],
        };
        let encoded = encode_open_args(&args).unwrap();
        let decoded = decode_open_args(&encoded).unwrap();
        assert_eq!(decoded, args);
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        let args = OpenArgs {
            salt: 1,
            deposit: 1,
            grace_period: 1,
            open_slot: 1,
            recipients: vec![],
        };
        let mut encoded = encode_open_args(&args).unwrap();
        encoded.push(0);
        assert!(decode_open_args(&encoded).is_err());
    }
}
