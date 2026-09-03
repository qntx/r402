//! Well-known Solana network definitions and default-asset lookup.

mod table;
mod usdc;

#[cfg(feature = "client")]
use std::str::FromStr;

#[cfg(feature = "client")]
use r402_client::DefaultAssetInfo;
#[cfg(feature = "client")]
use r402_protocol::ChainId;
use r402_protocol::NetworkInfo;
pub use table::{
    CASH, PYUSD, USDG, USDT, cash_solana_deployment, cash_solana_deployments,
    pyusd_solana_deployment, pyusd_solana_deployments, usdg_solana_deployment,
    usdg_solana_deployments, usdt_solana_deployment, usdt_solana_deployments,
};
pub use usdc::{USDC, usdc_solana_deployment, usdc_solana_deployments};

#[cfg(feature = "client")]
use crate::chain::Address;
#[cfg(any(feature = "client", test))]
use crate::chain::SolanaChainReference;

/// Well-known Solana networks with their names and CAIP-2 identifiers.
pub static SOLANA_NETWORKS: &[NetworkInfo] = &[
    NetworkInfo {
        name: "solana",
        namespace: "solana",
        reference: "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
    },
    NetworkInfo {
        name: "solana-devnet",
        namespace: "solana",
        reference: "EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
    },
    NetworkInfo {
        name: "solana-testnet",
        namespace: "solana",
        reference: "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z",
    },
];

/// Reverse lookup by mint and CAIP-2 network across default Solana USD assets.
#[cfg(feature = "client")]
#[must_use]
pub fn find_default_solana_asset(asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
    if network.namespace() != "solana" {
        return None;
    }
    let chain = SolanaChainReference::from_str(network.reference()).ok()?;
    let mint: Address = asset.parse().ok()?;
    table::default_solana_mints()
        .find(|(deployment, _)| deployment.chain_reference == chain && deployment.address == mint)
        .map(|(deployment, symbol)| {
            DefaultAssetInfo::new(
                deployment.address.to_string(),
                u32::from(deployment.decimals),
                symbol,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solana_networks_include_testnet() {
        assert_eq!(SOLANA_NETWORKS.len(), 3);
        let testnet = SOLANA_NETWORKS
            .iter()
            .find(|n| n.name == "solana-testnet")
            .expect("solana-testnet row");
        assert_eq!(testnet.namespace, "solana");
        assert_eq!(testnet.reference, "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z");
    }

    #[test]
    fn chain_reference_testnet_round_trips() {
        let parsed: SolanaChainReference = "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z"
            .parse()
            .expect("testnet reference");
        assert_eq!(parsed, SolanaChainReference::SOLANA_TESTNET);
        assert_eq!(parsed.to_string(), "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z");
    }
}
