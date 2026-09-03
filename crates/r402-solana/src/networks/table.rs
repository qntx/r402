//! Non-USDC default USD-pegged mint table (USDT, USDG, PYUSD, CASH).

use std::sync::LazyLock;

use solana_pubkey::pubkey;

#[cfg(feature = "client")]
use super::usdc::usdc_solana_deployments;
use crate::chain::{
    SolanaChainReference, SolanaTokenDeployment, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
};

/// Well-known USDT token deployments on Solana networks.
static USDT_DEPLOYMENTS: LazyLock<Vec<SolanaTokenDeployment>> = LazyLock::new(|| {
    vec![
        // Solana mainnet — Tether USDT (SPL Token)
        // Verify: https://solscan.io/token/Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA,
            pubkey!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").into(),
            6,
            TOKEN_PROGRAM_ID,
        ),
    ]
});

/// Well-known USDG token deployments on Solana networks.
static USDG_DEPLOYMENTS: LazyLock<Vec<SolanaTokenDeployment>> = LazyLock::new(|| {
    vec![
        // Solana mainnet — Global Dollar USDG (Token-2022)
        // Verify: https://solscan.io/token/2u1tszSeqZ3qBWF3uNGPFc8TzMk2tdiwknnRMWGWjGWH
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA,
            pubkey!("2u1tszSeqZ3qBWF3uNGPFc8TzMk2tdiwknnRMWGWjGWH").into(),
            6,
            TOKEN_2022_PROGRAM_ID,
        ),
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA_DEVNET,
            pubkey!("4F6PM96JJxngmHnZLBh9n58RH4aTVNWvDs2nuwrT5BP7").into(),
            6,
            TOKEN_2022_PROGRAM_ID,
        ),
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA_TESTNET,
            pubkey!("4F6PM96JJxngmHnZLBh9n58RH4aTVNWvDs2nuwrT5BP7").into(),
            6,
            TOKEN_2022_PROGRAM_ID,
        ),
    ]
});

/// Well-known PYUSD token deployments on Solana networks.
static PYUSD_DEPLOYMENTS: LazyLock<Vec<SolanaTokenDeployment>> = LazyLock::new(|| {
    vec![
        // Solana mainnet — PayPal USD (Token-2022)
        // Verify: https://solscan.io/token/2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA,
            pubkey!("2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo").into(),
            6,
            TOKEN_2022_PROGRAM_ID,
        ),
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA_DEVNET,
            pubkey!("CXk2AMBfi3TwaEL2468s6zP8xq9NxTXjp9gjMgzeUynM").into(),
            6,
            TOKEN_2022_PROGRAM_ID,
        ),
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA_TESTNET,
            pubkey!("CXk2AMBfi3TwaEL2468s6zP8xq9NxTXjp9gjMgzeUynM").into(),
            6,
            TOKEN_2022_PROGRAM_ID,
        ),
    ]
});

/// Well-known CASH token deployments on Solana networks.
static CASH_DEPLOYMENTS: LazyLock<Vec<SolanaTokenDeployment>> = LazyLock::new(|| {
    vec![
        // Solana mainnet — Phantom Cash (Token-2022)
        // Verify: https://solscan.io/token/CASHx9KJUStyftLFWGvEVf59SGeG9sh5FfcnZMVPCASH
        SolanaTokenDeployment::new(
            SolanaChainReference::SOLANA,
            pubkey!("CASHx9KJUStyftLFWGvEVf59SGeG9sh5FfcnZMVPCASH").into(),
            6,
            TOKEN_2022_PROGRAM_ID,
        ),
    ]
});

fn deployment_on(
    table: &'static [SolanaTokenDeployment],
    chain: &SolanaChainReference,
) -> Option<&'static SolanaTokenDeployment> {
    table.iter().find(|d| d.chain_reference == *chain)
}

/// Returns all known USDT deployments on Solana chains.
#[must_use]
pub fn usdt_solana_deployments() -> &'static [SolanaTokenDeployment] {
    &USDT_DEPLOYMENTS
}

/// Returns the USDT deployment for a specific Solana chain, if known.
#[must_use]
pub fn usdt_solana_deployment(
    chain: &SolanaChainReference,
) -> Option<&'static SolanaTokenDeployment> {
    deployment_on(&USDT_DEPLOYMENTS, chain)
}

/// Returns all known USDG deployments on Solana chains.
#[must_use]
pub fn usdg_solana_deployments() -> &'static [SolanaTokenDeployment] {
    &USDG_DEPLOYMENTS
}

