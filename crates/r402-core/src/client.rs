//! Client-side payment orchestration (official `x402Client`, V2-only).
//!
//! Registers scheme clients, applies policies, selects a candidate, signs,
//! and runs lifecycle hooks including **`on_payment_response`**.
//!
//! Transports (HTTP / MCP) wrap this type; they do not re-implement selection.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use compact_str::CompactString;

use crate::amount::MoneyAmount;
use crate::chain::ChainIdPattern;
use crate::error::ClientError;
use crate::facilitator::{BoxFuture, FailureRecovery, HookDecision};
use crate::is_authorization_payment_flow;
use crate::is_recognized_payment_flow;
use crate::scheme::{
    DefaultAssetInfo, FirstMatch, PaymentCandidate, PaymentPolicy, PaymentSelector, SchemeClient,
};
use crate::wire::{PaymentRequired, SettleResponse};

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

/// Client spend controls enforced before policies. Default-on: default assets
/// only, capped at `$1`.
#[derive(Debug, Clone, Default)]
pub struct SpendControls {
    /// Per-payment USD cap on assets `find_default_asset` recognizes.
    pub max_amount_per_payment: MaxAmountPerPayment,
    /// Opt-in non-default assets.
    pub allowed_assets: AllowedAssets,
}

/// Context for payment-creation hooks.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PaymentCreationContext {
    /// Parsed payment requirements from the 402 challenge.
    pub payment_required: PaymentRequired,
}

impl PaymentCreationContext {
    /// Constructs a creation context from a 402 challenge.
    #[must_use]
    pub const fn new(payment_required: PaymentRequired) -> Self {
        Self { payment_required }
    }
}

/// Result of successful payment creation (transport-agnostic).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CreatedPayment {
    /// Base64-encoded payment payload for `Payment-Signature` / MCP meta.
    pub signed_payload: String,
    /// Challenge that was paid.
    pub payment_required: PaymentRequired,
}

impl CreatedPayment {
    /// Constructs a created-payment record.
    #[must_use]
    pub fn new(signed_payload: impl Into<String>, payment_required: PaymentRequired) -> Self {
        Self {
            signed_payload: signed_payload.into(),
            payment_required,
        }
    }
}

/// Context delivered after a paid request completes.
///
/// Official semantics: exactly one of `settle_response` or
/// `corrective_payment_required` is typically set —
/// - settle: paid request succeeded with `Payment-Response`
/// - corrective 402: server rejected with a new `Payment-Required`
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PaymentResponseContext {
    /// Original 402 challenge used to build the payment.
    pub payment_required: PaymentRequired,
    /// Signed payload that was submitted.
    pub signed_payload: String,
    /// Parsed settle outcome when present.
    pub settle_response: Option<SettleResponse>,
    /// Corrective `Payment-Required` when the paid retry returned 402.
    pub corrective_payment_required: Option<PaymentRequired>,
}

impl PaymentResponseContext {
    /// Constructs a payment-response context.
    #[must_use]
    pub fn new(payment_required: PaymentRequired, signed_payload: impl Into<String>) -> Self {
        Self {
            payment_required,
            signed_payload: signed_payload.into(),
            settle_response: None,
            corrective_payment_required: None,
        }
    }

    /// Builder: attach a settle response.
    #[must_use]
    pub fn with_settle_response(mut self, settle: SettleResponse) -> Self {
        self.settle_response = Some(settle);
        self
    }

    /// Builder: attach a corrective payment-required challenge.
    #[must_use]
    pub fn with_corrective_payment_required(mut self, required: PaymentRequired) -> Self {
        self.corrective_payment_required = Some(required);
        self
    }
}

/// Result of [`ClientHooks::on_payment_response`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PaymentResponseResult {
    /// When `true`, the transport should retry once with a freshly built payload.
    pub recovered: bool,
}

impl PaymentResponseResult {
    /// No recovery — continue with the response as-is.
    #[must_use]
    pub const fn continue_() -> Self {
        Self { recovered: false }
    }

