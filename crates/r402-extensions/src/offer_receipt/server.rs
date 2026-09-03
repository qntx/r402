//! Resource-server declare, issuer, and 402/200 enrich.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use r402_protocol::extension::{AdvertiseContext, Extension, SettleContext};
use r402_protocol::payment::{ExtensionEntry, PaymentRequirements, SettleResponse};
use serde_json::{Value, json};

use super::create::{
    create_offer_eip712, create_offer_jws, create_receipt_eip712, create_receipt_jws,
};
use super::eip712::Eip712DigestSigner;
use super::jws::JwsSigner;
use super::{
    OFFER_RECEIPT, OfferInput, OfferReceiptError, ReceiptInput, SignatureFormat, SignedOffer,
    SignedReceipt,
};

/// Boxed future used by [`DynOfferReceiptIssuer`].
pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// High-level issuer that signs offers and receipts.
pub trait OfferReceiptIssuer: Send + Sync {
    /// Key identifier DID.
    fn kid(&self) -> &str;
    /// Signature format.
    fn format(&self) -> SignatureFormat;
    /// Sign an offer for `resource_url` / `input`.
    fn issue_offer(
        &self,
        resource_url: &str,
        input: OfferInput,
    ) -> impl Future<Output = Result<SignedOffer, OfferReceiptError>> + Send;
    /// Sign a receipt.
    fn issue_receipt(
        &self,
        input: ReceiptInput,
    ) -> impl Future<Output = Result<SignedReceipt, OfferReceiptError>> + Send;
}

/// Object-safe issuer.
pub trait DynOfferReceiptIssuer: Send + Sync {
    /// See [`OfferReceiptIssuer::kid`].
    fn kid(&self) -> &str;
    /// See [`OfferReceiptIssuer::format`].
    fn format(&self) -> SignatureFormat;
    /// See [`OfferReceiptIssuer::issue_offer`].
    fn issue_offer<'a>(
        &'a self,
        resource_url: &'a str,
        input: OfferInput,
    ) -> BoxFuture<'a, Result<SignedOffer, OfferReceiptError>>;
    /// See [`OfferReceiptIssuer::issue_receipt`].
    fn issue_receipt(
        &self,
        input: ReceiptInput,
    ) -> BoxFuture<'_, Result<SignedReceipt, OfferReceiptError>>;
}

impl<T: OfferReceiptIssuer + ?Sized> DynOfferReceiptIssuer for T {
    fn kid(&self) -> &str {
        <Self as OfferReceiptIssuer>::kid(self)
    }

    fn format(&self) -> SignatureFormat {
        <Self as OfferReceiptIssuer>::format(self)
    }

    fn issue_offer<'a>(
        &'a self,
        resource_url: &'a str,
        input: OfferInput,
    ) -> BoxFuture<'a, Result<SignedOffer, OfferReceiptError>> {
        Box::pin(<Self as OfferReceiptIssuer>::issue_offer(
            self,
            resource_url,
            input,
        ))
    }

    fn issue_receipt(
        &self,
        input: ReceiptInput,
    ) -> BoxFuture<'_, Result<SignedReceipt, OfferReceiptError>> {
        Box::pin(<Self as OfferReceiptIssuer>::issue_receipt(self, input))
    }
}

/// JWS issuer.
#[derive(Clone)]
pub struct JwsOfferReceiptIssuer<S> {
    signer: S,
}

impl<S> std::fmt::Debug for JwsOfferReceiptIssuer<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwsOfferReceiptIssuer")
            .finish_non_exhaustive()
    }
}

impl<S: JwsSigner> JwsOfferReceiptIssuer<S> {
    /// Wrap a JWS signer.
    #[must_use]
    pub const fn new(signer: S) -> Self {
        Self { signer }
    }
}

impl<S: JwsSigner> OfferReceiptIssuer for JwsOfferReceiptIssuer<S> {
    fn kid(&self) -> &str {
        self.signer.kid()
    }

    fn format(&self) -> SignatureFormat {
        SignatureFormat::Jws
    }

    fn issue_offer(
        &self,
        resource_url: &str,
        input: OfferInput,
    ) -> impl Future<Output = Result<SignedOffer, OfferReceiptError>> + Send {
        std::future::ready(create_offer_jws(resource_url, &input, &self.signer))
    }

