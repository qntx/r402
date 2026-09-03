//! Mirror Node REST client used for preflight and signature verification.

use std::fmt::{Debug, Formatter};

use base64::Engine;
use hedera::{AnyTransaction, PublicKey};
use serde::Deserialize;

use super::account::is_hbar_asset;

/// Errors from Mirror Node REST reads.
#[derive(Debug, thiserror::Error)]
pub enum HederaMirrorError {
    /// HTTP or transport failure.
    #[error("hedera mirror error: {0}")]
    Http(String),
    /// Response JSON could not be parsed.
    #[error("hedera mirror parse error: {0}")]
    Parse(String),
}

/// Account resolution used by alias policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HederaAccountResolution {
    /// Whether the identifier resolved to an on-chain account.
    pub exists: bool,
    /// Whether the identifier is an alias rather than a numeric entity id.
    pub is_alias: bool,
}

/// Result of a preflight balance / association check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HederaPreflightResult {
    /// Whether the transfer is expected to succeed.
    pub ok: bool,
    /// Machine-readable failure reason.
    pub reason: Option<String>,
    /// Human-readable detail.
    pub message: Option<String>,
}

impl HederaPreflightResult {
    /// Successful preflight.
    #[must_use]
    pub const fn ok() -> Self {
        Self {
            ok: true,
            reason: None,
            message: None,
        }
    }

    /// Failed preflight.
    #[must_use]
    pub fn fail(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: Some(reason.into()),
            message: Some(message.into()),
        }
    }

    /// Formats `reason: message` for the verify `invalidMessage` field.
    #[must_use]
    pub fn invalid_message(&self) -> Option<String> {
        match (&self.reason, &self.message) {
            (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
            (Some(reason), None) => Some(reason.clone()),
            (None, Some(message)) => Some(message.clone()),
            (None, None) => None,
        }
    }
}

/// Result of payer-signature verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HederaSignatureResult {
    /// Whether the payer key signed the frozen body.
    pub ok: bool,
    /// Machine-readable failure reason.
    pub reason: Option<String>,
    /// Human-readable detail.
    pub message: Option<String>,
}

impl HederaSignatureResult {
    /// Valid signature.
    #[must_use]
    pub const fn ok() -> Self {
        Self {
            ok: true,
            reason: None,
            message: None,
        }
    }

    /// Invalid or unverifiable signature.
    #[must_use]
    pub fn fail(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: Some(reason.into()),
            message: Some(message.into()),
        }
    }

    /// Formats `reason: message` for the verify `invalidMessage` field.
    #[must_use]
    pub fn invalid_message(&self) -> Option<String> {
        match (&self.reason, &self.message) {
            (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
            (Some(reason), None) => Some(reason.clone()),
            (None, Some(message)) => Some(message.clone()),
            (None, None) => None,
        }
    }
}

/// Mirror Node REST client.
#[derive(Clone)]
pub struct HederaMirrorClient {
    http: reqwest::Client,
    base_url: String,
}

impl Debug for HederaMirrorClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HederaMirrorClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

fn is_loopback_url(url: &str) -> bool {
    url.contains("://127.0.0.1")
        || url.contains("://localhost")
        || url.contains("://[::1]")
        || url.contains("://::1")
}

impl HederaMirrorClient {
    /// Connects to the given Mirror Node base URL (no trailing slash).
    #[must_use]
    pub fn connect(base_url: impl Into<String>) -> Self {
        let base_url = trim_slash(base_url.into());
        let mut builder = reqwest::Client::builder();
        if is_loopback_url(&base_url) {
            // HTTP_PROXY must not capture loopback mock servers or local Mirror.
            builder = builder.no_proxy();
        }
        let http = builder.build().unwrap_or_else(|_| reqwest::Client::new());
        Self { http, base_url }
    }

    /// Fetches `/api/v1/accounts/{id}`.
    ///
    /// # Errors
    ///
    /// Returns [`HederaMirrorError`] on HTTP or JSON failure.
    pub async fn account(&self, account_id: &str) -> Result<MirrorAccount, HederaMirrorError> {
        let url = format!(
            "{}/api/v1/accounts/{}",
            self.base_url,
            urlencoding(account_id)
        );
        self.get_json(&url).await
    }

