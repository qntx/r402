#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::unwrap_used,
    reason = "on-chain tests panic on missing bytecode or RPC failure by design"
)]

//! On-chain canonical-address deployment verification and opt-in exact verify/settle.
//!
//! Every r402-evm chain leans on four canonical contract deployments:
//!
//! | Constant                     | Address                                      | Role                                    |
//! |------------------------------|----------------------------------------------|-----------------------------------------|
//! | [`PERMIT2_ADDRESS`]          | `0x000000000022D473030F116dDEE9F6B43aC78BA3` | Uniswap Permit2 (CREATE2 singleton)     |
//! | [`X402_EXACT_PERMIT2_PROXY`] | `0x402085c248EeA27D92E8b30b2C58ed07f9E20001` | x402 exact Permit2 proxy                |
//! | [`X402_UPTO_PERMIT2_PROXY`]  | `0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002` | x402 upto Permit2 proxy                 |
//! | [`VALIDATOR_ADDRESS`]        | `0xdAcD51A54883eb67D95FAEb2BBfdC4a9a6BD2a3B` | EIP-6492 / EIP-1271 / EOA verifier      |
//!
//! Presence is `eth_getCode(address)` nonempty. Tests skip when no RPC env is
//! set (no `#[ignore]`). They do not invent missing types.
//!
//! ## Environment
//!
//! - `R402_RPC_URLS` — comma-separated `<chain_id>=<url>` pairs. Unset or empty
//!   skips the canonical-address suite.
//! - `R402_ANVIL=1` — opt into Anvil at `http://127.0.0.1:8545` (chain id 8453
//!   as a stand-in CAIP-2 for EIP-3009 verify/settle). Canonical CREATE2
//!   addresses are not expected on a fresh Anvil.
//!
//! ```bash
//! R402_RPC_URLS="1=https://eth.llamarpc.com,8453=https://mainnet.base.org" \
//!   cargo test -p r402-evm --test onchain_deployment --all-features
//!
//! R402_ANVIL=1 cargo test -p r402-evm --test onchain_deployment --all-features
//! ```

use std::env;
use std::str::FromStr;
use std::time::Duration;

use alloy_network::EthereumWallet;
use alloy_primitives::Address;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use r402_client::SchemeClient;
use r402_evm::chain::{
    Eip155ChainProvider, Eip155ChainReference, Eip155MetaTransactionProvider, Eip155TokenDeployment,
};
use r402_evm::exact::X402_EXACT_PERMIT2_PROXY;
use r402_evm::{
    EVM_NETWORKS, Eip155Exact, Eip155ExactClient, Eip155ExactFacilitator, PERMIT2_ADDRESS, USDC,
    VALIDATOR_ADDRESS, X402_UPTO_PERMIT2_PROXY,
};
use r402_facilitator::Facilitator;
use r402_protocol::error::{AsPaymentProblem, FacilitatorError, VerificationError};
use r402_protocol::payment::{
    Base64Bytes, PaymentRequired, ResourceInfo, SettleRequest, SettleResponse, VerifyRequest,
    VerifyResponse,
};
use url::Url;

/// Anvil account 0. Local signing only.
const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const ANVIL_RPC: &str = "http://127.0.0.1:8545";

/// Canonical contracts r402 expects on every supported EVM chain.
const CANONICAL_CONTRACTS: &[(&str, Address)] = &[
    ("PERMIT2", PERMIT2_ADDRESS),
    ("X402_EXACT_PERMIT2_PROXY", X402_EXACT_PERMIT2_PROXY),
    ("X402_UPTO_PERMIT2_PROXY", X402_UPTO_PERMIT2_PROXY),
    ("VALIDATOR_6492", VALIDATOR_ADDRESS),
];

const PER_CALL_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct ContractCheck {
    chain_id: u64,
    chain_name: &'static str,
    contract: &'static str,
    address: Address,
    outcome: CheckOutcome,
}

#[derive(Debug)]
enum CheckOutcome {
    Deployed { bytecode_len: usize },
    Missing,
    RpcError(String),
}

fn anvil_opted_in() -> bool {
    matches!(env::var("R402_ANVIL"), Ok(value) if value == "1")
}

