#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::missing_assert_message,
    clippy::panic,
    clippy::unwrap_used,
    reason = "idiomatic test-code patterns"
)]

//! `validate_accepts_against_supported` scoped to this accept list.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use r402_facilitator::Facilitator;
use r402_protocol::error::FacilitatorError;
use r402_protocol::network::ChainIdPattern;
use r402_protocol::payment::{
    PaymentRequirements, SettleRequest, SettleResponse, SupportedPaymentKind, SupportedResponse,
    VerifyRequest, VerifyResponse,
};
use r402_server::{
    FacilitatorSupportError, PaymentFlowConfig, ResourceServer, SDK_DEFAULT_ASSET_TRANSFER_METHOD,
    SchemeNetworkServer, validate_accepts_against_supported,
};

struct StubFacilitator;

impl Facilitator for StubFacilitator {
    fn verify(
        &self,
        _request: VerifyRequest,
    ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(VerifyResponse::valid("0xpayer")))
    }

    fn settle(
        &self,
        _request: SettleRequest,
    ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
        std::future::ready(Err(FacilitatorError::Onchain("unused".into())))
    }

    fn supported(
        &self,
    ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
        std::future::ready(Ok(SupportedResponse::new()))
    }
}

struct ProbeScheme {
    name: &'static str,
    flows: HashMap<String, PaymentFlowConfig>,
    support: Result<(), FacilitatorSupportError>,
}

impl ProbeScheme {
    fn exact(support: Result<(), FacilitatorSupportError>) -> Self {
        let mut flows = HashMap::new();
        flows.insert(
            SDK_DEFAULT_ASSET_TRANSFER_METHOD.into(),
            PaymentFlowConfig::authorization_only(),
        );
        Self {
            name: "exact",
            flows,
            support,
        }
    }

    fn named(name: &'static str, support: Result<(), FacilitatorSupportError>) -> Self {
        let mut scheme = Self::exact(support);
        scheme.name = name;
        scheme
    }
}

impl SchemeNetworkServer for ProbeScheme {
    fn scheme(&self) -> &str {
        self.name
    }

    fn default_asset_transfer_method(&self) -> &str {
        SDK_DEFAULT_ASSET_TRANSFER_METHOD
    }

    fn payment_flows(&self) -> &HashMap<String, PaymentFlowConfig> {
        &self.flows
    }

    fn validate_facilitator_support(
        &self,
        _network: &r402_protocol::network::ChainId,
        _kind: &SupportedPaymentKind,
        _facilitator_extensions: &[compact_str::CompactString],
    ) -> Result<(), FacilitatorSupportError> {
        self.support.clone()
    }
}

fn accept(scheme: &str, network: &str) -> PaymentRequirements {
    PaymentRequirements::new(
        scheme.into(),
        network.parse().unwrap(),
        "1".into(),
        "0xa".into(),
        "0xb".into(),
        60,
    )
}

fn exact_kind() -> SupportedPaymentKind {
    SupportedPaymentKind::new(2, "exact", "eip155:1")
}

fn server_with(scheme: ProbeScheme) -> ResourceServer {
    ResourceServer::new(Arc::new(StubFacilitator))
        .with_scheme(ChainIdPattern::wildcard("eip155"), scheme)
}

fn missing_fee_payer() -> FacilitatorSupportError {
    FacilitatorSupportError::MissingFeePayer {
        scheme: "exact".into(),
        network: "eip155:1".parse().unwrap(),
    }
}

#[test]
fn empty_kinds_is_kind_missing() {
    let server = server_with(ProbeScheme::exact(Ok(())));
    let err = validate_accepts_against_supported(
        &server,
        &[accept("exact", "eip155:1")],
        &SupportedResponse::new(),
    )
    .unwrap_err();
    assert!(matches!(err, FacilitatorSupportError::KindMissing { .. }));
}

#[test]
fn mismatched_kind_is_kind_missing() {
    let server = server_with(ProbeScheme::exact(Ok(())));
    let supported = SupportedResponse::new().with_kinds(vec![SupportedPaymentKind::new(
        2,
        "upto",
        "solana:devnet",
    )]);
    let err =
        validate_accepts_against_supported(&server, &[accept("exact", "eip155:1")], &supported)
            .unwrap_err();
    assert!(matches!(err, FacilitatorSupportError::KindMissing { .. }));
}

#[test]
fn version_mismatch_is_kind_missing() {
    let server = server_with(ProbeScheme::exact(Ok(())));
    let supported = SupportedResponse::new()
        .with_kinds(vec![SupportedPaymentKind::new(1, "exact", "eip155:1")]);
    let err =
        validate_accepts_against_supported(&server, &[accept("exact", "eip155:1")], &supported)
            .unwrap_err();
    assert!(matches!(err, FacilitatorSupportError::KindMissing { .. }));
}

#[test]
fn matching_kind_default_ok() {
    let server = server_with(ProbeScheme::exact(Ok(())));
    let supported = SupportedResponse::new().with_kinds(vec![exact_kind()]);
    validate_accepts_against_supported(&server, &[accept("exact", "eip155:1")], &supported)
        .expect("matching kind");
}

#[test]
fn matching_kind_forwards_scheme_error() {
    let server = server_with(ProbeScheme::exact(Err(missing_fee_payer())));
    let supported = SupportedResponse::new().with_kinds(vec![exact_kind()]);
    let err =
        validate_accepts_against_supported(&server, &[accept("exact", "eip155:1")], &supported)
            .unwrap_err();
    assert!(matches!(
        err,
        FacilitatorSupportError::MissingFeePayer { .. }
    ));
}

#[test]
fn unused_registration_is_ignored() {
    let mut server = ResourceServer::new(Arc::new(StubFacilitator));
    server.register_scheme(
        ChainIdPattern::wildcard("eip155"),
        ProbeScheme::exact(Ok(())),
    );
    server.register_scheme(
        ChainIdPattern::wildcard("eip155"),
        ProbeScheme::named("batch-settlement", Err(missing_fee_payer())),
    );
    let supported = SupportedResponse::new().with_kinds(vec![exact_kind()]);
    validate_accepts_against_supported(&server, &[accept("exact", "eip155:1")], &supported)
        .expect("unused batch-settlement must not fail exact accepts");
}

#[test]
fn unregistered_accept_skips_hook() {
    let server = ResourceServer::new(Arc::new(StubFacilitator));
    let supported = SupportedResponse::new().with_kinds(vec![exact_kind()]);
    validate_accepts_against_supported(&server, &[accept("exact", "eip155:1")], &supported)
        .expect("unregistered scheme skips validate_facilitator_support");
}

#[test]
fn second_accept_kind_missing() {
    let server = server_with(ProbeScheme::exact(Ok(())));
    let supported = SupportedResponse::new().with_kinds(vec![exact_kind()]);
    let err = validate_accepts_against_supported(
        &server,
        &[accept("exact", "eip155:1"), accept("upto", "eip155:1")],
        &supported,
    )
    .unwrap_err();
    match err {
        FacilitatorSupportError::KindMissing { scheme, network } => {
            assert_eq!(scheme, "upto");
            assert_eq!(network.to_string(), "eip155:1");
        }
        other => panic!("expected KindMissing, got {other:?}"),
    }
}
