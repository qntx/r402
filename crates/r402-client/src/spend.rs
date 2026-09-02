//! Client spend controls. Default-on: default assets only, capped at `$1`.

use compact_str::CompactString;
use r402_protocol::{ChainIdPattern, ClientError, MoneyAmount};

use crate::candidate::{DefaultAssetInfo, PaymentCandidate};
use crate::register::PaymentClient;
use crate::select::PaymentSelector;

/// Per-payment USD cap on assets `find_default_asset` recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxAmountPerPayment {
    /// Cap in USD (`MoneyAmount` of `1` is the default `$1`).
    Usd(MoneyAmount),
    /// No USD cap (`maxAmountPerPayment: false`).
    Disabled,
}

impl Default for MaxAmountPerPayment {
    fn default() -> Self {
        Self::Usd(MoneyAmount::from(1))
    }
}

/// Opt-in non-default assets for [`SpendControls`].
#[derive(Debug, Clone, Default)]
pub enum AllowedAssets {
    /// Default assets only (omit `allowedAssets`).
    #[default]
    DefaultOnly,
    /// Allow any asset; USD cap still applies to defaults.
    Any,
    /// Defaults plus listed entries; optional integer atomic cap per entry.
    List(Vec<SpendControlAsset>),
}

/// Opt-in asset for [`AllowedAssets::List`].
#[derive(Debug, Clone)]
pub struct SpendControlAsset {
    /// CAIP-2 network or pattern (`eip155:8453`, `eip155:*`).
    pub network: ChainIdPattern,
    /// On-chain asset id, or a default-asset symbol (e.g. `"PYUSD"`).
    pub asset: CompactString,
    /// Optional integer atomic per-payment cap. `None` means uncapped.
    pub max_amount_per_payment: Option<CompactString>,
}

/// Client spend controls enforced before policies.
#[derive(Debug, Clone, Default)]
pub struct SpendControls {
    /// Per-payment USD cap on assets `find_default_asset` recognizes.
    pub max_amount_per_payment: MaxAmountPerPayment,
    /// Opt-in non-default assets.
    pub allowed_assets: AllowedAssets,
}

impl<S: PaymentSelector> PaymentClient<S> {
    pub(crate) fn apply_spend_controls<'a>(
        &self,
        requirements: Vec<&'a PaymentCandidate>,
    ) -> Result<Vec<&'a PaymentCandidate>, ClientError> {
        let Some(controls) = self.spend_controls.as_ref() else {
            return Ok(requirements);
        };
        let allowed = apply_allowlist(self, controls, &requirements);
        if allowed.is_empty() {
            return Err(ClientError::SpendControls(
                "all payment requirements were rejected by spendControls: only default assets \
                 or entries in spendControls.allowedAssets are allowed. Add an allowedAssets \
                 entry for non-default tokens, set allowedAssets: true, or set spendControls: false."
                    .into(),
            ));
        }
        apply_amount_caps(self, controls, &allowed)
    }

    fn default_asset_for(&self, candidate: &PaymentCandidate) -> Option<DefaultAssetInfo> {
        self.schemes.iter().find_map(|scheme| {
            if scheme.scheme() != candidate.scheme.as_str() {
                return None;
            }
            if scheme.namespace() != candidate.chain_id.namespace() {
                return None;
            }
            scheme.find_default_asset(candidate.asset.as_str(), &candidate.chain_id)
        })
    }
}

fn is_atomic_amount(amount: &str) -> bool {
    !amount.is_empty() && amount.bytes().all(|b| b.is_ascii_digit())
}

fn parse_atomic(amount: &str) -> Option<u128> {
    if !is_atomic_amount(amount) {
        return None;
    }
    amount.parse().ok()
}

fn amount_at_most(amount: &str, cap: u128) -> Option<bool> {
    match parse_atomic(amount) {
        Some(value) => Some(value <= cap),
        None if is_atomic_amount(amount) => Some(false),
        None => None,
    }
}

fn matches_asset_entry(
    entry: &SpendControlAsset,
    candidate: &PaymentCandidate,
    default_asset: Option<&DefaultAssetInfo>,
) -> bool {
    if !entry.network.matches(&candidate.chain_id) {
        return false;
    }
    if entry.asset.eq_ignore_ascii_case(candidate.asset.as_str()) {
        return true;
    }
    default_asset.is_some_and(|info| entry.asset.eq_ignore_ascii_case(info.symbol.as_str()))
}

fn find_asset_entry<'a>(
    entries: &'a [SpendControlAsset],
    candidate: &PaymentCandidate,
    default_asset: Option<&DefaultAssetInfo>,
) -> Option<&'a SpendControlAsset> {
    entries
        .iter()
        .find(|entry| matches_asset_entry(entry, candidate, default_asset))
}

const fn listed_assets(controls: &SpendControls) -> Option<&[SpendControlAsset]> {
    match &controls.allowed_assets {
        AllowedAssets::Any => None,
        AllowedAssets::DefaultOnly => Some(&[]),
        AllowedAssets::List(entries) => Some(entries.as_slice()),
    }
}

fn apply_allowlist<'a>(
    client: &PaymentClient<impl PaymentSelector>,
    controls: &SpendControls,
    requirements: &[&'a PaymentCandidate],
) -> Vec<&'a PaymentCandidate> {
    if matches!(controls.allowed_assets, AllowedAssets::Any) {
        return requirements.to_vec();
    }
    let entries = listed_assets(controls).unwrap_or(&[]);
    requirements
        .iter()
        .copied()
        .filter(|candidate| {
            let default_asset = client.default_asset_for(candidate);
            default_asset.is_some()
                || find_asset_entry(entries, candidate, default_asset.as_ref()).is_some()
        })
        .collect()
}