/// Parses `R402_RPC_URLS` into `(chain_id, url)` pairs. Empty when unset.
fn parse_rpc_urls() -> Vec<(u64, Url)> {
    let Ok(raw) = env::var("R402_RPC_URLS") else {
        return Vec::new();
    };
    raw.split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (chain_id_str, url_str) = entry.split_once('=')?;
            let chain_id = chain_id_str.trim().parse::<u64>().ok()?;
            let url = Url::parse(url_str.trim()).ok()?;
            Some((chain_id, url))
        })
        .collect()
}

fn anvil_rpc() -> Option<Url> {
    anvil_opted_in().then(|| Url::parse(ANVIL_RPC).expect("anvil rpc url"))
}

fn lookup_chain_name(chain_id: u64) -> &'static str {
    EVM_NETWORKS
        .iter()
        .find(|n| n.reference.parse::<u64>().ok() == Some(chain_id))
        .map_or("unknown", |n| n.name)
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

async fn check_contract(
    provider: &impl Provider,
    chain_id: u64,
    contract: &'static str,
    address: Address,
) -> ContractCheck {
    let chain_name = lookup_chain_name(chain_id);
    let outcome = match tokio::time::timeout(PER_CALL_TIMEOUT, provider.get_code_at(address)).await
    {
        Ok(Ok(code)) => {
            if code.is_empty() {
                CheckOutcome::Missing
            } else {
                CheckOutcome::Deployed {
                    bytecode_len: code.len(),
                }
            }
        }
        Ok(Err(err)) => CheckOutcome::RpcError(err.to_string()),
        Err(_) => {
            CheckOutcome::RpcError(format!("eth_getCode timed out after {PER_CALL_TIMEOUT:?}"))
        }
    };
    ContractCheck {
        chain_id,
        chain_name,
        contract,
        address,
        outcome,
    }
}

async fn check_chain(chain_id: u64, rpc_url: Url) -> Vec<ContractCheck> {
    let provider = ProviderBuilder::new().connect_http(rpc_url);
    let mut results = Vec::with_capacity(CANONICAL_CONTRACTS.len());
    for (contract, address) in CANONICAL_CONTRACTS {
        results.push(check_contract(&provider, chain_id, contract, *address).await);
    }
    results
}

fn print_report(all_results: &[ContractCheck]) {
    eprintln!();
    eprintln!(
        "{:<22} {:<8} {:<42} outcome",
        "chain", "id", "contract@address"
    );
    eprintln!("{}", "-".repeat(110));
    for row in all_results {
        let outcome_display = match &row.outcome {
            CheckOutcome::Deployed { bytecode_len } => format!("deployed ({bytecode_len} B)"),
            CheckOutcome::Missing => "MISSING".to_owned(),
            CheckOutcome::RpcError(msg) => format!("RPC error: {msg}"),
        };
        eprintln!(
            "{:<22} {:<8} {:<42} {outcome_display}",
            row.chain_name,
            row.chain_id,
            format!("{}@{}", row.contract, row.address)
        );
    }
    eprintln!();
}

fn distinct_chain_count(failures: &[&ContractCheck]) -> usize {
    let mut chains: Vec<u64> = failures.iter().map(|f| f.chain_id).collect();
    chains.sort_unstable();
    chains.dedup();
    chains.len()
}

#[tokio::test]
async fn canonical_addresses_deployed_on_configured_chains() {
    let urls = parse_rpc_urls();
    if urls.is_empty() {
        eprintln!(
            "R402_RPC_URLS is empty or unset — skipping canonical-address checks. \
             Example: R402_RPC_URLS=\"1=https://eth.llamarpc.com,8453=https://mainnet.base.org\""
        );
        return;
    }

    let mut all_results = Vec::with_capacity(urls.len() * CANONICAL_CONTRACTS.len());
    for (chain_id, url) in urls {
        eprintln!(
            "[onchain] checking chain {chain_id} ({}) via {url}",
            lookup_chain_name(chain_id)
        );
        all_results.extend(check_chain(chain_id, url).await);
    }

    print_report(&all_results);

    let failures: Vec<&ContractCheck> = all_results
        .iter()
        .filter(|r| !matches!(r.outcome, CheckOutcome::Deployed { .. }))
        .collect();

    assert!(
        failures.is_empty(),
        "{} canonical-address checks failed across {} chain(s); see report above",
        failures.len(),
        distinct_chain_count(&failures)
    );
}

