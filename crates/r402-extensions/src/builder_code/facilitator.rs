//! Facilitator ERC-8021 suffix builder (`w` + echoed `a`/`s`).

use compact_str::CompactString;
use r402_protocol::payment::Extensions;
use serde_json::Value;

use super::cbor::encode_builder_code_suffix;
use super::{
    BuilderCodeData, BuilderCodeError, MAX_ECHOED_SERVICE_CODES, extract_info,
    is_valid_builder_code, normalize_service_codes, validate_code,
};

/// Facilitator-side `w` / extra `s` configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuilderCodeFacilitatorConfig {
    /// Wallet builder code (`w`).
    pub builder_code: Option<CompactString>,
    /// Facilitator service code appended to `s` (max 1, deduped).
    pub service_code: Option<CompactString>,
}

/// Facilitator extension that encodes the ERC-8021 Schema 2 calldata suffix.
#[derive(Debug, Clone, Default)]
pub struct BuilderCodeFacilitatorExtension {
    config: BuilderCodeFacilitatorConfig,
}

impl BuilderCodeFacilitatorExtension {
    /// Empty config: encode payload `a`/`s` only, no `w`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Validates optional `w` / facilitator `s`.
    ///
    /// # Errors
    ///
    /// [`BuilderCodeError::InvalidCode`].
    pub fn try_from_config(config: BuilderCodeFacilitatorConfig) -> Result<Self, BuilderCodeError> {
        if let Some(code) = config.builder_code.as_ref() {
            validate_code(code)?;
        }
        if let Some(code) = config.service_code.as_ref() {
            validate_code(code)?;
        }
        Ok(Self { config })
    }

    /// Sets `w`.
    ///
    /// # Errors
    ///
    /// Invalid code.
    pub fn with_builder_code(mut self, code: impl AsRef<str>) -> Result<Self, BuilderCodeError> {
        let code = code.as_ref();
        validate_code(code)?;
        self.config.builder_code = Some(CompactString::from(code));
        Ok(self)
    }

    /// Sets the facilitator's own `s` entry.
    ///
    /// # Errors
    ///
    /// Invalid code.
    pub fn with_service_code(mut self, code: impl AsRef<str>) -> Result<Self, BuilderCodeError> {
        let code = code.as_ref();
        validate_code(code)?;
        self.config.service_code = Some(CompactString::from(code));
        Ok(self)
    }

    /// Builds the suffix bytes, or `None` when no attribution is present.
    ///
    /// `x402_version != 2` drops client `a` (official v1 echo-gate skip).
    #[must_use]
    pub fn build_data_suffix(&self, extensions: &Extensions, x402_version: u8) -> Option<Vec<u8>> {
        let info = extract_info(extensions);
        let a = if x402_version == 2 {
            info.as_ref()
                .and_then(|v| v.get("a"))
                .and_then(Value::as_str)
                .filter(|s| is_valid_builder_code(s))
                .map(CompactString::from)
        } else {
            None
        };
        let echoed = resolve_service_codes(info.as_ref().and_then(|v| v.get("s")));
        let s = match self.config.service_code.as_ref() {
            Some(fac) if !echoed.iter().any(|c| c == fac) => {
                let mut out = echoed;
                out.push(fac.clone());
                out
            }
            _ => echoed,
        };
        let data = BuilderCodeData {
            a,
            w: self.config.builder_code.clone(),
            s,
        };
        if data.is_empty() {
            return None;
        }
        Some(encode_builder_code_suffix(&data))
    }
}

fn resolve_service_codes(raw: Option<&Value>) -> Vec<CompactString> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    normalize_service_codes(raw)
        .into_iter()
        .filter(|c| is_valid_builder_code(c))
        .take(MAX_ECHOED_SERVICE_CODES)
        .collect()
}
