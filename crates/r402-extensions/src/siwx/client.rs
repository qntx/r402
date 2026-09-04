//! Client `SIGN-IN-WITH-X` header. Official `createSIWxClientExtension`.
//!
//! Uses `ClientExtension::on_payment_required`, not `enrich_payment_payload`.
//! Origin is the 402 challenge (configured public origin). Never `Host`.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use compact_str::CompactString;
use http::{HeaderMap, HeaderValue};
use r402_client::ClientExtension;
use r402_protocol::PaymentRequired;
use serde_json::Value;

use super::{SIWX_HTTP_HEADER, SIWX_KEY, SiwxError, SiwxOrigin, SiwxProof};

/// Wallet that can produce a CAIP-122 SIWX proof.
pub trait SiwxSigner: Send + Sync {
    /// `eip191` or `ed25519`. Matched against `supportedChains[].type`.
    fn signature_type(&self) -> &'static str;

    /// Wallet address (hex for EVM, Base58 for Solana).
    fn address(&self) -> CompactString;

    /// Signs the canonical CAIP-122 message. EVM returns `0x` hex; Solana Base58.
    fn sign_message<'a>(
        &'a self,
        message: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CompactString, SiwxError>> + Send + 'a>>;
}

/// Client extension that signs HTTP SIWX challenges for compatible wallets.
///
/// Signers are tried in registration order until one matches a declared chain
/// and produces a header. Failures are skipped (payment retry still proceeds).
pub struct SiwxClientExtension {
    signers: Vec<Arc<dyn SiwxSigner>>,
}

impl Default for SiwxClientExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for SiwxClientExtension {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SiwxClientExtension")
            .field("signers", &self.signers.len())
            .finish()
    }
}

impl SiwxClientExtension {
    /// Empty signer list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            signers: Vec::new(),
        }
    }

    /// Appends a wallet signer.
    #[must_use]
    pub fn with_signer(mut self, signer: impl SiwxSigner + 'static) -> Self {
        self.signers.push(Arc::new(signer));
        self
    }
}

impl ClientExtension for SiwxClientExtension {
    fn key(&self) -> &'static str {
        SIWX_KEY
    }

    async fn on_payment_required(&self, payment_required: &PaymentRequired) -> HeaderMap {
        sign_header(self, payment_required)
            .await
            .map_or_else(HeaderMap::new, |encoded| siwx_header_map(&encoded))
    }
}

fn siwx_header_map(encoded: &str) -> HeaderMap {
    let Ok(value) = HeaderValue::from_str(encoded) else {
        return HeaderMap::new();
    };
    let mut headers = HeaderMap::new();
    let _ = headers.insert(SIWX_HTTP_HEADER, value);
    headers
}

async fn sign_header(
    extension: &SiwxClientExtension,
    payment_required: &PaymentRequired,
) -> Option<String> {
    let declaration = payment_required.extensions.get(SIWX_KEY)?.to_value();
    if !challenge_bound_to_resource(&declaration, payment_required.resource.url.as_str()) {
        return None;
    }
    let info = declaration.get("info")?;
    let chains = declaration.get("supportedChains")?.as_array()?;
    for signer in &extension.signers {
        let Some((chain_id, signature_type, signature_scheme)) =
            matching_chain(chains, signer.signature_type())
        else {
            continue;
        };
        let Some(mut proof) = proof_from_info(
            info,
            &chain_id,
            &signature_type,
            signature_scheme,
            signer.address(),
        ) else {
            continue;
        };
        let Ok(message) = proof.signing_message() else {
            continue;
        };
        let Ok(signature) = signer.sign_message(&message).await else {
            continue;
        };
        proof.signature = signature;
        if let Ok(encoded) = proof.encode_header() {
            return Some(encoded);
        }
    }
    None
}

/// Domain/URI origin must match `PaymentRequired.resource.url` (configured
/// public origin). Never `Host`.
fn challenge_bound_to_resource(declaration: &Value, resource_url: &str) -> bool {
    let Some(info) = declaration.get("info") else {
        return false;
    };
    let Some(domain) = info.get("domain").and_then(Value::as_str) else {
        return false;
    };
    let Some(uri) = info.get("uri").and_then(Value::as_str) else {
        return false;
    };
    let Ok(resource_origin) = SiwxOrigin::parse(resource_url) else {
        return false;
    };
    let Ok(uri_origin) = SiwxOrigin::parse(uri) else {
        return false;
    };
    domain == resource_origin.domain() && uri_origin.as_str() == resource_origin.as_str()
}

fn matching_chain(
    chains: &[Value],
    signature_type: &str,
) -> Option<(CompactString, CompactString, Option<CompactString>)> {
    for chain in chains {
        let Some(chain_type) = chain.get("type").and_then(Value::as_str) else {
            continue;
        };
        if chain_type != signature_type {
            continue;
        }
        let Some(chain_id) = chain.get("chainId").and_then(Value::as_str) else {
            continue;
        };
        let signature_scheme = chain
            .get("signatureScheme")
            .and_then(Value::as_str)
            .map(CompactString::from);
        return Some((chain_id.into(), chain_type.into(), signature_scheme));
    }
    None
}

fn proof_from_info(
    info: &Value,
    chain_id: &str,
    signature_type: &str,
    signature_scheme: Option<CompactString>,
    address: CompactString,
) -> Option<SiwxProof> {
    Some(SiwxProof {
        address,
        signature: CompactString::default(),
        domain: required_str(info, "domain")?,
        uri: required_str(info, "uri")?,
        version: required_str(info, "version")?,
        nonce: required_str(info, "nonce")?,
        issued_at: required_str(info, "issuedAt")?,
        expiration_time: opt_str(info, "expirationTime"),
        not_before: opt_str(info, "notBefore"),
        statement: opt_str(info, "statement"),
        request_id: opt_str(info, "requestId"),
        resources: opt_str_array(info, "resources"),
        chain_id: chain_id.into(),
        signature_type: signature_type.into(),
        signature_scheme,
    })
}

fn required_str(info: &Value, name: &str) -> Option<CompactString> {
    info.get(name)
        .and_then(Value::as_str)
        .map(CompactString::from)
}

fn opt_str(info: &Value, name: &str) -> Option<CompactString> {
    info.get(name)
        .and_then(Value::as_str)
        .map(CompactString::from)
}

fn opt_str_array(info: &Value, name: &str) -> Vec<CompactString> {
    info.get(name)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(CompactString::from)
                .collect()
        })
        .unwrap_or_default()
}
