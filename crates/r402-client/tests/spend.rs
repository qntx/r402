#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::missing_const_for_fn,
    clippy::shadow_unrelated,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! `SpendControls`: default `$1` cap, allow-list, and per-asset atomic caps.

use std::future::Future;
use std::pin::Pin;

use r402_client::{
    AllowedAssets, DefaultAssetInfo, MaxAmountPerPayment, PaymentCandidate, PaymentCandidateSigner,
    PaymentClient, SchemeClient, SpendControlAsset, SpendControls,
};
use r402_protocol::{
    ChainId, ClientError, MoneyAmount, PaymentRequired, PaymentRequirements, ResourceInfo, SchemeId,
};

const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
const NETWORK: &str = "eip155:8453";

struct StubSigner(String);
impl PaymentCandidateSigner for StubSigner {
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
        let payload = self.0.clone();
        Box::pin(async move { Ok(payload) })
    }
}

struct StubScheme {
    default_asset: Option<DefaultAssetInfo>,
}

impl StubScheme {
    fn bare() -> Self {
        Self {
            default_asset: None,
        }
    }

    fn with_default(info: DefaultAssetInfo) -> Self {
        Self {
            default_asset: Some(info),
        }
    }
}

impl SchemeId for StubScheme {
    fn namespace(&self) -> &'static str {
        "eip155"
    }
    fn scheme(&self) -> &'static str {
        "exact"
    }
}
impl SchemeClient for StubScheme {
    fn accept(&self, required: &PaymentRequired) -> Vec<PaymentCandidate> {
        required
            .accepts
            .iter()
            .map(|r| PaymentCandidate {
                chain_id: r.network.clone(),
                asset: r.asset.clone(),
                amount: r.amount.clone(),
                scheme: r.scheme.clone(),
                pay_to: r.pay_to.clone(),
                requirements: r.clone(),
                signer: Box::new(StubSigner("c2lnbmVk".into())),
            })
            .collect()
    }

    fn find_default_asset(&self, asset: &str, _network: &ChainId) -> Option<DefaultAssetInfo> {
        self.default_asset
            .as_ref()
            .filter(|info| info.asset.eq_ignore_ascii_case(asset))
            .cloned()
    }
}

fn usdc_info() -> DefaultAssetInfo {
    DefaultAssetInfo::new(USDC, 6, "USDC")
}

fn required_with(
    network: &str,
    asset: &str,
    amount: &str,
    extra: Option<serde_json::Value>,
) -> PaymentRequired {
    required_accepts(vec![accept(network, asset, amount, extra)])
}

fn required_accepts(accepts: Vec<PaymentRequirements>) -> PaymentRequired {
    PaymentRequired::new(ResourceInfo::new("https://example.com")).with_accepts(accepts)
}

fn accept(
    network: &str,
    asset: &str,
    amount: &str,
    extra: Option<serde_json::Value>,
) -> PaymentRequirements {
    let mut req = PaymentRequirements::new(
        "exact".into(),
        network.parse::<ChainId>().unwrap(),
        amount.into(),
        "0xb".into(),
        asset.into(),
        60,
    );
    if let Some(extra) = extra {
        req = req.with_extra(extra);
    }
    req
}

fn client_with_default() -> PaymentClient {
    PaymentClient::new().register(StubScheme::with_default(usdc_info()))
}

#[tokio::test]
async fn spend_controls_allow_at_default_usd_cap() {
    let created = client_with_default()
        .create_payment(&required_with(NETWORK, USDC, "1000000", None))
        .await
        .unwrap();
    assert_eq!(
        created.signed_payload, "c2lnbmVk",
        "$1 of 6-decimal USDC must pass the default cap"
    );
}

#[tokio::test]
async fn spend_controls_reject_above_default_usd_cap() {
    let err = client_with_default()
        .create_payment(&required_with(NETWORK, USDC, "1000001", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("maxAmountPerPayment")),
        "amount above $1 must fail, got {err:?}"
    );
}

#[tokio::test]
async fn spend_controls_pick_affordable_accept() {
    let required = required_accepts(vec![
        accept(NETWORK, USDC, "50000000", None),
        accept(NETWORK, USDC, "500000", None),
    ]);
    let created = client_with_default()
        .create_payment(&required)
        .await
        .unwrap();
    assert_eq!(
        created.signed_payload, "c2lnbmVk",
        "affordable accept must be signed"
    );
    let candidates = client_with_default().candidates(&required);
    let selected = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .select_candidate(&candidates)
        .unwrap();
    assert_eq!(
        selected.amount, "500000",
        "selector must keep the accept under the $1 cap"
    );
}

