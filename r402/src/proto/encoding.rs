//! Base64 encoding utilities for x402 wire format.

use std::fmt::{self, Display, Formatter};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as b64;

/// A wrapper for base64-encoded byte data.
///
/// This type holds bytes that represent base64-encoded data and provides
/// methods for encoding and decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base64Bytes(pub Vec<u8>);

impl Base64Bytes {
    /// Decodes the base64 string bytes to raw binary data.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid base64.
    pub fn decode(&self) -> Result<Vec<u8>, base64::DecodeError> {
        b64.decode(&self.0)
    }

    /// Encodes raw binary data into base64 string bytes.
    pub fn encode<T: AsRef<[u8]>>(input: T) -> Self {
        let encoded = b64.encode(input.as_ref());
        Self(encoded.into_bytes())
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
        write!(f, "{}", String::from_utf8_lossy(&self.0))
    }
}