    /// Signal one corrective retry.
    #[must_use]
    pub const fn recovered() -> Self {
        Self { recovered: true }
    }
}

/// Lifecycle hooks for the payment client (V2).
///
/// All methods default to no-ops. Override only what you need.
pub trait ClientHooks: Send + Sync {
    /// Runs before payment payload creation. Abort skips signing.
    fn before_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
    ) -> impl Future<Output = HookDecision> + Send + 'a {
        async { HookDecision::Continue }
    }

    /// Runs after a payment payload is successfully created.
    fn after_payment_creation<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _created: &'a CreatedPayment,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }

    /// Runs when payment creation fails. May recover with a substitute payload.
    fn on_payment_creation_failure<'a>(
        &'a self,
        _ctx: &'a PaymentCreationContext,
        _error: &'a str,
    ) -> impl Future<Output = FailureRecovery<CreatedPayment>> + Send + 'a {
        async { FailureRecovery::Propagate }
    }

    /// Runs after each paid response (settle success or corrective 402).
    ///
    /// Returning [`PaymentResponseResult::recovered`] asks the transport to
    /// retry once with a freshly built payment payload.
    fn on_payment_response<'a>(
        &'a self,
        _ctx: &'a PaymentResponseContext,
    ) -> impl Future<Output = PaymentResponseResult> + Send + 'a {
        async { PaymentResponseResult::continue_() }
    }
}

/// Object-safe erasure of [`ClientHooks`].
pub trait DynClientHooks: Send + Sync {
    /// See [`ClientHooks::before_payment_creation`].
    fn before_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>>;

    /// See [`ClientHooks::after_payment_creation`].
    fn after_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        created: &'a CreatedPayment,
    ) -> BoxFuture<'a, ()>;

    /// See [`ClientHooks::on_payment_creation_failure`].
    fn on_payment_creation_failure<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        error: &'a str,
    ) -> BoxFuture<'a, FailureRecovery<CreatedPayment>>;

    /// See [`ClientHooks::on_payment_response`].
    fn on_payment_response<'a>(
        &'a self,
        ctx: &'a PaymentResponseContext,
    ) -> Pin<Box<dyn Future<Output = PaymentResponseResult> + Send + 'a>>;
}

impl<T: ClientHooks + ?Sized> DynClientHooks for T {
    fn before_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
    ) -> Pin<Box<dyn Future<Output = HookDecision> + Send + 'a>> {
        Box::pin(<Self as ClientHooks>::before_payment_creation(self, ctx))
    }

    fn after_payment_creation<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        created: &'a CreatedPayment,
    ) -> BoxFuture<'a, ()> {
        Box::pin(<Self as ClientHooks>::after_payment_creation(
            self, ctx, created,
        ))
    }

    fn on_payment_creation_failure<'a>(
        &'a self,
        ctx: &'a PaymentCreationContext,
        error: &'a str,
    ) -> BoxFuture<'a, FailureRecovery<CreatedPayment>> {
        Box::pin(<Self as ClientHooks>::on_payment_creation_failure(
            self, ctx, error,
        ))
    }

    fn on_payment_response<'a>(
        &'a self,
        ctx: &'a PaymentResponseContext,
    ) -> Pin<Box<dyn Future<Output = PaymentResponseResult> + Send + 'a>> {
        Box::pin(<Self as ClientHooks>::on_payment_response(self, ctx))
    }
}

impl Debug for dyn DynClientHooks {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("DynClientHooks")
    }
}

/// V2 payment client — scheme registry + selector + policies + hooks.
///
/// Mirrors Go/TS `x402Client` without V1 registration paths.
pub struct PaymentClient<S = FirstMatch> {
    schemes: Vec<Arc<dyn SchemeClient>>,
    selector: S,
    policies: Vec<Arc<dyn PaymentPolicy>>,
    hooks: Vec<Arc<dyn DynClientHooks>>,
    spend_controls: Option<SpendControls>,
}

