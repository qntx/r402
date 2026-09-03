//! 402 challenge advertisement for `sign-in-with-x`.
//!
//! Per-request nonce and timestamps are supplied by the HTTP layer. This
//! module does not read `Host`.

use compact_str::CompactString;
use r402_protocol::extension::{AdvertiseContext, Extension};
use r402_protocol::payment::ExtensionEntry;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{DEFAULT_CHALLENGE_TTL, SIWX_KEY, SiwxError, SiwxOrigin};

/// One entry in `supportedChains`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiwxChain {
    /// CAIP-2 chain identifier.
    pub chain_id: CompactString,
    /// Signature algorithm (`eip191` or `ed25519`).
    pub signature_type: CompactString,
}

impl SiwxChain {
    /// EVM personal-sign chain.
    #[must_use]
    pub fn eip191(chain_id: impl Into<CompactString>) -> Self {
        Self {
            chain_id: chain_id.into(),
            signature_type: CompactString::from("eip191"),
        }
    }

    /// Solana ed25519 chain.
    #[must_use]
    pub fn ed25519(chain_id: impl Into<CompactString>) -> Self {
        Self {
            chain_id: chain_id.into(),
            signature_type: CompactString::from("ed25519"),
        }
    }
}

/// Resource-server SIWX declaration. Challenge fields are per-request.
#[derive(Debug, Clone)]
pub struct SiwxExtension {
    origin: SiwxOrigin,
    supported_chains: Vec<SiwxChain>,
    statement: Option<CompactString>,
}

impl SiwxExtension {
    /// Constructs an extension bound to a configured public origin.
    #[must_use]
    pub const fn new(origin: SiwxOrigin) -> Self {
        Self {
            origin,
            supported_chains: Vec::new(),
            statement: None,
        }
    }

    /// Adds a supported authentication chain.
    #[must_use]
    pub fn with_chain(mut self, chain: SiwxChain) -> Self {
        self.supported_chains.push(chain);
        self
    }

    /// Sets the CAIP-122 statement shown to the wallet.
    #[must_use]
    pub fn with_statement(mut self, statement: impl Into<CompactString>) -> Self {
        self.statement = Some(statement.into());
        self
    }

    /// Configured public origin (never `Host`).
    #[must_use]
    pub const fn origin(&self) -> &SiwxOrigin {
        &self.origin
    }

    /// Builds the per-request 402 challenge entry.
    ///
    /// `nonce_hex` must be 32 lowercase/uppercase hex characters.
    /// `issued_at` / `expiration_time` are ISO 8601 timestamps supplied by
    /// the HTTP layer (default expiry is issuedAt + 5 minutes).
    ///
    /// # Errors
    ///
    /// [`SiwxError::Nonce`] when `nonce_hex` is not 32 hex characters.
    pub fn challenge(
        &self,
        path: &str,
        nonce_hex: &str,
        issued_at: &str,
        expiration_time: &str,
    ) -> Result<ExtensionEntry, SiwxError> {
        if nonce_hex.len() != 32 || !nonce_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(SiwxError::Nonce);
        }
        let uri = self.origin.uri(path);
        let mut info = json!({
            "domain": self.origin.domain(),
            "uri": uri,
            "version": "1",
            "nonce": nonce_hex,
            "issuedAt": issued_at,
            "expirationTime": expiration_time,
            "resources": [uri],
        });
        if let Some(statement) = &self.statement
            && let Some(obj) = info.as_object_mut()
        {
            let _ = obj.insert("statement".into(), Value::String(statement.to_string()));
        }
        let supported: Vec<Value> = self
            .supported_chains
            .iter()
            .map(|c| {
                json!({
                    "chainId": c.chain_id,
                    "type": c.signature_type,
                })
            })
            .collect();
        Ok(ExtensionEntry::raw(json!({
            "info": info,
            "supportedChains": supported,
            "schema": client_proof_schema(),
        })))
    }

    /// Fresh nonce and timestamps for this request path.
    ///
    /// `expirationTime` is `issuedAt` + 5 minutes. Domain/URI come from the
    /// configured origin, never `Host`.
    ///
    /// # Errors
    ///
    /// [`SiwxError::IssuedAt`] / [`SiwxError::ExpirationTime`] if RFC 3339
    /// formatting fails.
    pub fn challenge_now(&self, path: &str) -> Result<ExtensionEntry, SiwxError> {
        let issued = OffsetDateTime::now_utc();
        let expires = issued + DEFAULT_CHALLENGE_TTL;
        let issued_at = issued.format(&Rfc3339).map_err(|_| SiwxError::IssuedAt)?;
        let expiration_time = expires
            .format(&Rfc3339)
            .map_err(|_| SiwxError::ExpirationTime)?;
        self.challenge(path, &random_nonce_hex(), &issued_at, &expiration_time)
    }
}

fn random_nonce_hex() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

impl Extension for SiwxExtension {
    fn id(&self) -> &'static str {
        SIWX_KEY
    }

    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        // Nonce and timestamps are per-request; HTTP calls [`Self::challenge`].
        None
    }
}

fn client_proof_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "domain": { "type": "string" },
            "address": { "type": "string" },
            "statement": { "type": "string" },
            "uri": { "type": "string", "format": "uri" },
            "version": { "type": "string" },
            "chainId": { "type": "string" },
            "type": { "type": "string" },
            "nonce": { "type": "string" },
            "issuedAt": { "type": "string", "format": "date-time" },
            "expirationTime": { "type": "string", "format": "date-time" },
            "notBefore": { "type": "string", "format": "date-time" },
            "requestId": { "type": "string" },
            "resources": { "type": "array", "items": { "type": "string", "format": "uri" } },
            "signature": { "type": "string" }
        },
        "required": [
            "domain",
            "address",
            "uri",
            "version",
            "chainId",
            "type",
            "nonce",
            "issuedAt",
            "signature"
        ]
    })
}
