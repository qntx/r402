//! Shared Sequential HTTP test helpers.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "helpers are shared across integration-test crates"
)]
#![allow(
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::significant_drop_in_scrutinee,
    clippy::unwrap_used,
    reason = "test helpers"
)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use axum_core::body::Body;
use axum_core::extract::Request;
use axum_core::response::Response;
use compact_str::CompactString;
use http::{HeaderValue, StatusCode};
use r402_facilitator::Facilitator;
use r402_http::server::{PAYMENT_SIGNATURE, StaticPriceTags, X402Layer, X402Middleware};
use r402_protocol::error::{ErrorReason, FacilitatorError, FacilitatorTransportKind};
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{
    Base64Bytes, Extensions, PaymentPayload, PaymentRequirements, PriceTag, SettleRequest,
    SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};
use r402_server::{
    AfterVerifyDecision, BeforeOpDecision, PaymentFlowConfig, PaymentFlowName, PaymentHookContext,
    ResourceServer, ResourceServerHooks, SDK_DEFAULT_ASSET_TRANSFER_METHOD, SchemeNetworkServer,
    SkipHandlerDirective, VerifiedPaymentCanceledContext, VerifyResultContext,
};
use serde_json::json;
use tower::{Layer, Service};
use url::Url;

pub fn base_url() -> Url {
    "https://api.example.com".parse().unwrap()
}

pub fn eip155_requirements() -> PaymentRequirements {
    PaymentRequirements::new(
        "exact".into(),
        "eip155:1".parse().unwrap(),
        "1000000".into(),
        "0xpay".into(),
        "0xasset".into(),
        60,
    )
}

pub fn eip155_tag() -> PriceTag {
    PriceTag::new(eip155_requirements())
}

pub fn upfront_tag() -> PriceTag {
    PriceTag::new(eip155_requirements().with_extra(json!({ "paymentFlow": "upfront" })))
}

pub fn escrow_tag() -> PriceTag {
    PriceTag::new(eip155_requirements().with_extra(json!({ "paymentFlow": "escrow" })))
}

pub fn payment_request(accepted: &PaymentRequirements) -> Request {
    let payload = PaymentPayload::new(accepted.clone(), json!({"sig": "0x"}));
    let encoded = Base64Bytes::encode(serde_json::to_vec(&payload).unwrap());
    Request::builder()
        .uri("/paid")
        .header(
            PAYMENT_SIGNATURE,
            HeaderValue::from_bytes(encoded.as_ref()).unwrap(),
        )
        .body(Body::empty())
        .unwrap()
}

pub fn unpaid_request() -> Request {
    Request::builder().uri("/paid").body(Body::empty()).unwrap()
}

#[derive(Clone)]
pub enum VerifyScript {
    Valid,
    Invalid(ErrorReason),
    Transport,
}

#[derive(Clone, Copy)]
pub enum SettleScript {
    Ok,
    Fail,
    Transport,
}

pub struct FakeFacilitator {
    pub verifies: AtomicUsize,
    pub settles: AtomicUsize,
    pub last_settle_amount: Mutex<Option<String>>,
    pub verify: Mutex<VerifyScript>,
    pub settle: Mutex<SettleScript>,
}

impl FakeFacilitator {
    pub fn new() -> Self {
        Self {
            verifies: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            last_settle_amount: Mutex::new(None),
            verify: Mutex::new(VerifyScript::Valid),
            settle: Mutex::new(SettleScript::Ok),
        }
    }

    pub fn settle_count(&self) -> usize {
        self.settles.load(Ordering::SeqCst)
    }

    pub fn verify_count(&self) -> usize {
        self.verifies.load(Ordering::SeqCst)
    }

    pub fn last_amount(&self) -> Option<String> {
        self.last_settle_amount.lock().unwrap().clone()
    }
}

impl Facilitator for FakeFacilitator {
    fn verify(
        &self,
        _request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        let out = match *self.verify.lock().unwrap() {
            VerifyScript::Valid => Ok(VerifyResponse::valid("0xpayer")),
            VerifyScript::Invalid(ref reason) => Ok(VerifyResponse::invalid(None, reason.clone())),
            VerifyScript::Transport => Err(FacilitatorError::transport(
                FacilitatorTransportKind::Timeout,
            )),
        };
        std::future::ready(out)
    }

