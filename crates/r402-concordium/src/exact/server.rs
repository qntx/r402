//! Server-side price tag generation for the Concordium exact scheme.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

use compact_str::CompactString;
use r402_protocol::money::{MoneyAmount, MoneyAmountParseError, ScaleFromMantissa};
use r402_protocol::network::{ChainId, DeployedTokenAmount};
use r402_protocol::payment::{PaymentRequirements, PriceTag, SupportedResponse, V2};
use r402_server::{
    PaymentFlowConfig, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    SchemePaymentRequiredContext,
};
use serde_json::{Map, Value};

use crate::chain::{ConcordiumAddress, ConcordiumTokenDeployment};
use crate::exact::{ConcordiumExact, ExactScheme};
use crate::networks::{asset_decimals, get_default_asset};

/// Parsed price used by [`ConcordiumExact::parse_price`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetAmount {
    /// Atomic units as a decimal string.
    pub amount: CompactString,
    /// `"CCD"` or a PLT symbol.
    pub asset: CompactString,
    /// Extra fields copied from an explicit `AssetAmount`.
    pub extra: Value,
}

/// Async money parser: `None` falls through to the next parser / USDR default.
pub type MoneyParser = Arc<
    dyn Fn(String, ChainId) -> Pin<Box<dyn Future<Output = Option<AssetAmount>> + Send>>
        + Send
        + Sync,
>;

fn concordium_exact_payment_flows() -> &'static HashMap<String, PaymentFlowConfig> {
    static FLOWS: LazyLock<HashMap<String, PaymentFlowConfig>> = LazyLock::new(|| {
        HashMap::from([(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.to_owned(),
            PaymentFlowConfig::authorization_and_upfront(),
        )])
    });
    &FLOWS
}

impl SchemeNetworkServer for ConcordiumExact {
    fn scheme(&self) -> &'static str {
        ExactScheme::VALUE
    }

    fn default_asset_transfer_method(&self) -> &'static str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        concordium_exact_payment_flows()
    }

    fn enrich_payment_required_response<'a>(
        &'a self,
        ctx: &'a SchemePaymentRequiredContext<'a>,
    ) -> impl Future<Output = Option<Vec<PaymentRequirements>>> + Send + 'a {
        let mut accepts = ctx.requirements.to_vec();
        let changed = accepts.iter_mut().fold(false, |acc, req| {
            acc | (req.network == *ctx.network && apply_concordium_fee_payer(req, ctx.supported))
        });
        std::future::ready(changed.then_some(accepts))
    }
}

impl ConcordiumExact {
    /// Registers a money parser tried before the USDR default.
    #[must_use]
    pub fn register_money_parser<F, Fut>(mut self, parser: F) -> Self
    where
        F: Fn(String, ChainId) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<AssetAmount>> + Send + 'static,
    {
        self.money_parsers.push(Arc::new(move |amount, network| {
            Box::pin(parser(amount, network))
        }));
        self
    }

    /// Decimals for a known default asset, or `None`.
    #[must_use]
    pub fn get_asset_decimals(asset: &str, network: &str) -> Option<u8> {
        asset_decimals(asset, network)
    }