    fn issue_receipt(
        &self,
        input: ReceiptInput,
    ) -> impl Future<Output = Result<SignedReceipt, OfferReceiptError>> + Send {
        std::future::ready(create_receipt_jws(&input, &self.signer))
    }
}

/// EIP-712 issuer.
#[derive(Clone)]
pub struct Eip712OfferReceiptIssuer<S> {
    kid: String,
    signer: S,
}

impl<S> std::fmt::Debug for Eip712OfferReceiptIssuer<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Eip712OfferReceiptIssuer")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl<S: Eip712DigestSigner> Eip712OfferReceiptIssuer<S> {
    /// Wrap an EIP-712 digest signer.
    #[must_use]
    pub fn new(kid: impl Into<String>, signer: S) -> Self {
        Self {
            kid: kid.into(),
            signer,
        }
    }
}

impl<S: Eip712DigestSigner> OfferReceiptIssuer for Eip712OfferReceiptIssuer<S> {
    fn kid(&self) -> &str {
        &self.kid
    }

    fn format(&self) -> SignatureFormat {
        SignatureFormat::Eip712
    }

    fn issue_offer(
        &self,
        resource_url: &str,
        input: OfferInput,
    ) -> impl Future<Output = Result<SignedOffer, OfferReceiptError>> + Send {
        std::future::ready(create_offer_eip712(resource_url, &input, &self.signer))
    }

    fn issue_receipt(
        &self,
        input: ReceiptInput,
    ) -> impl Future<Output = Result<SignedReceipt, OfferReceiptError>> + Send {
        std::future::ready(create_receipt_eip712(&input, &self.signer))
    }
}

/// `createJWSOfferReceiptIssuer` analogue.
#[must_use]
pub const fn create_jws_offer_receipt_issuer<S: JwsSigner>(signer: S) -> JwsOfferReceiptIssuer<S> {
    JwsOfferReceiptIssuer::new(signer)
}

/// `createEIP712OfferReceiptIssuer` analogue.
#[must_use]
pub fn create_eip712_offer_receipt_issuer<S: Eip712DigestSigner>(
    kid: impl Into<String>,
    signer: S,
) -> Eip712OfferReceiptIssuer<S> {
    Eip712OfferReceiptIssuer::new(kid, signer)
}

/// Route declaration `{ includeTxHash, offerValiditySeconds }`.
#[must_use]
pub fn declare_offer_receipt_extension(
    include_tx_hash: Option<bool>,
    offer_validity_seconds: Option<u64>,
) -> ExtensionEntry {
    let mut obj = serde_json::Map::new();
    if let Some(flag) = include_tx_hash {
        let _ = obj.insert("includeTxHash".into(), Value::Bool(flag));
    }
    if let Some(secs) = offer_validity_seconds {
        let _ = obj.insert("offerValiditySeconds".into(), json!(secs));
    }
    ExtensionEntry::raw(Value::Object(obj))
}

fn offer_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "offers": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "format": { "type": "string" },
                        "acceptIndex": { "type": "integer" },
                        "payload": { "type": "object" },
                        "signature": { "type": "string" },
                    },
                    "required": ["format", "signature"],
                },
            },
        },
        "required": ["offers"],
    })
}

fn receipt_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "receipt": {
                "type": "object",
                "properties": {
                    "format": { "type": "string" },
                    "payload": { "type": "object" },
                    "signature": { "type": "string" },
                },
                "required": ["format", "signature"],
            },
        },
        "required": ["receipt"],
    })
}

/// Resource-server offer-receipt extension (signed offers on 402, receipts on 200).
#[derive(Clone)]
pub struct OfferReceiptExtension {
    issuer: Arc<dyn DynOfferReceiptIssuer>,
    include_tx_hash: bool,
    offer_validity_seconds: Option<u64>,
}

impl std::fmt::Debug for OfferReceiptExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfferReceiptExtension")
            .field("kid", &self.issuer.kid())
            .field("format", &self.issuer.format())
            .field("include_tx_hash", &self.include_tx_hash)
            .finish_non_exhaustive()
    }
}

