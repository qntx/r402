//! Well-known EVM network definitions and token deployments.
//!
//! Static network metadata, USDC/USDM deployments, and the unified USD-pegged
//! default-asset table ([`get_default_evm_asset`], [`find_default_evm_asset`]).

use r402_protocol::network::NetworkInfo;

macro_rules! evm_networks {
    ($( $name:literal, $ref:literal );* $(;)?) => {
        &[ $( NetworkInfo { name: $name, namespace: "eip155", reference: $ref }, )* ]
    };
}

/// Well-known EVM (EIP-155) networks with their names and CAIP-2 identifiers.
///
/// Source: <https://developers.circle.com/stablecoins/usdc-contract-addresses>
pub static EVM_NETWORKS: &[NetworkInfo] = evm_networks![
    "ethereum",           "1";
    "ethereum-sepolia",   "11155111";
    "base",               "8453";
    "base-sepolia",       "84532";
    "arbitrum",           "42161";
    "arbitrum-sepolia",   "421614";
    "optimism",           "10";
    "optimism-sepolia",   "11155420";
    "polygon",            "137";
    "polygon-amoy",       "80002";
    "avalanche",          "43114";
    "avalanche-fuji",     "43113";
    "celo",               "42220";
    "celo-sepolia",       "11142220";
    "sei",                "1329";
    "sei-testnet",        "1328";
    "sonic",              "146";
    "sonic-blaze",        "57054";
    "unichain",           "130";
    "unichain-sepolia",   "1301";
    "world-chain",        "480";
    "world-chain-sepolia","4801";
    "zksync",             "324";
    "zksync-sepolia",     "300";
    "linea",              "59144";
    "linea-sepolia",      "59141";
    "ink",                "57073";
    "ink-sepolia",        "763373";
    "hyperevm",           "999";
    "hyperevm-testnet",   "998";
    "monad",              "143";
    "monad-testnet",      "10143";
    "plume",              "98866";
    "plume-testnet",      "98867";
    "codex",              "81224";
    "codex-testnet",      "812242";
    "xdc",                "50";
    "xdc-apothem",        "51";
    "xrpl-evm",           "1440000";
    "peaq",               "3338";
    "iotex",              "4689";
    "megaeth",            "4326";
    "stable",             "988";
    "stable-testnet",     "2201";
    "mezo",               "31612";
    "mezo-testnet",       "31611";
    "radius",             "723487";
    "radius-testnet",     "72344";
    "adi",                "36900";
    "hpp",                "190415";
    "hpp-sepolia",        "181228";
    "igra",               "38833";
    "flare",              "14";
];

mod table;
mod usdc;

pub use table::*;
pub use usdc::*;