impl PaymentClient<FirstMatch> {
    /// Empty client with [`FirstMatch`] selection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for PaymentClient<FirstMatch> {
    fn default() -> Self {
        Self {
            schemes: Vec::new(),
            selector: FirstMatch,
            policies: Vec::new(),
            hooks: Vec::new(),
            spend_controls: Some(SpendControls::default()),
        }
    }
}

impl<S> Debug for PaymentClient<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentClient")
            .field("schemes", &self.schemes.len())
            .field("policies", &self.policies.len())
            .field("hooks", &self.hooks.len())
            .finish_non_exhaustive()
    }
}

impl<S> PaymentClient<S> {
    /// Registers a V2 scheme client.
    #[must_use]
    pub fn register(mut self, scheme: impl SchemeClient + 'static) -> Self {
        self.schemes.push(Arc::new(scheme));
        self
    }

    /// Replaces the payment selector.
    #[must_use]
    pub fn with_selector<P: PaymentSelector>(self, selector: P) -> PaymentClient<P> {
        PaymentClient {
            schemes: self.schemes,
            selector,
            policies: self.policies,
            hooks: self.hooks,
            spend_controls: self.spend_controls,
        }
    }

    /// Enables spend controls with the given configuration.
    #[must_use]
    pub fn with_spend_controls(mut self, controls: SpendControls) -> Self {
        self.spend_controls = Some(controls);
        self
    }

    /// Disables all spend controls (any asset, no caps).
    #[must_use]
    pub fn disable_spend_controls(mut self) -> Self {
        self.spend_controls = None;
        self
    }

    /// Appends a payment policy (applied in registration order).
    #[must_use]
    pub fn with_policy(mut self, policy: impl PaymentPolicy + 'static) -> Self {
        self.policies.push(Arc::new(policy));
        self
    }

    /// Registers a client lifecycle hook.
    #[must_use]
    pub fn with_hook(mut self, hook: impl ClientHooks + 'static) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// Number of registered scheme clients.
    #[must_use]
    pub fn scheme_count(&self) -> usize {
        self.schemes.len()
    }

    /// Number of registered hooks.
    #[must_use]
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }
}

impl<S: PaymentSelector> PaymentClient<S> {
    /// Collects candidates from every registered scheme client.
    #[must_use]
    pub fn candidates(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let mut out = Vec::new();
        for client in &self.schemes {
            out.extend(client.accept(payment_required));
        }
        out
    }

    /// Creates a signed payment payload for a 402 challenge.
    ///
    /// Runs before/after/failure hooks around selection + signing.
    ///
    /// # Errors
    ///
    /// Propagates selection, signing, and before-hook abort errors.
    pub async fn create_payment(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<CreatedPayment, ClientError> {
        let ctx = PaymentCreationContext {
            payment_required: payment_required.clone(),
        };

        if let Some(err) = self.run_before_creation(&ctx).await {
            return Err(err);
        }

        match self.create_payment_inner(payment_required).await {
            Ok(created) => {
                self.run_after_creation(&ctx, &created).await;
                Ok(created)
            }
            Err(err) => self.recover_creation(&ctx, err).await,
        }
    }

    async fn run_before_creation(&self, ctx: &PaymentCreationContext) -> Option<ClientError> {
        for hook in &self.hooks {
            if let Some(err) =
                Self::client_error_from_abort(hook.before_payment_creation(ctx).await)
            {
                return Some(err);
            }
        }
        None
    }

    fn client_error_from_abort(decision: HookDecision) -> Option<ClientError> {
        let HookDecision::Abort { reason, message } = decision else {
            return None;
        };
        let detail = if message.is_empty() {
            reason
        } else {
            format!("{reason}: {message}")
        };
        Some(ClientError::Parse(detail))
    }

    async fn run_after_creation(&self, ctx: &PaymentCreationContext, created: &CreatedPayment) {
        for hook in &self.hooks {
            hook.after_payment_creation(ctx, created).await;
        }
    }

    async fn recover_creation(
        &self,
        ctx: &PaymentCreationContext,
        err: ClientError,
    ) -> Result<CreatedPayment, ClientError> {
        let msg = err.to_string();
        for hook in &self.hooks {
            let recovery = hook.on_payment_creation_failure(ctx, &msg).await;
            if let FailureRecovery::Recovered(created) = recovery {
                return Ok(created);
            }
        }
        Err(err)
    }

    async fn create_payment_inner(
        &self,
        payment_required: &PaymentRequired,
    ) -> Result<CreatedPayment, ClientError> {
        let candidates = self.candidates(payment_required);
        let selected = self.select_candidate(&candidates)?;
        let signed_payload = selected.sign().await?;
        Ok(CreatedPayment::new(
            signed_payload,
            payment_required.clone(),
        ))
    }

    fn select_candidate<'a>(
        &self,
        candidates: &'a [PaymentCandidate],
    ) -> Result<&'a PaymentCandidate, ClientError> {
        let recognized: Vec<&PaymentCandidate> = candidates
            .iter()
            .filter(|candidate| is_recognized_payment_flow(candidate.requirements.extra.as_ref()))
            .collect();
        if recognized.is_empty() {
            return Err(if candidates.is_empty() {
                ClientError::NoMatchingPaymentOption
            } else {
                ClientError::UnrecognizedPaymentFlow
            });
        }