#[tokio::test]
async fn spend_controls_reject_unrecognized_assets_and_missing_lookup() {
    let err = client_with_default()
        .create_payment(&required_with(NETWORK, "0xCustomUnknownToken", "1", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("allowedAssets")),
        "unknown asset must hit allow-list, got {err:?}"
    );
}

#[tokio::test]
async fn spend_controls_reject_scheme_without_find_default_asset() {
    let missing_lookup = PaymentClient::new()
        .register(StubScheme::bare())
        .create_payment(&required_with(NETWORK, USDC, "1", None))
        .await
        .unwrap_err();
    assert!(
        matches!(missing_lookup, ClientError::SpendControls(ref msg) if msg.contains("allowedAssets")),
        "missing default-asset lookup must hit allow-list, got {missing_lookup:?}"
    );
}

#[tokio::test]
async fn disable_spend_controls_allows_any_asset_and_amount() {
    let client = PaymentClient::new()
        .disable_spend_controls()
        .register(StubScheme::with_default(usdc_info()));
    client
        .create_payment(&required_with(
            NETWORK,
            "0xCustomUnknownToken",
            "999999999999",
            None,
        ))
        .await
        .unwrap();
    client
        .create_payment(&required_with(NETWORK, USDC, "5000000", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn allowed_assets_any_still_usd_caps_defaults() {
    let client = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            allowed_assets: AllowedAssets::Any,
            ..SpendControls::default()
        });
    client
        .create_payment(&required_with(
            NETWORK,
            "0xCustomUnknownToken",
            "999999999999",
            None,
        ))
        .await
        .unwrap();
    let err = client
        .create_payment(&required_with(NETWORK, USDC, "1000001", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("maxAmountPerPayment")),
        "Any allow-list must still USD-cap defaults, got {err:?}"
    );
}

#[tokio::test]
async fn spend_controls_scale_18_decimal_default_asset() {
    let m_usd = DefaultAssetInfo::new("0x118917a40FAF1CD7a13dB0Ef56C86De7973Ac503", 18, "mUSD");
    let client = PaymentClient::new().register(StubScheme::with_default(m_usd));
    let mezo = "eip155:31611";
    let asset = "0x118917a40FAF1CD7a13dB0Ef56C86De7973Ac503";
    let err = client
        .create_payment(&required_with(mezo, asset, "1000000000000000001", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("maxAmountPerPayment")),
        "18-decimal $1 + 1 wei must fail, got {err:?}"
    );
    client
        .create_payment(&required_with(mezo, asset, "1000000000000000000", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn spend_controls_honour_disabled_and_custom_usd_cap() {
    let client = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            max_amount_per_payment: MaxAmountPerPayment::Disabled,
            ..SpendControls::default()
        });
    client
        .create_payment(&required_with(NETWORK, USDC, "5000000", None))
        .await
        .unwrap();

    let client5 = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            max_amount_per_payment: MaxAmountPerPayment::Usd(MoneyAmount::from(5)),
            ..SpendControls::default()
        });
    client5
        .create_payment(&required_with(NETWORK, USDC, "5000000", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn spend_controls_opt_in_asset_atomic_cap_and_uncapped() {
    let custom = "0xCustomToken";
    let capped = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            allowed_assets: AllowedAssets::List(vec![SpendControlAsset {
                network: NETWORK.parse().unwrap(),
                asset: custom.into(),
                max_amount_per_payment: Some("10000".into()),
            }]),
            ..SpendControls::default()
        });
    let err = capped
        .create_payment(&required_with(NETWORK, custom, "10001", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("allowedAssets maxAmountPerPayment")),
        "per-asset cap must reject over-cap amount, got {err:?}"
    );

    let uncapped = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            allowed_assets: AllowedAssets::List(vec![SpendControlAsset {
                network: "eip155:*".parse().unwrap(),
                asset: custom.to_ascii_lowercase().into(),
                max_amount_per_payment: None,
            }]),
            ..SpendControls::default()
        });
    uncapped
        .create_payment(&required_with(NETWORK, custom, "999999999999", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn spend_controls_drop_non_integer_on_per_asset_cap() {
    let custom = "0xCustomToken";
    let client = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            allowed_assets: AllowedAssets::List(vec![SpendControlAsset {
                network: NETWORK.parse().unwrap(),
                asset: custom.into(),
                max_amount_per_payment: Some("10000".into()),
            }]),
            ..SpendControls::default()
        });
    let err = client
        .create_payment(&required_with(NETWORK, custom, "1.5", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("allowedAssets maxAmountPerPayment")),
        "non-integer amount under an atomic cap must fail, got {err:?}"
    );

    let mixed = required_accepts(vec![
        accept(NETWORK, custom, "1.5", None),
        accept(NETWORK, custom, "100", None),
    ]);
    let candidates = client.candidates(&mixed);
    let selected = client.select_candidate(&candidates).unwrap();
    assert_eq!(
        selected.amount, "100",
        "integer accept under the cap must remain"
    );
}