struct AmountCapRejects {
    by_asset_cap: bool,
    usd_symbol: Option<CompactString>,
}

fn apply_amount_caps<'a>(
    client: &PaymentClient<impl PaymentSelector>,
    controls: &SpendControls,
    requirements: &[&'a PaymentCandidate],
) -> Result<Vec<&'a PaymentCandidate>, ClientError> {
    let entries = listed_assets(controls).unwrap_or(&[]);
    let mut rejects = AmountCapRejects {
        by_asset_cap: false,
        usd_symbol: None,
    };
    let mut kept = Vec::new();
    for candidate in requirements {
        let default_asset = client.default_asset_for(candidate);
        let asset_entry = find_asset_entry(entries, candidate, default_asset.as_ref());
        match candidate_cap_decision(candidate, controls, asset_entry, default_asset.as_ref())? {
            CapDecision::Keep => kept.push(*candidate),
            CapDecision::RejectAssetCap => rejects.by_asset_cap = true,
            CapDecision::RejectUsd { symbol } => rejects.usd_symbol = Some(symbol),
        }
    }
    if kept.is_empty() {
        return Err(amount_cap_error(
            client,
            controls,
            requirements,
            entries,
            &rejects,
        ));
    }
    Ok(kept)
}

enum CapDecision {
    Keep,
    RejectAssetCap,
    RejectUsd { symbol: CompactString },
}

fn candidate_cap_decision(
    candidate: &PaymentCandidate,
    controls: &SpendControls,
    asset_entry: Option<&SpendControlAsset>,
    default_asset: Option<&DefaultAssetInfo>,
) -> Result<CapDecision, ClientError> {
    if let Some(cap) = asset_entry.and_then(|entry| entry.max_amount_per_payment.as_deref()) {
        if !is_atomic_amount(cap) {
            return Err(ClientError::SpendControls(format!(
                "spendControls.allowedAssets[].maxAmountPerPayment must be an integer atomic amount, not a dollar value; got {cap:?}"
            )));
        }
        let cap_n = parse_atomic(cap).ok_or_else(|| {
            ClientError::SpendControls(format!(
                "spendControls.allowedAssets[].maxAmountPerPayment must be an integer atomic amount, not a dollar value; got {cap:?}"
            ))
        })?;
        return Ok(match amount_at_most(candidate.amount.as_str(), cap_n) {
            Some(true) => CapDecision::Keep,
            _ => CapDecision::RejectAssetCap,
        });
    }

    let Some(default_asset) = default_asset else {
        return Ok(CapDecision::Keep);
    };
    let MaxAmountPerPayment::Usd(usd) = controls.max_amount_per_payment else {
        return Ok(CapDecision::Keep);
    };
    let decimals = u8::try_from(default_asset.decimals).map_err(|_| {
        ClientError::SpendControls(format!(
            "default asset {} decimals {} exceed u8",
            default_asset.symbol, default_asset.decimals
        ))
    })?;
    let max_atomic: u128 = usd.to_token_amount(decimals).map_err(|err| {
        ClientError::SpendControls(format!(
            "spendControls.maxAmountPerPayment cannot convert {usd} at {} decimals: {err}",
            default_asset.decimals
        ))
    })?;
    Ok(
        match amount_at_most(candidate.amount.as_str(), max_atomic) {
            Some(true) => CapDecision::Keep,
            _ => CapDecision::RejectUsd {
                symbol: default_asset.symbol.clone(),
            },
        },
    )
}

fn amount_cap_error(
    client: &PaymentClient<impl PaymentSelector>,
    controls: &SpendControls,
    before_caps: &[&PaymentCandidate],
    entries: &[SpendControlAsset],
    rejects: &AmountCapRejects,
) -> ClientError {
    let all_asset_capped = rejects.by_asset_cap
        && before_caps.iter().all(|candidate| {
            let default_asset = client.default_asset_for(candidate);
            find_asset_entry(entries, candidate, default_asset.as_ref())
                .and_then(|entry| entry.max_amount_per_payment.as_ref())
                .is_some()
        });
    if all_asset_capped {
        return ClientError::SpendControls(
            "all payment requirements were rejected by spendControls.allowedAssets maxAmountPerPayment. \
             Raise the per-asset cap, or omit maxAmountPerPayment to allow uncapped \
             (default assets then fall back to the top-level USD cap)."
                .into(),
        );
    }
    let usd_limit = match controls.max_amount_per_payment {
        MaxAmountPerPayment::Disabled => "false".to_owned(),
        MaxAmountPerPayment::Usd(amount) => format!("${amount}"),
    };
    let symbol_note = rejects
        .usd_symbol
        .as_ref()
        .map(|symbol| format!(", including {symbol}"))
        .unwrap_or_default();
    ClientError::SpendControls(format!(
        "all payment requirements were rejected by spendControls.maxAmountPerPayment \
         ({usd_limit}{symbol_note}). Raise maxAmountPerPayment, set it to false to disable, \
         set allowedAssets[].maxAmountPerPayment for a per-asset atomic cap, \
         or set spendControls: false to disable all spend controls."
    ))
}
