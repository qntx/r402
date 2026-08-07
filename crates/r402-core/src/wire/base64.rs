//! Lazy base64 byte container.

use std::fmt::{self, Display, Formatter};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

/// Raw bytes holding the base64-encoded ASCII representation of some payload.
///
/// Useful as a field type for x402 wire messages that transport arbitrary
/// binary data (for example, the raw Solana transaction wrapped in
/// [`PaymentPayload`](super::PaymentPayload)). Encoding is eager, decoding
/// is deferred.
///
/// # Examples
///
/// ```
/// use r402_core::wire::Base64Bytes;
///
/// let encoded = Base64Bytes::encode(b"hello world");
/// let decoded = encoded.decode().unwrap();
/// assert_eq!(decoded, b"hello world");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base64Bytes(pub Vec<u8>);

impl Base64Bytes {
    /// Decodes the inner base64 bytes into their raw binary form.
    ///
    /// # Errors
    ///
    /// Returns a [`base64::DecodeError`] when the stored bytes are not
    /// valid base64.
    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        B64.decode(&self.0)
    }

    /// Encodes arbitrary bytes into a [`Base64Bytes`] wrapper.
    #[must_use]
    pub fn encode<T: AsRef<[u8]>>(input: T) -> Self {
        Self(B64.encode(input.as_ref()).into_bytes())
    }
}

impl AsRef<[u8]> for Base64Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<&[u8]> for Base64Bytes {
    fn from(slice: &[u8]) -> Self {
        Self(slice.to_vec())
    }
}

impl Display for Base64Bytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}