        let mut filtered = self.apply_spend_controls(recognized)?;
        for policy in &self.policies {
            filtered = policy.apply(filtered);
            if filtered.is_empty() {
                return Err(ClientError::NoMatchingPaymentOption);
            }
        }

        let preferred = prefer_authorization(filtered);
        self.selector
            .select(&preferred)
            .ok_or(ClientError::NoMatchingPaymentOption)
    }

    fn apply_spend_controls<'a>(
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

    /// Dispatches [`ClientHooks::on_payment_response`] for every hook.
    ///
    /// First `recovered: true` wins; remaining hooks still run (instrumentation).
    pub async fn handle_payment_response(
        &self,
        ctx: &PaymentResponseContext,
    ) -> PaymentResponseResult {
        let mut recovered = false;
        for hook in &self.hooks {
            let result = hook.on_payment_response(ctx).await;
            if result.recovered {
                recovered = true;
            }
        }
        if recovered {
            PaymentResponseResult::recovered()
        } else {
            PaymentResponseResult::continue_()
        }
    }
}

fn prefer_authorization(candidates: Vec<&PaymentCandidate>) -> Vec<&PaymentCandidate> {
    let authorization: Vec<&PaymentCandidate> = candidates
        .iter()
        .copied()
        .filter(|candidate| is_authorization_payment_flow(candidate.requirements.extra.as_ref()))
        .collect();
    if authorization.is_empty() {
        candidates
    } else {
        authorization
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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use super::*;
    use crate::chain::ChainId;
    use crate::scheme::{PaymentCandidateSigner, SchemeId, Sealed};
    use crate::wire::{PaymentRequirements, ResourceInfo};

    const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
    const NETWORK: &str = "eip155:8453";

    struct StubSigner(String);
    impl PaymentCandidateSigner for StubSigner {
        fn sign_payment<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>> {
            Box::pin(async move { Ok(self.0.clone()) })
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

    impl Sealed for StubScheme {}
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

    struct AbortHook;
    impl ClientHooks for AbortHook {
        fn before_payment_creation<'a>(
            &'a self,
            _: &PaymentCreationContext,
        ) -> impl Future<Output = HookDecision> + Send + 'a {
            std::future::ready(HookDecision::Abort {
                reason: "nope".into(),
                message: String::new(),
            })
        }
    }

    struct RecoverHook;
    impl ClientHooks for RecoverHook {
        fn on_payment_response<'a>(
            &'a self,
            _: &PaymentResponseContext,
        ) -> impl Future<Output = PaymentResponseResult> + Send + 'a {
            std::future::ready(PaymentResponseResult::recovered())
        }
    }

    fn sample_required() -> PaymentRequired {
        required_with("eip155:1", "0xa", "1", None)
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
        PaymentRequired::new(ResourceInfo::new("https://example.com")).with_accepts(vec![req])
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
    async fn create_payment_signs() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare());
        let created = client.create_payment(&sample_required()).await.unwrap();
        assert_eq!(created.signed_payload, "c2lnbmVk");
    }

    #[tokio::test]
    async fn before_hook_aborts() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare())
            .with_hook(AbortHook);
        let err = client.create_payment(&sample_required()).await.unwrap_err();
        assert!(matches!(err, ClientError::Parse(s) if s.contains("nope")));
    }

    #[tokio::test]
    async fn payment_response_hook_recovers() {
        let client = PaymentClient::new().with_hook(RecoverHook);
        let ctx = PaymentResponseContext::new(sample_required(), "x")
            .with_corrective_payment_required(sample_required());
        let result = client.handle_payment_response(&ctx).await;
        assert!(result.recovered);
    }

    #[tokio::test]
    async fn no_schemes_yields_no_match() {
        let client = PaymentClient::new();
        let err = client.create_payment(&sample_required()).await.unwrap_err();
        assert!(matches!(err, ClientError::NoMatchingPaymentOption));
    }

    #[tokio::test]
    async fn spend_controls_allow_at_default_usd_cap() {
        let created = client_with_default()
            .create_payment(&required_with(NETWORK, USDC, "1000000", None))
            .await
            .unwrap();
        assert_eq!(created.signed_payload, "c2lnbmVk");
    }

    #[tokio::test]
    async fn spend_controls_reject_above_default_usd_cap() {
        let err = client_with_default()
            .create_payment(&required_with(NETWORK, USDC, "1000001", None))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientError::SpendControls(msg) if msg.contains("maxAmountPerPayment"))
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
        assert_eq!(created.signed_payload, "c2lnbmVk");
        let candidates = client_with_default().candidates(&required);
        let selected = PaymentClient::new()
            .register(StubScheme::with_default(usdc_info()))
            .select_candidate(&candidates)
            .unwrap();
        assert_eq!(selected.amount, "500000");
    }

    #[tokio::test]
    async fn spend_controls_reject_unrecognized_assets_and_missing_lookup() {
        let err = client_with_default()
            .create_payment(&required_with(NETWORK, "0xCustomUnknownToken", "1", None))
            .await
            .unwrap_err();
        assert!(matches!(err, ClientError::SpendControls(msg) if msg.contains("allowedAssets")));
    }

    #[tokio::test]
    async fn spend_controls_reject_scheme_without_find_default_asset() {
        let missing_lookup = PaymentClient::new()
            .register(StubScheme::bare())
            .create_payment(&required_with(NETWORK, USDC, "1", None))
            .await
            .unwrap_err();
        assert!(
            matches!(missing_lookup, ClientError::SpendControls(msg) if msg.contains("allowedAssets"))
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
            matches!(err, ClientError::SpendControls(msg) if msg.contains("maxAmountPerPayment"))
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
            matches!(err, ClientError::SpendControls(msg) if msg.contains("maxAmountPerPayment"))
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
            matches!(err, ClientError::SpendControls(msg) if msg.contains("allowedAssets maxAmountPerPayment"))
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
            matches!(err, ClientError::SpendControls(msg) if msg.contains("allowedAssets maxAmountPerPayment"))
        );

        let mixed = required_accepts(vec![
            accept(NETWORK, custom, "1.5", None),
            accept(NETWORK, custom, "100", None),
        ]);
        let candidates = client.candidates(&mixed);
        let selected = client.select_candidate(&candidates).unwrap();
        assert_eq!(selected.amount, "100");
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
                matches!(err, ClientError::SpendControls(msg) if msg.contains("must be an integer atomic amount"))
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
            matches!(over_id, ClientError::SpendControls(msg) if msg.contains("allowedAssets maxAmountPerPayment"))
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
            matches!(over_symbol, ClientError::SpendControls(msg) if msg.contains("allowedAssets maxAmountPerPayment"))
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
            matches!(err, ClientError::SpendControls(msg) if msg.contains("maxAmountPerPayment"))
        );
    }

    #[tokio::test]
    async fn drops_unrecognized_payment_flow_and_prefers_authorization() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare());
        let required = required_accepts(vec![
            accept(
                NETWORK,
                USDC,
                "200",
                Some(serde_json::json!({"paymentFlow": "future-flow"})),
            ),
            accept(NETWORK, USDC, "100", None),
        ]);
        let recognized = client.candidates(&required);
        let picked = client.select_candidate(&recognized).unwrap();
        assert_eq!(picked.amount, "100");
    }

    #[tokio::test]
    async fn all_unrecognized_payment_flow_errors() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare());
        let err = client
            .create_payment(&required_accepts(vec![accept(
                NETWORK,
                USDC,
                "1",
                Some(serde_json::json!({"paymentFlow": "future-flow"})),
            )]))
            .await
            .unwrap_err();
        assert!(matches!(err, ClientError::UnrecognizedPaymentFlow));
    }

    #[tokio::test]
    async fn prefers_omitted_authorization_over_upfront() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare());
        let mixed = required_accepts(vec![
            accept(
                NETWORK,
                USDC,
                "100",
                Some(serde_json::json!({"paymentFlow": "upfront"})),
            ),
            accept(NETWORK, USDC, "200", None),
        ]);
        let mixed_candidates = client.candidates(&mixed);
        let mixed_selected = client.select_candidate(&mixed_candidates).unwrap();
        assert_eq!(mixed_selected.amount, "200");
    }

    #[tokio::test]
    async fn prefers_explicit_authorization_over_escrow() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare());
        let escrow_and_auth = required_accepts(vec![
            accept(
                NETWORK,
                USDC,
                "100",
                Some(serde_json::json!({"paymentFlow": "escrow"})),
            ),
            accept(
                NETWORK,
                USDC,
                "200",
                Some(serde_json::json!({"paymentFlow": "authorization"})),
            ),
        ]);
        let auth_candidates = client.candidates(&escrow_and_auth);
        let auth_selected = client.select_candidate(&auth_candidates).unwrap();
        assert_eq!(auth_selected.amount, "200");
    }

    #[tokio::test]
    async fn selects_upfront_when_it_is_the_only_accept() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare());
        let upfront_only = required_accepts(vec![accept(
            NETWORK,
            USDC,
            "100",
            Some(serde_json::json!({"paymentFlow": "upfront"})),
        )]);
        let upfront_candidates = client.candidates(&upfront_only);
        let upfront_selected = client.select_candidate(&upfront_candidates).unwrap();
        assert_eq!(upfront_selected.amount, "100");
    }

    struct UpfrontOnly;
    impl PaymentPolicy for UpfrontOnly {
        fn apply<'a>(&self, candidates: Vec<&'a PaymentCandidate>) -> Vec<&'a PaymentCandidate> {
            candidates
                .into_iter()
                .filter(|candidate| {
                    candidate
                        .requirements
                        .extra
                        .as_ref()
                        .and_then(|extra| extra.get("paymentFlow"))
                        .and_then(serde_json::Value::as_str)
                        == Some("upfront")
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn policy_can_override_authorization_preference() {
        let client = PaymentClient::new()
            .disable_spend_controls()
            .register(StubScheme::bare())
            .with_policy(UpfrontOnly);
        let required = required_accepts(vec![
            accept(NETWORK, USDC, "200", None),
            accept(
                NETWORK,
                USDC,
                "100",
                Some(serde_json::json!({"paymentFlow": "upfront"})),
            ),
        ]);
        let candidates = client.candidates(&required);
        let selected = client.select_candidate(&candidates).unwrap();
        assert_eq!(selected.amount, "100");
    }

    #[tokio::test]
    async fn spend_controls_drop_non_integer_usd_capped_default() {
        let err = client_with_default()
            .create_payment(&required_with(NETWORK, USDC, "0.01", None))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientError::SpendControls(msg) if msg.contains("maxAmountPerPayment"))
        );
    }
}