fn wallet() -> EthereumWallet {
    let signer = PrivateKeySigner::from_str(ANVIL_KEY).expect("anvil key");
    EthereumWallet::from(signer)
}

fn chain_provider(chain_id: u64, rpc: &Url) -> Eip155ChainProvider {
    Eip155ChainProvider::new(
        Eip155ChainReference::new(chain_id),
        wallet(),
        &[(rpc.clone(), None)],
        true,
        false,
        15,
    )
    .expect("provider")
}

const fn pay_to() -> Address {
    alloy_primitives::address!("0x1111111111111111111111111111111111111111")
}

fn verify_targets() -> Vec<(u64, Url)> {
    let mut urls = parse_rpc_urls();
    if let Some(anvil) = anvil_rpc() {
        urls.push((8453, anvil));
    }
    urls
}

const ASSET_NOT_DEPLOYED: &str = "asset_not_deployed_contract";

fn token_for(chain_id: u64) -> &'static Eip155TokenDeployment {
    USDC::on(Eip155ChainReference::new(chain_id)).unwrap_or_else(USDC::base)
}

async fn asset_code_present(provider: &Eip155ChainProvider, asset: Address) -> bool {
    let inner = Eip155MetaTransactionProvider::inner(provider);
    let code = tokio::time::timeout(PER_CALL_TIMEOUT, inner.get_code_at(asset))
        .await
        .expect("eth_getCode(asset) timed out")
        .expect("eth_getCode(asset)");
    !code.is_empty()
}

async fn signed_eip3009(chain_id: u64) -> (serde_json::Value, serde_json::Value) {
    let token = token_for(chain_id);
    let tag = Eip155Exact::price_tag(pay_to(), token.amount(1_000_000u64), None);
    let signer = PrivateKeySigner::from_str(ANVIL_KEY).expect("anvil key");
    let client = Eip155ExactClient::new(signer);
    let required = PaymentRequired::new(ResourceInfo::new("https://example.com/paid"))
        .with_accepts(vec![tag.requirements.clone()]);
    let b64 = client
        .accept(&required)
        .first()
        .expect("one eip3009 accept")
        .sign()
        .await
        .expect("local EIP-712 sign");
    let json = Base64Bytes(b64.into_bytes()).decode().expect("b64");
    let payload: serde_json::Value = serde_json::from_slice(&json).expect("payload json");
    let requirements = serde_json::to_value(&tag.requirements).expect("requirements json");
    (payload, requirements)
}

fn as_verify_request(
    payload: &serde_json::Value,
    requirements: &serde_json::Value,
) -> VerifyRequest {
    VerifyRequest::from(serde_json::json!({
        "x402Version": 2,
        "paymentPayload": payload,
        "paymentRequirements": requirements,
    }))
}

fn verify_wire_reason(result: &Result<VerifyResponse, FacilitatorError>) -> Option<String> {
    match result {
        Ok(VerifyResponse::Invalid { reason, .. }) => {
            reason.as_ref().map(|r| r.as_str().to_owned())
        }
        Err(FacilitatorError::Verification(err)) => {
            Some(err.as_payment_problem().reason().as_str().to_owned())
        }
        _ => None,
    }
}

fn assert_undeployed_verify(result: &Result<VerifyResponse, FacilitatorError>) {
    assert!(
        !matches!(result, Ok(VerifyResponse::Valid { .. })),
        "undeployed asset must not verify Valid"
    );
    let reason = verify_wire_reason(result).unwrap_or_default();
    assert_eq!(
        reason, ASSET_NOT_DEPLOYED,
        "undeployed asset must be {ASSET_NOT_DEPLOYED}, got {result:?}"
    );
}

