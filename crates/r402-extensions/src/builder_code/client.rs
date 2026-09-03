//! Client helper that attaches `s[]` to a payment payload.

use compact_str::CompactString;
use r402_protocol::payment::{ExtensionEntry, Extensions};
use serde_json::json;

use super::{BUILDER_CODE, BuilderCodeError, MAX_CLIENT_SERVICE_CODES, validate_codes};

/// Client-side builder-code codes (`s`), not wired to `PaymentClient` in this PR.
#[derive(Debug, Clone)]
pub struct BuilderCodeClient {
    service_codes: Vec<CompactString>,
}

impl BuilderCodeClient {
    /// One or more client service codes (max [`MAX_CLIENT_SERVICE_CODES`]).
    ///
    /// # Errors
    ///
    /// Invalid code or too many codes.
    pub fn try_new<'a, I>(codes: I) -> Result<Self, BuilderCodeError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        Ok(Self {
            service_codes: validate_codes(codes, MAX_CLIENT_SERVICE_CODES)?,
        })
    }

    /// Payload extension `{ info: { s: [...] } }` (official client shape).
    #[must_use]
    pub fn payload_entry(&self) -> ExtensionEntry {
        ExtensionEntry::info(json!({ "s": self.service_codes }))
    }

    /// Inserts this client's `s[]` under `builder-code`.
    pub fn enrich_extensions(&self, extensions: &mut Extensions) {
        extensions.insert(BUILDER_CODE, self.payload_entry());
    }

    /// Configured service codes.
    #[must_use]
    pub fn service_codes(&self) -> &[CompactString] {
        &self.service_codes
    }
}