    /// Parse price into [`AssetAmount`].
    ///
    /// - Object with `amount`: pass-through; `asset` is required.
    /// - Money string/number: registered parsers, then USDR via
    ///   [`get_default_asset`]. Native CCD is never a silent fallback.
    ///
    /// # Errors
    ///
    /// Missing asset on `AssetAmount`, unknown network/ticker, or invalid money.
    pub async fn parse_price(
        &self,
        price: &Value,
        network: &str,
    ) -> Result<AssetAmount, ParsePriceError> {
        if let Some(obj) = price.as_object()
            && obj.contains_key("amount")
        {
            let asset = obj
                .get("asset")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let Some(asset) = asset else {
                return Err(ParsePriceError::AssetRequired {
                    network: network.to_owned(),
                });
            };
            let amount = match obj.get("amount") {
                Some(Value::String(s)) => CompactString::from(s.as_str()),
                Some(Value::Number(n)) => CompactString::from(n.to_string()),
                _ => CompactString::from("0"),
            };
            let extra = obj
                .get("extra")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new()));
            return Ok(AssetAmount {
                amount,
                asset: CompactString::from(asset),
                extra,
            });
        }

        let (amount, symbol) = parse_money(price)?;
        let chain: ChainId = network
            .parse()
            .map_err(|_| ParsePriceError::UnknownNetwork {
                network: network.to_owned(),
            })?;
        for parser in &self.money_parsers {
            if let Some(result) = parser(amount.clone(), chain.clone()).await {
                return Ok(result);
            }
        }
        Self::default_money_conversion(&amount, network, symbol.as_deref())
    }

    fn default_money_conversion(
        amount: &str,
        network: &str,
        symbol: Option<&str>,
    ) -> Result<AssetAmount, ParsePriceError> {
        let asset_info = get_default_asset(network, symbol)?;
        let atomic = convert_to_token_amount(amount, u32::from(asset_info.decimals))?;
        Ok(AssetAmount {
            amount: CompactString::from(atomic),
            asset: CompactString::from(asset_info.asset),
            extra: Value::Object(Map::new()),
        })
    }

    /// Creates a price tag. `extra` is filled from `/supported` at enrich.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "DeployedTokenAmount is consumed for its fields"
    )]
    pub fn price_tag<A: Into<ConcordiumAddress>>(
        pay_to: A,
        asset: DeployedTokenAmount<u64, ConcordiumTokenDeployment>,
    ) -> PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.into();
        let requirements = PaymentRequirements::new(
            ExactScheme.to_string().into(),
            chain_id,
            asset.amount.to_string().into(),
            pay_to.into().to_string().into(),
            asset.token.asset.into(),
            60,
        );
        PriceTag::new(requirements)
    }
}

/// Failure parsing a price.
#[derive(Debug, thiserror::Error)]
pub enum ParsePriceError {
    /// `AssetAmount` object omitted `asset`.
    #[error("Asset must be specified for AssetAmount on network {network}")]
    AssetRequired {
        /// Network that was being priced.
        network: String,
    },
    /// Default-asset lookup failed.
    #[error(transparent)]
    DefaultAsset(#[from] crate::networks::DefaultAssetError),
    /// Human-readable money string was invalid.
    #[error(transparent)]
    Money(#[from] MoneyAmountParseError),
    /// CAIP-2 network string was malformed.
    #[error("No default asset configured for network {network}")]
    UnknownNetwork {
        /// Requested network.
        network: String,
    },
}

fn convert_to_token_amount(amount: &str, decimals: u32) -> Result<String, MoneyAmountParseError> {
    let money = MoneyAmount::parse(amount)?;
    if money.scale() > decimals {
        return Err(MoneyAmountParseError::WrongPrecision {
            money: money.scale(),
            token: decimals,
        });
    }
    let raw = u128::from_mantissa_scaled(money.mantissa(), decimals.saturating_sub(money.scale()))?;
    Ok(raw.to_string())
}

fn parse_money(price: &Value) -> Result<(String, Option<String>), MoneyAmountParseError> {
    let raw = match price {
        Value::String(s) => s.as_str(),
        Value::Number(n) => return Ok((n.to_string(), None)),
        _ => return Err(MoneyAmountParseError::InvalidFormat),
    };
    let trimmed = raw.trim();
    let (body, symbol) = split_ticker(trimmed);
    let _ = MoneyAmount::parse(body)?;
    Ok((body.to_owned(), symbol))
}

fn split_ticker(input: &str) -> (&str, Option<String>) {
    let bytes = input.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        let Some(b) = bytes.get(end - 1) else {
            break;
        };
        if b.is_ascii_alphabetic() {
            end -= 1;
        } else {
            break;
        }
    }
    if end == bytes.len() || end == 0 {
        return (input.trim(), None);
    }
    let (num, tick) = input.split_at(end);
    let tick = tick.trim();
    if tick.is_empty() {
        (num.trim(), None)
    } else {
        (num.trim(), Some(tick.to_owned()))
    }
}

fn apply_concordium_fee_payer(
    req: &mut PaymentRequirements,
    capabilities: &SupportedResponse,
) -> bool {
    if req.scheme.as_str() != ExactScheme::VALUE {
        return false;
    }
    let fee_payer = matching_kind_extra(capabilities, ExactScheme::VALUE, &req.network.to_string())
        .and_then(|extra| extra.get("feePayer").cloned());
    let mut extra = match req.extra.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            let mut map = Map::new();
            let _ = map.insert("_previous".to_owned(), other);
            map
        }
        None => Map::new(),
    };
    if let Some(fee_payer) = fee_payer {
        let _ = extra.insert("feePayer".to_owned(), fee_payer);
    }
    req.extra = Some(Value::Object(extra));
    true
}