    /// Fetches token relationships, optionally filtered by token id.
    ///
    /// # Errors
    ///
    /// Returns [`HederaMirrorError`] on HTTP or JSON failure.
    pub async fn account_tokens(
        &self,
        account_id: &str,
        token_id: Option<&str>,
    ) -> Result<MirrorTokensResponse, HederaMirrorError> {
        let mut url = format!(
            "{}/api/v1/accounts/{}/tokens",
            self.base_url,
            urlencoding(account_id)
        );
        if let Some(token_id) = token_id {
            url.push_str("?token.id=");
            url.push_str(&urlencoding(token_id));
        }
        self.get_json(&url).await
    }

    /// Fetches a Mirror Node path returned in `links.next`.
    ///
    /// # Errors
    ///
    /// Returns [`HederaMirrorError`] on HTTP or JSON failure.
    pub async fn get_path<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, HederaMirrorError> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            path.to_owned()
        } else {
            format!("{}{path}", self.base_url)
        };
        self.get_json(&url).await
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
    ) -> Result<T, HederaMirrorError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| HederaMirrorError::Http(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(HederaMirrorError::Http(format!(
                "Mirror Node request failed with status {status}"
            )));
        }
        response
            .json()
            .await
            .map_err(|e| HederaMirrorError::Parse(e.to_string()))
    }
}

/// Account fields read from `/accounts/{id}`.
#[derive(Debug, Clone, Deserialize)]
pub struct MirrorAccount {
    /// Tinybar balance wrapper.
    #[serde(default)]
    pub balance: MirrorBalance,
    /// Auto-association slot cap (`-1` is unlimited).
    #[serde(default)]
    pub max_automatic_token_associations: i64,
    /// On-chain account key.
    #[serde(default)]
    pub key: Option<MirrorAccountKey>,
    /// Alias bytes or evm address when present.
    #[serde(default)]
    pub alias: Option<String>,
}

/// Balance object from the Mirror Node account endpoint.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct MirrorBalance {
    /// Tinybars held by the account.
    #[serde(default)]
    pub balance: i64,
}

/// Account key as returned by the Mirror Node.
#[derive(Debug, Clone, Deserialize)]
pub struct MirrorAccountKey {
    /// Key encoding: `ED25519`, `ECDSA_SECP256K1`, or `ProtobufEncoded`.
    #[serde(rename = "_type")]
    pub key_type: String,
    /// Hex-encoded key bytes.
    pub key: String,
}

/// A token relationship entry from `/accounts/{id}/tokens`.
#[derive(Debug, Clone, Deserialize)]
pub struct MirrorTokenRelationship {
    /// Token entity id.
    pub token_id: String,
    /// Token balance in atomic units.
    #[serde(default)]
    pub balance: i64,
    /// Whether this relationship used an auto-association slot.
    #[serde(default)]
    pub automatic_association: bool,
}

/// Paged response from `/accounts/{id}/tokens`.
#[derive(Debug, Clone, Deserialize)]
pub struct MirrorTokensResponse {
    /// Token relationships on this page.
    #[serde(default)]
    pub tokens: Vec<MirrorTokenRelationship>,
    /// Pagination links.
    #[serde(default)]
    pub links: MirrorLinks,
}

/// Pagination links from the Mirror Node.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MirrorLinks {
    /// Relative path of the next page, if any.
    #[serde(default)]
    pub next: Option<String>,
}