impl OfferReceiptExtension {
    /// Bind an issuer. Receipts omit `transaction` by default.
    #[must_use]
    pub fn new(issuer: Arc<dyn DynOfferReceiptIssuer>) -> Self {
        Self {
            issuer,
            include_tx_hash: false,
            offer_validity_seconds: None,
        }
    }

    /// Include the settlement transaction hash on receipts.
    #[must_use]
    pub const fn with_include_tx_hash(mut self, include: bool) -> Self {
        self.include_tx_hash = include;
        self
    }

    /// Override offer validity (seconds). Default 300 / accept `maxTimeoutSeconds`.
    #[must_use]
    pub const fn with_offer_validity_seconds(mut self, seconds: u64) -> Self {
        self.offer_validity_seconds = Some(seconds);
        self
    }

    async fn sign_offers(
        &self,
        resource_url: &str,
        ctx: &AdvertiseContext<'_>,
    ) -> Vec<SignedOffer> {
        let (_, declared_validity) = declaration_config(ctx.existing);
        let validity = declared_validity.or(self.offer_validity_seconds);
        let mut offers = Vec::new();
        for (i, requirement) in ctx.accepts.iter().enumerate() {
            let index = u32::try_from(i).unwrap_or(u32::MAX);
            let input = requirements_to_offer_input(requirement, index, validity);
            if let Ok(signed) = self.issuer.issue_offer(resource_url, input).await {
                offers.push(signed);
            }
        }
        offers
    }
}

fn declaration_config(existing: Option<&ExtensionEntry>) -> (Option<bool>, Option<u64>) {
    let Some(existing) = existing else {
        return (None, None);
    };
    let value = existing.to_value();
    let obj = value.get("info").unwrap_or(&value);
    let include = obj.get("includeTxHash").and_then(Value::as_bool);
    let validity = obj.get("offerValiditySeconds").and_then(Value::as_u64);
    (include, validity)
}

fn requirements_to_offer_input(
    requirements: &PaymentRequirements,
    accept_index: u32,
    offer_validity_seconds: Option<u64>,
) -> OfferInput {
    let validity = offer_validity_seconds.or(Some(requirements.max_timeout_seconds));
    OfferInput {
        accept_index,
        scheme: requirements.scheme.to_string(),
        network: requirements.network.to_string(),
        asset: requirements.asset.to_string(),
        pay_to: requirements.pay_to.to_string(),
        amount: requirements.amount.to_string(),
        offer_validity_seconds: validity,
    }
}

impl Extension for OfferReceiptExtension {
    fn id(&self) -> &'static str {
        OFFER_RECEIPT
    }

    fn dynamic_info_fields(&self) -> &'static [&'static str] {
        &["offers"]
    }

    async fn enrich_payment_required(&self, ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        let resource_url = ctx.resource.map(|r| r.url.as_str())?;
        let offers = self.sign_offers(resource_url, ctx).await;
        if offers.is_empty() {
            return None;
        }
        serde_json::to_value(&offers)
            .ok()
            .map(|offers| ExtensionEntry::with_schema(json!({ "offers": offers }), offer_schema()))
    }

    async fn on_settle(&self, ctx: &SettleContext<'_>) -> Option<ExtensionEntry> {
        let SettleResponse::Success {
            payer,
            network,
            transaction,
            ..
        } = ctx.response
        else {
            return None;
        };
        let payer = payer.as_deref()?;
        let resource_url = ctx
            .resource_url
            .or_else(|| ctx.payload.resource.as_ref().map(|r| r.url.as_str()))?;
        let (declared_include, _) =
            declaration_config(ctx.advertised.and_then(|ext| ext.get(OFFER_RECEIPT)));
        let include_tx = declared_include.unwrap_or(self.include_tx_hash);
        let tx = if include_tx && !transaction.is_empty() {
            Some(transaction.to_string())
        } else {
            None
        };
        let input = ReceiptInput {
            resource_url: resource_url.to_owned(),
            payer: payer.to_owned(),
            network: network.to_string(),
            transaction: tx,
        };
        let signed = self.issuer.issue_receipt(input).await.ok()?;
        serde_json::to_value(&signed).ok().map(|receipt| {
            ExtensionEntry::with_schema(json!({ "receipt": receipt }), receipt_schema())
        })
    }
}
