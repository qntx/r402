//! Well-known Tron network definitions and token deployments.
//!
//! This module provides static network metadata and USDT token deployment
//! information for Tron mainnet and the Nile testnet, along with the
//! canonical SUN.io Permit2 and `x402ExactPermit2Proxy` contract addresses
//! used by the Permit2 asset transfer method.
//!
//! Contract addresses are sourced from the `x402-chain-tron` reference
//! implementation: <https://github.com/x402-rs/x402-rs/tree/main/crates/chains/x402-chain-tron>.

use std::str::FromStr;
use std::sync::LazyLock;

use r402_core::chain::{ChainId, NetworkInfo};
use r402_core::scheme::DefaultAssetInfo;

use crate::chain::{Address, TronChainReference, TronTokenDeployment};

/// Well-known Tron networks with their names and CAIP-2 identifiers.
pub static TRON_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "tron",
        namespace: "tron",
        reference: "0x2b6653dc",
    },
    NetworkInfo {
        name: "tron-nile",
        namespace: "tron",
        reference: "0xcd8690dc",
    },
];

/// Parses a well-known `Base58Check` address, panicking on malformed input.
///
/// Only used for hardcoded constants below; the address strings are fixed
/// and covered by tests, so a panic here indicates a build-time defect,
/// never a runtime condition.
#[allow(
    clippy::expect_used,
    reason = "hardcoded constant is infallible; validated by tests"
)]
fn well_known(s: &str) -> Address {
    Address::from_base58check(s).expect("well-known tron address must be valid base58check")
}

/// Canonical SUN.io Permit2 contract addresses, keyed by network.
///
/// This is the `verifyingContract` in the TIP-712 domain that clients sign
/// against — distinct from [`X402_EXACT_PERMIT2_PROXY`], which is the
/// `spender` in the signed message.
static SUN_PERMIT2: LazyLock<[(TronChainReference, Address); 2]> = LazyLock::new(|| {
    [
        (
            TronChainReference::MAINNET,
            well_known("TTJxU3P8rHycAyFY4kVtGNfmnMH4ezcuM9"),
        ),
        (
            TronChainReference::NILE,
            well_known("TCJjTtzwRJYPapGTdyJdKcr7MqkngRRWQx"),
        ),
    ]
});

/// Canonical `x402ExactPermit2Proxy` contract addresses, keyed by network.
///
/// This is the `spender` in the signed Permit2 message and the contract the
/// facilitator calls to settle — distinct from [`SUN_PERMIT2`].
static X402_EXACT_PERMIT2_PROXY: LazyLock<[(TronChainReference, Address); 2]> =
    LazyLock::new(|| {
        [
            (
                TronChainReference::MAINNET,
                well_known("TNtw4Wg6uQe4bqFywtcn5qagVZesSdYBSs"),
            ),
            (
                TronChainReference::NILE,
                well_known("TTjbkCh8sC4gNTWG48iWNssrLBqZq2tiTy"),
            ),
        ]
    });

impl TronChainReference {
    /// Returns the SUN.io Permit2 contract address for this network, if known.
    #[must_use]
    pub fn sun_permit2(self) -> Option<Address> {
        SUN_PERMIT2
            .iter()
            .find(|(chain, _)| *chain == self)
            .map(|(_, addr)| *addr)
    }

    /// Returns the `x402ExactPermit2Proxy` contract address for this network, if known.
    #[must_use]
    pub fn x402_exact_permit2_proxy(self) -> Option<Address> {
        X402_EXACT_PERMIT2_PROXY
            .iter()
            .find(|(chain, _)| *chain == self)
            .map(|(_, addr)| *addr)
    }
}