/// Runs the HBAR / HTS association preflight against a Mirror Node.
///
/// # Errors
///
/// Returns [`HederaMirrorError`] when Mirror Node requests fail.
pub async fn preflight_transfer(
    mirror: &HederaMirrorClient,
    payer: &str,
    pay_to: &str,
    asset: &str,
    amount: &str,
) -> Result<HederaPreflightResult, HederaMirrorError> {
    let required = amount
        .parse::<i128>()
        .map_err(|e| HederaMirrorError::Parse(e.to_string()))?;

    if is_hbar_asset(asset) {
        let account = mirror.account(payer).await?;
        let held = i128::from(account.balance.balance);
        if held < required {
            return Ok(HederaPreflightResult::fail(
                "insufficient_balance",
                format!("payer has {held} tinybars, needs {required}"),
            ));
        }
        return Ok(HederaPreflightResult::ok());
    }

    let payer_tokens = mirror.account_tokens(payer, Some(asset)).await?;
    let held = payer_tokens
        .tokens
        .first()
        .map_or(0, |t| i128::from(t.balance));
    if held < required {
        return Ok(HederaPreflightResult::fail(
            "insufficient_balance",
            format!("payer holds {held} of {asset}, needs {required}"),
        ));
    }

    if is_pay_to_associated(mirror, pay_to, asset).await? {
        return Ok(HederaPreflightResult::ok());
    }
    Ok(HederaPreflightResult::fail(
        "pay_to_not_associated",
        format!("payTo {pay_to} is not associated with {asset} and has no auto-association slots"),
    ))
}

async fn is_pay_to_associated(
    mirror: &HederaMirrorClient,
    pay_to: &str,
    asset: &str,
) -> Result<bool, HederaMirrorError> {
    let direct = mirror.account_tokens(pay_to, Some(asset)).await?;
    if !direct.tokens.is_empty() {
        return Ok(true);
    }

    let account = mirror.account(pay_to).await?;
    let max_auto = account.max_automatic_token_associations;
    if max_auto == -1 {
        return Ok(true);
    }
    if max_auto == 0 {
        return Ok(false);
    }

    let mut consumed = 0i64;
    let mut next = Some(format!("/api/v1/accounts/{pay_to}/tokens"));
    while let Some(path) = next {
        let page: MirrorTokensResponse = mirror.get_path(&path).await?;
        let auto_count = page
            .tokens
            .iter()
            .filter(|t| t.automatic_association)
            .count();
        let auto_count = i64::try_from(auto_count).unwrap_or(i64::MAX);
        consumed = consumed.saturating_add(auto_count);
        if consumed >= max_auto {
            return Ok(false);
        }
        next = page.links.next;
    }
    Ok(consumed < max_auto)
}

/// Resolves `payTo` against the Mirror Node for alias policy.
///
/// # Errors
///
/// Returns [`HederaMirrorError`] when the account lookup fails for a reason
/// other than not-found.
pub async fn resolve_account(
    mirror: &HederaMirrorClient,
    account_id_or_alias: &str,
) -> Result<HederaAccountResolution, HederaMirrorError> {
    match mirror.account(account_id_or_alias).await {
        Ok(_) => Ok(HederaAccountResolution {
            exists: true,
            is_alias: !super::account::is_entity_id(account_id_or_alias),
        }),
        Err(HederaMirrorError::Http(msg)) if msg.contains("status 404") => {
            Ok(HederaAccountResolution {
                exists: false,
                is_alias: !super::account::is_entity_id(account_id_or_alias),
            })
        }
        Err(err) => Err(err),
    }
}

/// Verifies that `payer` signed `transaction_base64` using the on-chain key.
///
/// # Errors
///
/// Returns [`HederaMirrorError`] when the account or key cannot be loaded.
pub async fn verify_payer_signature(
    mirror: &HederaMirrorClient,
    payer: &str,
    transaction_base64: &str,
) -> Result<HederaSignatureResult, HederaMirrorError> {
    let bytes = Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        transaction_base64,
    )
    .map_err(|e| HederaMirrorError::Parse(e.to_string()))?;
    let mut tx =
        AnyTransaction::from_bytes(&bytes).map_err(|e| HederaMirrorError::Parse(e.to_string()))?;
    let account = mirror.account(payer).await?;
    let Some(mirror_key) = account.key.as_ref() else {
        return Ok(HederaSignatureResult::fail(
            "signature_invalid",
            "could not resolve payer key",
        ));
    };
    let key = match parse_mirror_key(mirror_key) {
        Ok(key) => key,
        Err(err) => {
            return Ok(HederaSignatureResult::fail("signature_unverifiable", err));
        }
    };
    if key.signs(&mut tx) {
        Ok(HederaSignatureResult::ok())
    } else {
        Ok(HederaSignatureResult::fail(
            "signature_invalid",
            format!("payer {payer} did not sign the transaction"),
        ))
    }
}