/// Returns the USDG deployment for a specific Solana chain, if known.
#[must_use]
pub fn usdg_solana_deployment(
    chain: &SolanaChainReference,
) -> Option<&'static SolanaTokenDeployment> {
    deployment_on(&USDG_DEPLOYMENTS, chain)
}

/// Returns all known PYUSD deployments on Solana chains.
#[must_use]
pub fn pyusd_solana_deployments() -> &'static [SolanaTokenDeployment] {
    &PYUSD_DEPLOYMENTS
}

/// Returns the PYUSD deployment for a specific Solana chain, if known.
#[must_use]
pub fn pyusd_solana_deployment(
    chain: &SolanaChainReference,
) -> Option<&'static SolanaTokenDeployment> {
    deployment_on(&PYUSD_DEPLOYMENTS, chain)
}

/// Returns all known CASH deployments on Solana chains.
#[must_use]
pub fn cash_solana_deployments() -> &'static [SolanaTokenDeployment] {
    &CASH_DEPLOYMENTS
}

/// Returns the CASH deployment for a specific Solana chain, if known.
#[must_use]
pub fn cash_solana_deployment(
    chain: &SolanaChainReference,
) -> Option<&'static SolanaTokenDeployment> {
    deployment_on(&CASH_DEPLOYMENTS, chain)
}

#[cfg(feature = "client")]
pub(super) fn default_solana_mints()
-> impl Iterator<Item = (&'static SolanaTokenDeployment, &'static str)> {
    usdc_solana_deployments()
        .iter()
        .map(|d| (d, "USDC"))
        .chain(USDT_DEPLOYMENTS.iter().map(|d| (d, "USDT")))
        .chain(USDG_DEPLOYMENTS.iter().map(|d| (d, "USDG")))
        .chain(PYUSD_DEPLOYMENTS.iter().map(|d| (d, "PYUSD")))
        .chain(CASH_DEPLOYMENTS.iter().map(|d| (d, "CASH")))
}

/// Ergonomic accessors for USDT token deployments on well-known Solana chains.
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
    pub fn on(chain: &SolanaChainReference) -> Option<&'static SolanaTokenDeployment> {
        usdt_solana_deployment(chain)
    }

    /// Returns all known USDT deployments on Solana chains.
    #[must_use]
    pub fn all() -> &'static [SolanaTokenDeployment] {
        usdt_solana_deployments()
    }

    /// USDT on Solana mainnet (`solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`).
    #[must_use]
    pub fn solana() -> &'static SolanaTokenDeployment {
        usdt_solana_deployment(&SolanaChainReference::SOLANA)
            .expect("built-in USDT deployment for Solana mainnet missing")
    }
}

/// Ergonomic accessors for USDG token deployments on well-known Solana chains.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "USDG is a well-known token ticker"
)]
pub struct USDG;

#[allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl USDG {
    /// Looks up a USDG deployment by chain reference.
    #[must_use]
    pub fn on(chain: &SolanaChainReference) -> Option<&'static SolanaTokenDeployment> {
        usdg_solana_deployment(chain)
    }

    /// Returns all known USDG deployments on Solana chains.
    #[must_use]
    pub fn all() -> &'static [SolanaTokenDeployment] {
        usdg_solana_deployments()
    }

    /// USDG on Solana mainnet (`solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`).
    #[must_use]
    pub fn solana() -> &'static SolanaTokenDeployment {
        usdg_solana_deployment(&SolanaChainReference::SOLANA)
            .expect("built-in USDG deployment for Solana mainnet missing")
    }

    /// USDG on Solana devnet (`solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1`).
    #[must_use]
    pub fn solana_devnet() -> &'static SolanaTokenDeployment {
        usdg_solana_deployment(&SolanaChainReference::SOLANA_DEVNET)
            .expect("built-in USDG deployment for Solana devnet missing")
    }

    /// USDG on Solana testnet (`solana:4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z`).
    #[must_use]
    pub fn solana_testnet() -> &'static SolanaTokenDeployment {
        usdg_solana_deployment(&SolanaChainReference::SOLANA_TESTNET)
            .expect("built-in USDG deployment for Solana testnet missing")
    }
}

/// Ergonomic accessors for PYUSD token deployments on well-known Solana chains.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "PYUSD is a well-known token ticker"
)]
pub struct PYUSD;

