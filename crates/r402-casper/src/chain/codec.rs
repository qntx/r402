//! Unprefixed lowercase hex for Casper wire values.
//!
//! Account hashes, package hashes, public keys, signatures, and nonces travel
//! as hex without a `0x` prefix.

/// Length, in hex characters, of a 32-byte Casper hash.
pub const HASH_HEX_LEN: usize = 64;

/// Errors produced while decoding a hex string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HexDecodeError {
    /// The input had an odd number of characters.
    #[error("hex string must have an even number of characters, got {0}")]
    OddLength(usize),
    /// The input contained a non-hex character.
    #[error("invalid hex character {0:?}")]
    InvalidCharacter(char),
    /// The decoded byte length did not match the expected length.
    #[error("expected {expected} bytes, got {actual}")]
    UnexpectedLength {
        /// Required byte length.
        expected: usize,
        /// Observed byte length.
        actual: usize,
    },
}

/// Returns `true` when every character is an ASCII hex digit.
#[must_use]
pub fn is_hex(input: &str) -> bool {
    !input.is_empty() && input.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Decodes an unprefixed hex string into bytes.
///
/// # Errors
///
/// Returns [`HexDecodeError`] when the input length is odd or a character is
/// not an ASCII hex digit.
pub fn decode(input: &str) -> Result<Vec<u8>, HexDecodeError> {
    if !input.len().is_multiple_of(2) {
        return Err(HexDecodeError::OddLength(input.len()));
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let Some(hi_byte) = bytes.get(index).copied() else {
            return Err(HexDecodeError::OddLength(input.len()));
        };
        let Some(lo_byte) = bytes.get(index + 1).copied() else {
            return Err(HexDecodeError::OddLength(input.len()));
        };
        let hi = nibble(hi_byte)?;
        let lo = nibble(lo_byte)?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    Ok(out)
}

/// Decodes an unprefixed hex string into exactly `N` bytes.
///
/// # Errors
///
/// Returns [`HexDecodeError`] when decoding fails or the decoded length is
/// not exactly `N`.
pub fn decode_exact<const N: usize>(input: &str) -> Result<[u8; N], HexDecodeError> {
    let bytes = decode(input)?;
    <[u8; N]>::try_from(bytes.as_slice()).map_err(|_| HexDecodeError::UnexpectedLength {
        expected: N,
        actual: bytes.len(),
    })
}

/// Encodes bytes as a lowercase hex string.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, byte| {
            let _unused = write!(acc, "{byte:02x}");
            acc
        })
}

fn nibble(byte: u8) -> Result<u8, HexDecodeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(HexDecodeError::InvalidCharacter(char::from(byte))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_lowercase() {
        let bytes = [0x00u8, 0x0f, 0xa1, 0xff];
        assert_eq!(encode(&bytes), "000fa1ff");
        assert_eq!(decode("000fa1ff").unwrap(), bytes);
    }

    #[test]
    fn accepts_uppercase_input() {
        assert_eq!(decode("A1B2").unwrap(), vec![0xa1, 0xb2]);
    }

    #[test]
    fn rejects_odd_length() {
        assert_eq!(decode("abc").unwrap_err(), HexDecodeError::OddLength(3));
    }

    #[test]
    fn rejects_non_hex_character() {
        assert_eq!(
            decode("zz").unwrap_err(),
            HexDecodeError::InvalidCharacter('z')
        );
    }

    #[test]
    fn decode_exact_enforces_length() {
        assert!(decode_exact::<2>("a1b2").is_ok());
        assert_eq!(
            decode_exact::<4>("a1b2").unwrap_err(),
            HexDecodeError::UnexpectedLength {
                expected: 4,
                actual: 2
            }
        );
    }

    #[test]
    fn is_hex_rejects_empty_and_garbage() {
        assert!(is_hex("00ff"));
        assert!(!is_hex(""));
        assert!(!is_hex("00 ff"));
    }
}