/// Account key reconstructed from Mirror Node JSON.
#[derive(Debug, Clone)]
enum AccountKey {
    /// A single public key.
    Single(PublicKey),
    /// A key list, optionally thresholded.
    List {
        /// Nested keys.
        keys: Vec<Self>,
        /// Number of keys that must sign.
        threshold: usize,
    },
}

impl AccountKey {
    fn signs(&self, tx: &mut AnyTransaction) -> bool {
        match self {
            Self::Single(pk) => pk.verify_transaction(tx).is_ok(),
            Self::List { keys, threshold } => {
                keys.iter().filter(|k| k.signs(tx)).count() >= *threshold
            }
        }
    }
}

fn parse_mirror_key(mirror_key: &MirrorAccountKey) -> Result<AccountKey, String> {
    if mirror_key.key.is_empty() {
        return Err("could not resolve payer key".to_owned());
    }
    match mirror_key.key_type.as_str() {
        "ED25519" => {
            let bytes = decode_hex(&mirror_key.key)?;
            PublicKey::from_bytes_ed25519(&bytes)
                .map(AccountKey::Single)
                .map_err(|e| e.to_string())
        }
        "ECDSA_SECP256K1" => {
            let bytes = decode_hex(&mirror_key.key)?;
            PublicKey::from_bytes_ecdsa(&bytes)
                .map(AccountKey::Single)
                .map_err(|e| e.to_string())
        }
        "ProtobufEncoded" => {
            let bytes = decode_hex(&mirror_key.key)?;
            parse_proto_key(&bytes)
        }
        other => Err(format!("unrecognized mirror key type {other}")),
    }
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).map_err(|_| "invalid hex key".to_owned())
}

// `hedera::protobuf::FromProtobuf` is `pub(crate)`; `Key` has no public decode.
fn parse_proto_key(bytes: &[u8]) -> Result<AccountKey, String> {
    let mut rest = bytes;
    let mut found = None;
    while !rest.is_empty() {
        let (tag, after_tag) = read_varint(rest)?;
        rest = after_tag;
        let field = tag >> 3;
        let wire = tag & 7;
        if wire != 2 {
            return Err("unsupported protobuf key wire type".to_owned());
        }
        let (len, after_len) = read_varint(rest)?;
        rest = after_len;
        let len = usize::try_from(len).map_err(|_| "invalid protobuf key length".to_owned())?;
        if rest.len() < len {
            return Err("truncated protobuf key".to_owned());
        }
        let (payload, after_payload) = rest.split_at(len);
        rest = after_payload;
        found = Some(match field {
            2 => AccountKey::Single(
                PublicKey::from_bytes_ed25519(payload).map_err(|e| e.to_string())?,
            ),
            5 => parse_threshold_key(payload)?,
            6 => parse_key_list(payload, None)?,
            7 => {
                AccountKey::Single(PublicKey::from_bytes_ecdsa(payload).map_err(|e| e.to_string())?)
            }
            1 | 3 | 4 | 8 => return Err("unsupported account key kind".to_owned()),
            _ => continue,
        });
    }
    found.ok_or_else(|| "empty protobuf key".to_owned())
}