#[tokio::test]
async fn spend_controls_reject_non_integer_per_asset_cap_config() {
    let custom = "0xCustomToken";
    for cap in ["$1", "1.5"] {
        let client = PaymentClient::new()
            .register(StubScheme::with_default(usdc_info()))
            .with_spend_controls(SpendControls {
                allowed_assets: AllowedAssets::List(vec![SpendControlAsset {
                    network: NETWORK.parse().unwrap(),
                    asset: custom.into(),
                    max_amount_per_payment: Some(cap.into()),
                }]),
                ..SpendControls::default()
            });
        let err = client
            .create_payment(&required_with(NETWORK, custom, "100", None))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientError::SpendControls(ref msg) if msg.contains("must be an integer atomic amount")),
            "cap {cap:?} must be rejected as non-atomic, got {err:?}"
        );
    }
}

#[tokio::test]
async fn spend_controls_override_usd_cap_by_id_or_symbol() {
    let by_id = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            allowed_assets: AllowedAssets::List(vec![SpendControlAsset {
                network: NETWORK.parse().unwrap(),
                asset: USDC.into(),
                max_amount_per_payment: Some("500000".into()),
            }]),
            ..SpendControls::default()
        });
    let over_id = by_id
        .create_payment(&required_with(NETWORK, USDC, "600000", None))
        .await
        .unwrap_err();
    assert!(
        matches!(over_id, ClientError::SpendControls(ref msg) if msg.contains("allowedAssets maxAmountPerPayment")),
        "id-listed cap must reject 600000, got {over_id:?}"
    );
    by_id
        .create_payment(&required_with(NETWORK, USDC, "400000", None))
        .await
        .unwrap();
}

#[tokio::test]
async fn spend_controls_override_usd_cap_by_symbol() {
    let pyusd = DefaultAssetInfo::new("0xPayPalUsdAsset000000000000000000000001", 6, "PYUSD");
    let by_symbol = PaymentClient::new()
        .register(StubScheme::with_default(pyusd.clone()))
        .with_spend_controls(SpendControls {
            allowed_assets: AllowedAssets::List(vec![SpendControlAsset {
                network: NETWORK.parse().unwrap(),
                asset: "pyusd".into(),
                max_amount_per_payment: Some("500000".into()),
            }]),
            ..SpendControls::default()
        });
    let over_symbol = by_symbol
        .create_payment(&required_with(
            NETWORK,
            pyusd.asset.as_str(),
            "600000",
            None,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(over_symbol, ClientError::SpendControls(ref msg) if msg.contains("allowedAssets maxAmountPerPayment")),
        "symbol-listed cap must reject 600000, got {over_symbol:?}"
    );
    by_symbol
        .create_payment(&required_with(
            NETWORK,
            pyusd.asset.as_str(),
            "400000",
            None,
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn listed_default_without_per_entry_cap_keeps_usd_cap() {
    let client = PaymentClient::new()
        .register(StubScheme::with_default(usdc_info()))
        .with_spend_controls(SpendControls {
            allowed_assets: AllowedAssets::List(vec![SpendControlAsset {
                network: NETWORK.parse().unwrap(),
                asset: "USDC".into(),
                max_amount_per_payment: None,
            }]),
            ..SpendControls::default()
        });
    let err = client
        .create_payment(&required_with(NETWORK, USDC, "1000001", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("maxAmountPerPayment")),
        "listed default without per-entry cap must keep $1, got {err:?}"
    );
}

#[tokio::test]
async fn spend_controls_drop_non_integer_usd_capped_default() {
    let err = client_with_default()
        .create_payment(&required_with(NETWORK, USDC, "0.01", None))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::SpendControls(ref msg) if msg.contains("maxAmountPerPayment")),
        "non-integer default-asset amount must fail USD cap, got {err:?}"
    );
}

#[test]
fn default_max_amount_is_one_dollar() {
    assert_eq!(
        MaxAmountPerPayment::default(),
        MaxAmountPerPayment::Usd(MoneyAmount::from(1)),
        "default spend cap is $1"
    );
}