    fn settle(
        &self,
        request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
        self.settles.fetch_add(1, Ordering::SeqCst);
        let amount = request
            .into_json()
            .get("paymentRequirements")
            .and_then(|req| req.get("amount"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        *self.last_settle_amount.lock().unwrap() = amount;
        let out = match *self.settle.lock().unwrap() {
            SettleScript::Ok => Ok(SettleResponse::Success {
                payer: "0xpayer".into(),
                transaction: "0xtx".into(),
                network: "eip155:1".into(),
                amount: Some("1000000".into()),
                extensions: Extensions::new(),
                extension_responses: Extensions::new(),
                extra: None,
            }),
            SettleScript::Fail => Ok(SettleResponse::Failure {
                reason: ErrorReason::InvalidTransactionState,
                message: Some("reverted".into()),
                payer: None,
                transaction: CompactString::default(),
                network: "eip155:1".into(),
                extensions: Extensions::new(),
                extension_responses: Extensions::new(),
                extra: None,
            }),
            SettleScript::Transport => Err(FacilitatorError::transport(
                FacilitatorTransportKind::Timeout,
            )),
        };
        std::future::ready(out)
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(SupportedResponse::new()))
    }
}

pub struct FlowScheme {
    flows: HashMap<String, PaymentFlowConfig>,
    settle_on_cancel: bool,
}

impl FlowScheme {
    pub fn authorization() -> Self {
        Self::from_config(PaymentFlowConfig::authorization_only(), false)
    }

    pub fn auth_and_upfront() -> Self {
        Self::from_config(PaymentFlowConfig::authorization_and_upfront(), false)
    }

    pub fn escrow() -> Self {
        Self::from_config(
            PaymentFlowConfig::new(vec![PaymentFlowName::Escrow], PaymentFlowName::Escrow),
            true,
        )
    }

    fn from_config(config: PaymentFlowConfig, settle_on_cancel: bool) -> Self {
        let mut flows = HashMap::new();
        flows.insert(SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(), config);
        Self {
            flows,
            settle_on_cancel,
        }
    }
}

impl SchemeNetworkServer for FlowScheme {
    fn scheme(&self) -> &'static str {
        "exact"
    }

    fn default_asset_transfer_method(&self) -> &str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        &self.flows
    }

    fn settle_on_cancel<'a>(
        &'a self,
        ctx: &'a VerifiedPaymentCanceledContext,
    ) -> impl Future<Output = Option<PaymentRequirements>> + Send + 'a {
        let out = self
            .settle_on_cancel
            .then(|| ctx.payment.requirements.clone());
        std::future::ready(out)
    }
}

#[derive(Clone)]
pub struct OkInner;

impl Service<Request> for OkInner {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: Request) -> Self::Future {
        std::future::ready(Ok(Response::new(Body::from("ok"))))
    }
}

#[derive(Clone)]
pub struct StatusInner(pub StatusCode);

impl Service<Request> for StatusInner {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: Request) -> Self::Future {
        let mut response = Response::new(Body::from("err"));
        *response.status_mut() = self.0;
        std::future::ready(Ok(response))
    }
}

pub fn middleware(fac: Arc<FakeFacilitator>, scheme: FlowScheme) -> X402Middleware {
    X402Middleware::from_facilitator(fac)
        .with_base_url(base_url())
        .with_scheme(ChainIdPattern::wildcard("eip155"), scheme)
}

pub fn middleware_on(server: ResourceServer) -> X402Middleware {
    X402Middleware::from_resource_server(server).with_base_url(base_url())
}

pub struct SkipAfterVerify;

impl ResourceServerHooks for SkipAfterVerify {
    fn after_verify<'a>(
        &'a self,
        _: &'a VerifyResultContext,
    ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
        std::future::ready(AfterVerifyDecision::SkipHandler {
            response: SkipHandlerDirective::empty(),
        })
    }
}

pub struct SkipVerifyThenHandler;

impl ResourceServerHooks for SkipVerifyThenHandler {
    fn before_verify<'a>(
        &'a self,
        _: &'a PaymentHookContext,
    ) -> impl Future<Output = BeforeOpDecision<VerifyResponse>> + Send + 'a {
        std::future::ready(BeforeOpDecision::Skip {
            result: VerifyResponse::valid("0xlocal"),
        })
    }

    fn after_verify<'a>(
        &'a self,
        _: &'a VerifyResultContext,
    ) -> impl Future<Output = AfterVerifyDecision> + Send + 'a {
        std::future::ready(AfterVerifyDecision::SkipHandler {
            response: SkipHandlerDirective::empty(),
        })
    }
}

pub async fn call_layer<S>(layer: X402Layer<StaticPriceTags>, inner: S, req: Request) -> Response
where
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
{
    let mut svc = layer.layer(inner);
    svc.call(req).await.expect("infallible")
}