fn matching_kind_extra<'a>(
    capabilities: &'a SupportedResponse,
    scheme: &str,
    network: &str,
) -> Option<&'a Value> {
    capabilities.kinds.iter().find_map(|kind| {
        (V2 == kind.x402_version
            && kind.scheme.as_str() == scheme
            && kind.network.as_str() == network)
            .then_some(kind.extra.as_ref())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use r402_server::PaymentFlowName;

    use super::*;
    use crate::USDR;
    use crate::chain::{CONCORDIUM_TESTNET_CAIP2, ConcordiumAddress};

    #[test]
    fn price_tag_omits_extra_until_scheme_enrich() {
        let pay_to = ConcordiumAddress::from_bytes([1u8; 32]);
        let tag = ConcordiumExact::price_tag(pay_to, USDR::concordium_testnet().amount(1_000_000));
        assert_eq!(tag.requirements.scheme, "exact");
        assert_eq!(
            tag.requirements.network.to_string(),
            CONCORDIUM_TESTNET_CAIP2
        );
        assert!(tag.requirements.extra.is_none());
        assert_eq!(tag.requirements.asset, "USDR");
    }

    #[test]
    fn payment_flows_use_default_authorization_and_upfront() {
        let scheme = ConcordiumExact::new();
        assert_eq!(scheme.scheme(), "exact");
        assert_eq!(
            scheme.default_asset_transfer_method(),
            SDK_DEFAULT_ASSET_TRANSFER_METHOD
        );
        let row = scheme
            .payment_flows()
            .get(SDK_DEFAULT_ASSET_TRANSFER_METHOD)
            .expect("default ATM");
        assert_eq!(*row, PaymentFlowConfig::authorization_and_upfront());
        assert_eq!(row.default, PaymentFlowName::Authorization);
        assert!(scheme.dynamic_extra_fields().is_empty());
    }

    #[tokio::test]
    async fn parse_price_passthrough_and_usdr_default() {
        let scheme = ConcordiumExact::new();
        let ccd = scheme
            .parse_price(
                &serde_json::json!({ "amount": "1000000", "asset": "CCD" }),
                CONCORDIUM_TESTNET_CAIP2,
            )
            .await
            .expect("ccd");
        assert_eq!(ccd.amount, "1000000");
        assert_eq!(ccd.asset, "CCD");

        let usd = scheme
            .parse_price(&serde_json::json!("$0.001"), CONCORDIUM_TESTNET_CAIP2)
            .await
            .expect("usd");
        assert_eq!(usd.amount, "1000");
        assert_eq!(usd.asset, "USDR");

        let ten = scheme
            .parse_price(&serde_json::json!("10"), CONCORDIUM_TESTNET_CAIP2)
            .await
            .expect("ten");
        assert_eq!(ten.amount, "10000000");
        assert_eq!(ten.asset, "USDR");
    }

    #[tokio::test]
    async fn parse_price_requires_asset() {
        let scheme = ConcordiumExact::new();
        let err = scheme
            .parse_price(
                &serde_json::json!({ "amount": "100" }),
                CONCORDIUM_TESTNET_CAIP2,
            )
            .await
            .expect_err("asset");
        assert!(matches!(err, ParsePriceError::AssetRequired { .. }));
    }

    #[tokio::test]
    async fn parse_price_passthrough_unknown_asset_and_extra() {
        let scheme = ConcordiumExact::new();
        let unknown = scheme
            .parse_price(
                &serde_json::json!({ "amount": "1", "asset": "UNKNOWN" }),
                CONCORDIUM_TESTNET_CAIP2,
            )
            .await
            .expect("unknown");
        assert_eq!(unknown.amount, "1");
        assert_eq!(unknown.asset, "UNKNOWN");

        let with_extra = scheme
            .parse_price(
                &serde_json::json!({
                    "amount": "1000000",
                    "asset": "CCD",
                    "extra": { "memo": "test-memo", "priority": "high" }
                }),
                CONCORDIUM_TESTNET_CAIP2,
            )
            .await
            .expect("extra");
        assert_eq!(
            with_extra.extra,
            serde_json::json!({ "memo": "test-memo", "priority": "high" })
        );

        let empty_extra = scheme
            .parse_price(
                &serde_json::json!({ "amount": "500", "asset": "EURR" }),
                CONCORDIUM_TESTNET_CAIP2,
            )
            .await
            .expect("empty extra");
        assert_eq!(empty_extra.extra, Value::Object(Map::new()));
    }

    #[test]
    fn get_asset_decimals_matches_oracle() {
        assert_eq!(
            ConcordiumExact::get_asset_decimals("USDR", CONCORDIUM_TESTNET_CAIP2),
            Some(6)
        );
        assert_eq!(
            ConcordiumExact::get_asset_decimals("CCD", CONCORDIUM_TESTNET_CAIP2),
            None
        );
        assert_eq!(
            ConcordiumExact::get_asset_decimals("EURR", CONCORDIUM_TESTNET_CAIP2),
            None
        );
    }

    #[tokio::test]
    async fn enrich_injects_fee_payer_and_preserves_extra() {
        use r402_protocol::payment::{PaymentRequired, ResourceInfo, SupportedPaymentKind, V2};
        use r402_server::SchemePaymentRequiredContext;

        let scheme = ConcordiumExact::new();
        let pay_to = ConcordiumAddress::from_bytes([1u8; 32]);
        let mut req =
            ConcordiumExact::price_tag(pay_to, USDR::concordium_testnet().amount(1_000_000))
                .requirements;
        req.extra = Some(serde_json::json!({
            "customField": "customValue",
            "anotherField": 42
        }));
        let fee_payer = ConcordiumAddress::from_bytes([2u8; 32]).to_string();
        let supported = SupportedResponse::new().with_kinds(vec![
            SupportedPaymentKind::new(V2.into(), "exact", CONCORDIUM_TESTNET_CAIP2)
                .with_extra(serde_json::json!({ "feePayer": fee_payer })),
        ]);
        let resource = ResourceInfo::new("https://api.example.com/premium-data");
        let payment_required = PaymentRequired::new(resource.clone());
        let ctx = SchemePaymentRequiredContext::new(
            std::slice::from_ref(&req),
            &resource,
            &payment_required,
            &supported,
            &req.network,
        );
        let enriched = scheme
            .enrich_payment_required_response(&ctx)
            .await
            .expect("changed");
        let extra = enriched[0].extra.as_ref().expect("extra");
        assert_eq!(
            extra.get("feePayer").and_then(Value::as_str),
            Some(fee_payer.as_str())
        );
        assert_eq!(
            extra.get("customField").and_then(Value::as_str),
            Some("customValue")
        );
        assert_eq!(extra.get("anotherField").and_then(Value::as_u64), Some(42));
        assert!(extra.get("decimals").is_none());
    }

    #[tokio::test]
    async fn enrich_omits_fee_payer_when_supported_has_none() {
        use r402_protocol::payment::{PaymentRequired, ResourceInfo, SupportedPaymentKind, V2};
        use r402_server::SchemePaymentRequiredContext;

        let scheme = ConcordiumExact::new();
        let pay_to = ConcordiumAddress::from_bytes([1u8; 32]);
        let req = ConcordiumExact::price_tag(pay_to, USDR::concordium_testnet().amount(1_000_000))
            .requirements;
        let supported = SupportedResponse::new().with_kinds(vec![SupportedPaymentKind::new(
            V2.into(),
            "exact",
            CONCORDIUM_TESTNET_CAIP2,
        )]);
        let resource = ResourceInfo::new("https://api.example.com/premium-data");
        let payment_required = PaymentRequired::new(resource.clone());
        let ctx = SchemePaymentRequiredContext::new(
            std::slice::from_ref(&req),
            &resource,
            &payment_required,
            &supported,
            &req.network,
        );
        let enriched = scheme
            .enrich_payment_required_response(&ctx)
            .await
            .expect("changed");
        let extra = enriched[0].extra.as_ref().expect("extra object");
        assert!(extra.get("feePayer").is_none());
    }

    #[tokio::test]
    async fn money_parsers_run_in_order() {
        let scheme = ConcordiumExact::new()
            .register_money_parser(|_amount, _net| async { None })
            .register_money_parser(|amount, _net| async move {
                Some(AssetAmount {
                    amount: CompactString::from(convert_to_token_amount(&amount, 6).expect("amt")),
                    asset: CompactString::from("EURR"),
                    extra: Value::Object(Map::new()),
                })
            });
        let result = scheme
            .parse_price(&serde_json::json!("10"), CONCORDIUM_TESTNET_CAIP2)
            .await
            .expect("parsed");
        assert_eq!(result.asset, "EURR");
        assert_eq!(result.amount, "10000000");
    }
}