fn parse_threshold_key(bytes: &[u8]) -> Result<AccountKey, String> {
    let mut rest = bytes;
    let mut threshold = 0u32;
    let mut list = None;
    while !rest.is_empty() {
        let (tag, after_tag) = read_varint(rest)?;
        rest = after_tag;
        let field = tag >> 3;
        let wire = tag & 7;
        match (field, wire) {
            (1, 0) => {
                let (value, after) = read_varint(rest)?;
                rest = after;
                threshold = u32::try_from(value).map_err(|_| "invalid threshold".to_owned())?;
            }
            (2, 2) => {
                let (len, after_len) = read_varint(rest)?;
                rest = after_len;
                let len =
                    usize::try_from(len).map_err(|_| "invalid protobuf key length".to_owned())?;
                if rest.len() < len {
                    return Err("truncated threshold key list".to_owned());
                }
                let (payload, after_payload) = rest.split_at(len);
                rest = after_payload;
                list = Some(payload);
            }
            (_, 2) => {
                let (len, after_len) = read_varint(rest)?;
                rest = after_len;
                let len =
                    usize::try_from(len).map_err(|_| "invalid protobuf key length".to_owned())?;
                rest = rest
                    .get(len..)
                    .ok_or_else(|| "truncated protobuf key".to_owned())?;
            }
            (_, 0) => {
                let (_, after) = read_varint(rest)?;
                rest = after;
            }
            _ => return Err("unsupported threshold key field".to_owned()),
        }
    }
    let Some(list) = list else {
        return Err("threshold key missing key list".to_owned());
    };
    let threshold = if threshold == 0 {
        None
    } else {
        Some(threshold)
    };
    parse_key_list(list, threshold)
}

fn parse_key_list(bytes: &[u8], threshold: Option<u32>) -> Result<AccountKey, String> {
    let mut rest = bytes;
    let mut keys = Vec::new();
    while !rest.is_empty() {
        let (tag, after_tag) = read_varint(rest)?;
        rest = after_tag;
        let field = tag >> 3;
        let wire = tag & 7;
        if field != 1 || wire != 2 {
            return Err("unexpected key list field".to_owned());
        }
        let (len, after_len) = read_varint(rest)?;
        rest = after_len;
        let len = usize::try_from(len).map_err(|_| "invalid protobuf key length".to_owned())?;
        if rest.len() < len {
            return Err("truncated key list entry".to_owned());
        }
        let (payload, after_payload) = rest.split_at(len);
        rest = after_payload;
        keys.push(parse_proto_key(payload)?);
    }
    let threshold = threshold.map_or(keys.len(), |t| t as usize);
    Ok(AccountKey::List { keys, threshold })
}

fn read_varint(bytes: &[u8]) -> Result<(u64, &[u8]), String> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (i, b) in bytes.iter().copied().enumerate() {
        let bits = u64::from(b & 0x7f);
        value |= bits
            .checked_shl(shift)
            .ok_or_else(|| "varint overflow".to_owned())?;
        if b & 0x80 == 0 {
            let rest = bytes
                .get(i.saturating_add(1)..)
                .ok_or_else(|| "truncated varint".to_owned())?;
            return Ok((value, rest));
        }
        shift = shift.saturating_add(7);
        if shift > 63 {
            return Err("varint overflow".to_owned());
        }
    }
    Err("truncated varint".to_owned())
}

fn trim_slash(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                let hi = HEX.get(usize::from(b >> 4)).copied().unwrap_or(b'0');
                let lo = HEX.get(usize::from(b & 0x0f)).copied().unwrap_or(b'0');
                out.push(char::from(hi));
                out.push(char::from(lo));
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn hbar_preflight_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/accounts/0.0.9001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "balance": { "balance": 5000 },
                "max_automatic_token_associations": 0
            })))
            .mount(&server)
            .await;
        let mirror = HederaMirrorClient::connect(server.uri());
        let result = preflight_transfer(&mirror, "0.0.9001", "0.0.7001", "0.0.0", "1000")
            .await
            .unwrap();
        assert!(result.ok);
    }

    #[tokio::test]
    async fn hbar_preflight_insufficient() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/accounts/0.0.9001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "balance": { "balance": 500 },
                "max_automatic_token_associations": 0
            })))
            .mount(&server)
            .await;
        let mirror = HederaMirrorClient::connect(server.uri());
        let result = preflight_transfer(&mirror, "0.0.9001", "0.0.7001", "0.0.0", "1000")
            .await
            .unwrap();
        assert!(!result.ok);
        assert_eq!(result.reason.as_deref(), Some("insufficient_balance"));
    }
}
