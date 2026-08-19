#![allow(unused_crate_dependencies, reason = "sibling tests consume them")]
#![allow(
    clippy::tests_outside_test_module,
    reason = "integration test binaries put #[test] fns at file scope"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "ignored on-chain test panics on missing bytecode by design"
)]

//! On-chain canonical-address deployment verification.
//!
//! Every r402-evm chain leans on four canonical contract deployments:
//!
//! | Constant                   | Address                                      | Role                                       |
//! |----------------------------|----------------------------------------------|--------------------------------------------|
//! | [`PERMIT2_ADDRESS`]        | `0x000000000022D473030F116dDEE9F6B43aC78BA3` | Uniswap Permit2 (singleton-via-CREATE2)    |
//! | [`X402_EXACT_PERMIT2_PROXY`] | `0x402085c248EeA27D92E8b30b2C58ed07f9E20001` | x402 *exact* scheme Permit2 proxy        |
//! | [`X402_UPTO_PERMIT2_PROXY`]  | `0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002` | x402 *upto* scheme Permit2 proxy         |
//! | [`VALIDATOR_ADDRESS`]      | `0xdAcD51A54883eb67D95FAEb2BBfdC4a9a6BD2a3B` | EIP-6492 / EIP-1271 / EOA verifier         |
//!
//! If any of these is missing on a chain r402 advertises, signature
//! verification or settlement silently fails on that chain in production.
//! These tests confirm presence by calling `eth_getCode(address)` and
//! asserting non-empty bytecode.
//!
//! ## Running
//!
//! The test is `#[ignore]` because it issues live RPC calls. Configure
//! the chains you want to verify via the `R402_RPC_URLS` environment
//! variable, formatted as a comma-separated list of `<chain_id>=<url>`
//! pairs:
//!
//! ```bash
//! R402_RPC_URLS=\
//! "1=https://eth.llamarpc.com,\
//! 8453=https://mainnet.base.org,\
//! 42161=https://arb1.arbitrum.io/rpc" \
//! cargo test -p r402-evm --test onchain_deployment -- --ignored --nocapture
//! ```
//!
//! Per-chain RPC failures (transport, rate-limit, timeout) are surfaced
//! verbatim. Missing bytecode triggers a final aggregated panic that
//! lists every (chain, address) that failed so a single run gives you
//! the complete picture rather than failing on the first miss.

use std::env;
use std::time::Duration;

use alloy_primitives::Address;
use alloy_provider::{Provider, ProviderBuilder};
use r402_evm::EVM_NETWORKS;
use r402_evm::exact::X402_EXACT_PERMIT2_PROXY;
use r402_evm::upto::X402_UPTO_PERMIT2_PROXY;
use r402_evm::{PERMIT2_ADDRESS, VALIDATOR_ADDRESS};
use url::Url;

/// Canonical contracts r402 expects on every supported EVM chain.
///
/// Order is preserved in the diagnostic output to make audit logs scan
/// predictably.
const CANONICAL_CONTRACTS: &[(&str, Address)] = &[
    ("PERMIT2", PERMIT2_ADDRESS),
    ("X402_EXACT_PERMIT2_PROXY", X402_EXACT_PERMIT2_PROXY),
    ("X402_UPTO_PERMIT2_PROXY", X402_UPTO_PERMIT2_PROXY),
    ("VALIDATOR_6492", VALIDATOR_ADDRESS),
];

/// Per-RPC timeout — generous enough for cold public RPCs, strict enough
/// to prevent a stalled chain from hanging the suite.
const PER_CALL_TIMEOUT: Duration = Duration::from_secs(15);

/// One row in the final report.
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

/// Parses `R402_RPC_URLS` into `(chain_id, url)` pairs.
///
/// Returns an empty vec if the env var is unset; the caller treats that
/// as "skip the test gracefully" rather than as a failure.
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

fn lookup_chain_name(chain_id: u64) -> &'static str {
    EVM_NETWORKS
        .iter()
        .find(|n| n.reference.parse::<u64>().ok() == Some(chain_id))
        .map_or("unknown", |n| n.name)
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

#[tokio::test]
#[ignore = "issues live RPC calls; configure via R402_RPC_URLS env var"]
async fn canonical_addresses_deployed_on_configured_chains() {
    let urls = parse_rpc_urls();
    if urls.is_empty() {
        // Skip silently rather than fail; running `--ignored` without
        // configuration is a common dev workflow ("does it compile?").
        eprintln!(
            "R402_RPC_URLS is empty or unset — no chains to check. \
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

    // Print a per-row report regardless of outcome — operators want the
    // full picture so they can plan deployments.
    eprintln!();
    eprintln!(
        "{:<22} {:<8} {:<42} {}",
        "chain", "id", "contract@address", "outcome"
    );
    eprintln!("{}", "-".repeat(110));
    for r in &all_results {
        let outcome_display = match &r.outcome {
            CheckOutcome::Deployed { bytecode_len } => format!("✓ deployed ({bytecode_len} B)"),
            CheckOutcome::Missing => "✗ MISSING".to_owned(),
            CheckOutcome::RpcError(msg) => format!("✗ RPC error: {msg}"),
        };
        eprintln!(
            "{:<22} {:<8} {:<42} {outcome_display}",
            r.chain_name,
            r.chain_id,
            format!("{}@{}", r.contract, r.address)
        );
    }
    eprintln!();

    let failures: Vec<&ContractCheck> = all_results
        .iter()
        .filter(|r| !matches!(r.outcome, CheckOutcome::Deployed { .. }))
        .collect();

    assert!(
        failures.is_empty(),
        "{} canonical-address checks failed across {} chain(s); see report above for details",
        failures.len(),
        urls_count_in(&failures)
    );
}

fn urls_count_in(failures: &[&ContractCheck]) -> usize {
    let mut chains: Vec<u64> = failures.iter().map(|f| f.chain_id).collect();
    chains.sort_unstable();
    chains.dedup();
    chains.len()
}
