#![allow(
    unused_crate_dependencies,
    reason = "lib optional deps appear as unused externs"
)]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_assert_message,
    reason = "idiomatic test-code patterns"
)]

//! Offline upto-scheme tests (no RPC).

use r402_protocol::scheme::SchemeId;
use r402_solana::SolanaUpto;
use r402_solana::upto::{ASSET_TRANSFER_METHOD_CHANNEL, is_upto_svm_payload};

#[test]
fn solana_upto_scheme_id() {
    assert_eq!(SolanaUpto.namespace(), "solana");
    assert_eq!(SolanaUpto.scheme(), "upto");
    assert_eq!(SolanaUpto.caip_family(), "solana:*");
    assert_eq!(ASSET_TRANSFER_METHOD_CHANNEL, "channel");
}

#[test]
fn payload_shape_accepts_channel_fields() {
    let payload = serde_json::json!({
        "from": "11111111111111111111111111111111",
        "maxAmount": "10000",
        "expiresAt": 1_893_456_000i64,
        "validAfter": 0i64,
        "nonce": "42",
        "openSlot": "341000000",
        "channelId": "SysvarC1ock11111111111111111111111111111111",
        "deposit": "10000",
        "authorizedSigner": "SysvarRent111111111111111111111111111111111",
        "openTransaction": "AQ==",
    });
    assert!(is_upto_svm_payload(&payload));
    assert!(!is_upto_svm_payload(
        &serde_json::json!({"transaction": "AQ=="})
    ));
}

#[cfg(feature = "facilitator")]
#[test]
fn try_new_is_result_so_question_mark_compiles() {
    use r402_solana::upto::facilitator::SolanaUptoFacilitator;

    let _fac = SolanaUptoFacilitator::try_new(()).expect("infallible constructor");
}

#[cfg(feature = "server")]
#[tokio::test]
async fn create_payment_required_writes_escrow_payment_flow() {
    use std::future::Future;
    use std::sync::Arc;

    use r402_facilitator::Facilitator;
    use r402_protocol::{
        Extensions, FacilitatorError, PaymentRequirements, ResourceInfo, SettleRequest,
        SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
    };
    use r402_server::{PaymentRequiredBuildContext, ResourceServer};
    use r402_solana::USDC;
    use r402_solana::UptoSvmScheme;
    use serde_json::Value;
    use solana_keypair::Keypair;
    use solana_signer::Signer;

    struct NoopFacilitator;

    impl Facilitator for NoopFacilitator {
        fn verify(
            &self,
            _request: VerifyRequest,
        ) -> impl Future<Output = Result<VerifyResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(VerifyResponse::valid("payer")))
        }

        fn settle(
            &self,
            _request: SettleRequest,
        ) -> impl Future<Output = Result<SettleResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SettleResponse::Success {
                payer: "payer".into(),
                transaction: "sig".into(),
                network: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".into(),
                amount: Some("10000".into()),
                extensions: Extensions::new(),
                extension_responses: Extensions::new(),
                extra: None,
            }))
        }

        fn supported(
            &self,
        ) -> impl Future<Output = Result<SupportedResponse, FacilitatorError>> + Send {
            std::future::ready(Ok(SupportedResponse::default()))
        }
    }

    let key = Keypair::new();
    let scheme = UptoSvmScheme::new(key.insecure_clone());
    let rs = ResourceServer::new(Arc::new(NoopFacilitator))
        .with_scheme(r402_protocol::ChainIdPattern::wildcard("solana"), scheme);
    let req = PaymentRequirements::new(
        "upto".into(),
        "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1"
            .parse()
            .expect("devnet"),
        "10000".into(),
        "SysvarRent111111111111111111111111111111111".into(),
        USDC::solana_devnet().address.to_string().into(),
        600,
    );
    let built = rs
        .create_payment_required_response(
            vec![req],
            PaymentRequiredBuildContext {
                resource: ResourceInfo::new("https://example.com/upto"),
                error: None,
                extensions: Extensions::new(),
                supported: SupportedResponse::default(),
                payment_payload: None,
            },
        )
        .await
        .expect("402");
    let extra = built
        .accepts
        .first()
        .and_then(|accept| accept.extra.as_ref())
        .expect("upto 402 extra");
    assert_eq!(
        extra.get("paymentFlow").and_then(Value::as_str),
        Some("escrow")
    );
    assert!(extra.get("assetTransferMethod").is_none());
    assert_eq!(
        extra.get("receiverAuthorizer").and_then(Value::as_str),
        Some(key.pubkey().to_string().as_str())
    );
}