#[allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl PYUSD {
    /// Looks up a PYUSD deployment by chain reference.
    #[must_use]
    pub fn on(chain: &SolanaChainReference) -> Option<&'static SolanaTokenDeployment> {
        pyusd_solana_deployment(chain)
    }

    /// Returns all known PYUSD deployments on Solana chains.
    #[must_use]
    pub fn all() -> &'static [SolanaTokenDeployment] {
        pyusd_solana_deployments()
    }

    /// PYUSD on Solana mainnet (`solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`).
    #[must_use]
    pub fn solana() -> &'static SolanaTokenDeployment {
        pyusd_solana_deployment(&SolanaChainReference::SOLANA)
            .expect("built-in PYUSD deployment for Solana mainnet missing")
    }

    /// PYUSD on Solana devnet (`solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1`).
    #[must_use]
    pub fn solana_devnet() -> &'static SolanaTokenDeployment {
        pyusd_solana_deployment(&SolanaChainReference::SOLANA_DEVNET)
            .expect("built-in PYUSD deployment for Solana devnet missing")
    }

    /// PYUSD on Solana testnet (`solana:4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z`).
    #[must_use]
    pub fn solana_testnet() -> &'static SolanaTokenDeployment {
        pyusd_solana_deployment(&SolanaChainReference::SOLANA_TESTNET)
            .expect("built-in PYUSD deployment for Solana testnet missing")
    }
}

/// Ergonomic accessors for CASH token deployments on well-known Solana chains.
#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "CASH is a well-known token ticker"
)]
pub struct CASH;

#[allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::expect_used,
    reason = "static deployment lookups are infallible for built-in data"
)]
impl CASH {
    /// Looks up a CASH deployment by chain reference.
    #[must_use]
    pub fn on(chain: &SolanaChainReference) -> Option<&'static SolanaTokenDeployment> {
        cash_solana_deployment(chain)
    }

    /// Returns all known CASH deployments on Solana chains.
    #[must_use]
    pub fn all() -> &'static [SolanaTokenDeployment] {
        cash_solana_deployments()
    }

    /// CASH on Solana mainnet (`solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`).
    #[must_use]
    pub fn solana() -> &'static SolanaTokenDeployment {
        cash_solana_deployment(&SolanaChainReference::SOLANA)
            .expect("built-in CASH deployment for Solana mainnet missing")
    }
}

#[cfg(test)]
mod tests {
    use solana_pubkey::pubkey;

    use super::*;

    #[test]
    fn usdt_mainnet_only_spl_token() {
        assert_eq!(USDT::all().len(), 1);
        assert_eq!(USDT::solana().token_program, TOKEN_PROGRAM_ID);
        assert_eq!(USDT::solana().decimals, 6);
        assert!(USDT::on(&SolanaChainReference::SOLANA_DEVNET).is_none());
        assert!(USDT::on(&SolanaChainReference::SOLANA_TESTNET).is_none());
        assert_eq!(
            USDT::solana().address,
            pubkey!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").into()
        );
    }

    #[test]
    fn usdg_token_2022_on_all_networks() {
        assert_eq!(USDG::all().len(), 3);
        for deployment in USDG::all() {
            assert_eq!(deployment.token_program, TOKEN_2022_PROGRAM_ID);
            assert_eq!(deployment.decimals, 6);
        }
        assert_eq!(
            USDG::solana_testnet().address,
            USDG::solana_devnet().address
        );
        assert_eq!(
            USDG::solana().address,
            pubkey!("2u1tszSeqZ3qBWF3uNGPFc8TzMk2tdiwknnRMWGWjGWH").into()
        );
    }

    #[test]
    fn pyusd_token_2022_on_all_networks() {
        assert_eq!(PYUSD::all().len(), 3);
        for deployment in PYUSD::all() {
            assert_eq!(deployment.token_program, TOKEN_2022_PROGRAM_ID);
            assert_eq!(deployment.decimals, 6);
        }
        assert_eq!(
            PYUSD::solana_testnet().address,
            PYUSD::solana_devnet().address
        );
        assert_eq!(
            PYUSD::solana().address,
            pubkey!("2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo").into()
        );
        let parsed = PYUSD::solana().parse("$0.10").unwrap();
        assert_eq!(parsed.amount, 100_000);
        assert_eq!(parsed.token.token_program, TOKEN_2022_PROGRAM_ID);
    }

    #[test]
    fn cash_mainnet_only_token_2022() {
        assert_eq!(CASH::all().len(), 1);
        assert_eq!(CASH::solana().token_program, TOKEN_2022_PROGRAM_ID);
        assert_eq!(CASH::solana().decimals, 6);
        assert!(CASH::on(&SolanaChainReference::SOLANA_DEVNET).is_none());
        assert!(CASH::on(&SolanaChainReference::SOLANA_TESTNET).is_none());
        assert_eq!(
            CASH::solana().address,
            pubkey!("CASHx9KJUStyftLFWGvEVf59SGeG9sh5FfcnZMVPCASH").into()
        );
    }
}
