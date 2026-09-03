//! `builder-code` extension — ERC-8021 Schema 2 attribution (official wire).
//!
//! Server declare `a` (and optional `s[]`) on 402. Facilitator appends a
//! CBOR Schema 2 suffix at EVM settle (`w` plus echoed `a`/`s`). Client `s[]`
//! enrich is a helper here; `PaymentClient` wiring is a later PR.

mod cbor;
mod client;
mod facilitator;

pub use cbor::{encode_builder_code_suffix, parse_builder_code_suffix_from_calldata};
pub use client::BuilderCodeClient;
use compact_str::CompactString;
pub use facilitator::{BuilderCodeFacilitatorConfig, BuilderCodeFacilitatorExtension};
use r402_protocol::extension::{AdvertiseContext, Extension};
use r402_protocol::payment::ExtensionEntry;
use serde_json::{Value, json};

/// Stable extension key on the wire.
pub const BUILDER_CODE: &str = "builder-code";

/// ERC-8021 marker (16 bytes) appended at the end of every suffix.
pub const ERC_8021_MARKER: [u8; 16] = [
    0x80, 0x21, 0x80, 0x21, 0x80, 0x21, 0x80, 0x21, 0x80, 0x21, 0x80, 0x21, 0x80, 0x21, 0x80, 0x21,
];

/// ERC-8021 marker as lowercase hex (no `0x`).
pub const ERC_8021_MARKER_HEX: &str = "80218021802180218021802180218021";

/// Schema 2 identifier byte.
pub const SCHEMA_2_ID: u8 = 0x02;

/// Maximum client-provided service codes reserved in `s`.
pub const MAX_CLIENT_SERVICE_CODES: usize = 5;

/// Maximum server-declared service codes reserved in `s`.
pub const MAX_SERVER_SERVICE_CODES: usize = 5;

/// Maximum facilitator-appended service codes reserved in `s`.
pub const MAX_FACILITATOR_SERVICE_CODES: usize = 1;

/// Maximum encoded `s` entries: client + server + facilitator reservations.
pub const MAX_SERVICE_CODES: usize =
    MAX_CLIENT_SERVICE_CODES + MAX_SERVER_SERVICE_CODES + MAX_FACILITATOR_SERVICE_CODES;

/// Echoed client+server budget before the facilitator appends its own code.
pub const MAX_ECHOED_SERVICE_CODES: usize = MAX_CLIENT_SERVICE_CODES + MAX_SERVER_SERVICE_CODES;

/// Builder-code declaration / suffix fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuilderCodeData {
    /// App code (`a`).
    pub a: Option<CompactString>,
    /// Wallet / facilitator code (`w`).
    pub w: Option<CompactString>,
    /// Service codes (`s`).
    pub s: Vec<CompactString>,
}

impl BuilderCodeData {
    /// Whether no attribution fields are present.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.a.is_none() && self.w.is_none() && self.s.is_empty()
    }
}

/// Invalid builder-code at declaration or construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BuilderCodeError {
    /// Code failed `^[a-z0-9_]{1,32}$`.
    #[error(
        "Invalid builder code: \"{0}\". Must be 1-32 characters, lowercase alphanumeric and underscores only."
    )]
    InvalidCode(String),
    /// More service codes than this party's reservation.
    #[error("Too many service codes: {got} exceeds the maximum of {max}.")]
    TooManyServiceCodes {
        /// Observed count.
        got: usize,
        /// Party reservation.
        max: usize,
    },
}

/// `^[a-z0-9_]{1,32}$`.
#[must_use]
pub fn is_valid_builder_code(code: &str) -> bool {
    let len = code.len();
    (1..=32).contains(&len)
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub(crate) fn validate_code(code: &str) -> Result<(), BuilderCodeError> {
    if is_valid_builder_code(code) {
        Ok(())
    } else {
        Err(BuilderCodeError::InvalidCode(code.to_owned()))
    }
}

pub(crate) fn validate_codes<'a, I>(
    codes: I,
    max: usize,
) -> Result<Vec<CompactString>, BuilderCodeError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut out = Vec::new();
    for code in codes {
        validate_code(code)?;
        out.push(CompactString::from(code));
    }
    if out.len() > max {
        return Err(BuilderCodeError::TooManyServiceCodes {
            got: out.len(),
            max,
        });
    }
    Ok(out)
}

/// Official JSON Schema for builder-code `info`.
fn info_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "a": {
                "type": "string",
                "pattern": "^[a-z0-9_]{1,32}$",
                "description": "App builder code",
            },
            "w": {
                "type": "string",
                "pattern": "^[a-z0-9_]{1,32}$",
                "description": "Wallet builder code",
            },
            "s": {
                "type": "array",
                "maxItems": MAX_SERVICE_CODES,
                "items": {
                    "type": "string",
                    "pattern": "^[a-z0-9_]{1,32}$",
                },
                "description": "Service builder codes",
            },
        },
        "additionalProperties": false,
    })
}

/// Declares `builder-code` for `PaymentRequired.extensions`.
///
/// # Errors
///
/// Invalid app/service code, or more than [`MAX_SERVER_SERVICE_CODES`] services.
pub fn declare_builder_code_extension(
    app_code: &str,
    service_codes: &[&str],
) -> Result<ExtensionEntry, BuilderCodeError> {
    validate_code(app_code)?;
    let codes = validate_codes(service_codes.iter().copied(), MAX_SERVER_SERVICE_CODES)?;
    let info = if codes.is_empty() {
        json!({ "a": app_code })
    } else {
        json!({ "a": app_code, "s": codes })
    };
    Ok(ExtensionEntry::with_schema(info, info_schema()))
}

/// Resource-server `builder-code` declaration (`a` plus optional server `s[]`).
#[derive(Debug, Clone)]
pub struct BuilderCodeExtension {
    app_code: CompactString,
    service_codes: Vec<CompactString>,
}

impl BuilderCodeExtension {
    /// App code only.
    ///
    /// # Errors
    ///
    /// [`BuilderCodeError::InvalidCode`].
    pub fn try_new(app_code: impl AsRef<str>) -> Result<Self, BuilderCodeError> {
        let app_code = app_code.as_ref();
        validate_code(app_code)?;
        Ok(Self {
            app_code: CompactString::from(app_code),
            service_codes: Vec::new(),
        })
    }

    /// Server-declared service codes (max [`MAX_SERVER_SERVICE_CODES`]).
    ///
    /// # Errors
    ///
    /// Invalid code or too many codes.
    pub fn with_service_codes<'a, I>(mut self, codes: I) -> Result<Self, BuilderCodeError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.service_codes = validate_codes(codes, MAX_SERVER_SERVICE_CODES)?;
        Ok(self)
    }
}

impl Extension for BuilderCodeExtension {
    fn id(&self) -> &'static str {
        BUILDER_CODE
    }

    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        let refs: Vec<&str> = self
            .service_codes
            .iter()
            .map(CompactString::as_str)
            .collect();
        declare_builder_code_extension(self.app_code.as_str(), &refs).ok()
    }
}

pub(crate) fn extract_info(extensions: &r402_protocol::payment::Extensions) -> Option<Value> {
    let entry = extensions.get(BUILDER_CODE)?;
    let value = entry.to_value();
    if let Some(info) = value.get("info").cloned()
        && info.is_object()
    {
        return Some(info);
    }
    Some(value)
}

pub(crate) fn normalize_service_codes(raw: &Value) -> Vec<CompactString> {
    match raw {
        Value::String(s) => vec![CompactString::from(s.as_str())],
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(CompactString::from)
            .collect(),
        _ => Vec::new(),
    }
}