/// Well-known USDT token deployments on Tron networks.
///
/// Tron's canonical USDT does not implement EIP-3009, so it is deployed
/// here without a TIP-712 domain — pair with `AssetTransferMethod::Permit2`.
///
/// Use [`usdt_tron_deployment()`] for per-chain lookups, or
/// [`usdt_tron_deployments()`] to iterate over all known deployments.
static USDT_DEPLOYMENTS: LazyLock<Vec<TronTokenDeployment>> = LazyLock::new(|| {
    vec![
        // Tron mainnet — canonical USDT (TRC-20)
        // Verify: https://tronscan.org/#/token20/TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t
        TronTokenDeployment::new(
            TronChainReference::MAINNET,
            well_known("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
            6,
        ),
        // Tron Nile testnet — test USDT (TRC-20)
        // Verify: https://nile.tronscan.org/#/token20/TXLAQ63Xg1NAzckPwKHvzw7CSEmLMEqcdj
        TronTokenDeployment::new(
            TronChainReference::NILE,
            well_known("TXLAQ63Xg1NAzckPwKHvzw7CSEmLMEqcdj"),
            6,
        ),
    ]
});

/// Returns all known USDT deployments on Tron chains.
#[must_use]
pub fn usdt_tron_deployments() -> &'static [TronTokenDeployment] {
    &USDT_DEPLOYMENTS
}

/// Returns the USDT deployment for a specific Tron chain, if known.
#[must_use]
pub fn usdt_tron_deployment(chain: TronChainReference) -> Option<&'static TronTokenDeployment> {
    USDT_DEPLOYMENTS.iter().find(|d| d.chain_reference == chain)
}

/// Reverse lookup by TRC-20 address and CAIP-2 network.
#[must_use]
pub fn find_default_tron_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "tron" {
        return None;
    }
    let chain = TronChainReference::from_str(network.reference()).ok()?;
    let address: Address = asset.parse().ok()?;
    usdt_tron_deployment(chain)
        .filter(|deployment| deployment.address == address)
        .map(|deployment| {
            DefaultAssetInfo::new(
                deployment.address.to_string(),
                u32::from(deployment.decimals),
                "USDT",
            )
        })
}

/// Ergonomic accessors for USDT token deployments on well-known Tron chains.
///
/// Provides named methods for each supported chain, returning a static
/// reference to the deployment metadata. Combine with
/// [`TronTokenDeployment::amount`] for a fluent pricing API:
///
/// ```ignore
/// use r402_tron::{TronExact, USDT};
///
/// let tag = TronExact::price_tag(pay_to, USDT::mainnet().amount(1_000_000u64), None);
/// ```
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "USDT is a well-known token ticker"
)]
pub struct USDT;

#[allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl USDT {
    /// Looks up a USDT deployment by chain reference.
    #[must_use]
    pub fn on(chain: TronChainReference) -> Option<&'static TronTokenDeployment> {
        usdt_tron_deployment(chain)
    }

    /// Returns all known USDT deployments on Tron chains.
    #[must_use]
    pub fn all() -> &'static [TronTokenDeployment] {
        usdt_tron_deployments()
    }

    /// USDT on Tron mainnet (`tron:0x2b6653dc`).
    #[must_use]
    pub fn mainnet() -> &'static TronTokenDeployment {
        usdt_tron_deployment(TronChainReference::MAINNET)
            .expect("built-in USDT deployment for tron mainnet missing")
    }

    /// USDT on Tron Nile testnet (`tron:0xcd8690dc`).
    #[must_use]
    pub fn nile() -> &'static TronTokenDeployment {
        usdt_tron_deployment(TronChainReference::NILE)
            .expect("built-in USDT deployment for tron nile testnet missing")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdt_deployments_resolve() {
        assert_eq!(USDT::mainnet().decimals, 6);
        assert_eq!(USDT::nile().decimals, 6);
        assert_eq!(USDT::all().len(), 2);
        let network: ChainId = "tron:0x2b6653dc".parse().unwrap();
        let info = find_default_tron_asset(&USDT::mainnet().address.to_string(), &network).unwrap();
        assert_eq!(info.symbol, "USDT");
        assert_eq!(info.decimals, 6);
    }

    #[test]
    fn permit2_addresses_resolve_for_known_networks() {
        assert!(TronChainReference::MAINNET.sun_permit2().is_some());
        assert!(
            TronChainReference::MAINNET
                .x402_exact_permit2_proxy()
                .is_some()
        );
        assert!(TronChainReference::NILE.sun_permit2().is_some());
        assert!(
            TronChainReference::NILE
                .x402_exact_permit2_proxy()
                .is_some()
        );
    }

    #[test]
    fn permit2_addresses_absent_for_unknown_network() {
        let unknown = TronChainReference::new(0xdead_beef);
        assert!(unknown.sun_permit2().is_none());
        assert!(unknown.x402_exact_permit2_proxy().is_none());
    }
}