fn assert_structured_verify(result: Result<VerifyResponse, FacilitatorError>) {
    match result {
        Ok(VerifyResponse::Valid { .. } | VerifyResponse::Invalid { .. })
        | Err(FacilitatorError::Verification(_)) => {}
        other => panic!("expected structured EIP-3009 verify outcome, got {other:?}"),
    }
}

fn assert_undeployed_settle(result: Result<SettleResponse, FacilitatorError>) {
    match result {
        Ok(SettleResponse::Success { .. }) => {
            panic!("undeployed asset must not settle Success")
        }
        Ok(SettleResponse::Failure { reason, .. }) => {
            assert_eq!(reason.as_str(), ASSET_NOT_DEPLOYED, "got {reason}");
        }
        Err(FacilitatorError::Verification(err)) => {
            assert_eq!(
                err.as_payment_problem().reason().as_str(),
                ASSET_NOT_DEPLOYED,
                "got {err}"
            );
        }
        Err(FacilitatorError::Settlement(err)) => {
            assert_eq!(
                err.as_payment_problem().reason().as_str(),
                ASSET_NOT_DEPLOYED,
                "got {err}"
            );
        }
        other => panic!("expected {ASSET_NOT_DEPLOYED} settle, got {other:?}"),
    }
}

fn assert_structured_settle(result: Result<SettleResponse, FacilitatorError>) {
    match result {
        Ok(_) | Err(FacilitatorError::Verification(_) | FacilitatorError::Settlement(_)) => {}
        other => panic!("expected structured EIP-3009 settle outcome, got {other:?}"),
    }
}

async fn probe_rpc(provider: &Eip155ChainProvider) {
    let inner = Eip155MetaTransactionProvider::inner(provider);
    tokio::time::timeout(PER_CALL_TIMEOUT, inner.get_chain_id())
        .await
        .expect("RPC chain id timed out")
        .expect("RPC chain id");
}

#[tokio::test]
async fn verify_and_settle_eip3009_when_rpc_configured() {
    let targets = verify_targets();
    if targets.is_empty() {
        eprintln!(
            "R402_RPC_URLS unset and R402_ANVIL!=1 — skipping EIP-3009 verify/settle. \
             Set R402_ANVIL=1 (Anvil at {ANVIL_RPC}) or R402_RPC_URLS."
        );
        return;
    }

    for (chain_id, url) in targets {
        eprintln!(
            "[onchain] EIP-3009 verify/settle chain {chain_id} ({}) via {url}",
            lookup_chain_name(chain_id)
        );
        let provider = chain_provider(chain_id, &url);
        probe_rpc(&provider).await;
        let token = token_for(chain_id);
        let asset_deployed = asset_code_present(&provider, token.address).await;
        let fac = Eip155ExactFacilitator::try_new(provider).expect("try_new");

        let supported = fac.supported().await.expect("supported");
        assert!(
            supported.kinds.iter().any(|k| k.scheme == "exact"),
            "facilitator must advertise exact"
        );

        let empty = fac
            .verify(VerifyRequest::from(serde_json::json!({})))
            .await
            .expect_err("empty JSON is not a verify request");
        assert!(
            matches!(
                empty,
                FacilitatorError::Verification(VerificationError::InvalidFormat(_))
            ),
            "got {empty:?}"
        );

        let (payload, requirements) = signed_eip3009(chain_id).await;
        let verify_req = as_verify_request(&payload, &requirements);
        let verify_result = fac.verify(verify_req).await;
        if let Err(FacilitatorError::Verification(err)) = &verify_result {
            eprintln!(
                "[onchain] verify reason={} details={err}",
                err.as_payment_problem().reason().as_str()
            );
        }
        if asset_deployed {
            assert_structured_verify(verify_result);
        } else {
            assert_undeployed_verify(&verify_result);
        }

        if !is_loopback(&url) {
            eprintln!("[onchain] skipping settle on non-loopback RPC {url}");
            continue;
        }
        let settle_req = SettleRequest::from(as_verify_request(&payload, &requirements));
        let settle_result = fac.settle(settle_req).await;
        if let Err(err) = &settle_result {
            eprintln!("[onchain] settle error={err}");
        }
        if asset_deployed {
            assert_structured_settle(settle_result);
        } else {
            assert_undeployed_settle(settle_result);
        }
    }
}
