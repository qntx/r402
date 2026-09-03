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

//! Offline exact-scheme tests (no RPC).

use r402_protocol::scheme::SchemeId;
use r402_solana::chain::{Address, SolanaChainReference, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};
use r402_solana::{CASH, PYUSD, SOLANA_NETWORKS, SolanaExact, USDC, USDG, USDT};
use solana_pubkey::pubkey;

#[test]
fn crate_name_matches_directory() {
    assert_eq!(env!("CARGO_PKG_NAME"), "r402-solana");
}

#[test]
fn solana_exact_scheme_id() {
    assert_eq!(SolanaExact.namespace(), "solana");
    assert_eq!(SolanaExact.scheme(), "exact");
    assert_eq!(SolanaExact.caip_family(), "solana:*");
}

#[test]
fn networks_table_has_mainnet_devnet_testnet() {
    assert_eq!(SOLANA_NETWORKS.len(), 3);
    let names: Vec<_> = SOLANA_NETWORKS.iter().map(|n| n.name).collect();
    assert_eq!(names, ["solana", "solana-devnet", "solana-testnet"]);
}

#[test]
fn default_usd_mints_match_oracle() {
    assert_eq!(USDC::all().len(), 3);
    assert_eq!(USDC::solana().token_program, TOKEN_PROGRAM_ID);
    assert_eq!(USDT::all().len(), 1);
    assert_eq!(USDT::solana().token_program, TOKEN_PROGRAM_ID);
    assert_eq!(USDG::all().len(), 3);
    assert_eq!(USDG::solana().token_program, TOKEN_2022_PROGRAM_ID);
    assert_eq!(PYUSD::all().len(), 3);
    assert_eq!(CASH::solana().token_program, TOKEN_2022_PROGRAM_ID);
    assert_eq!(
        USDC::solana().address,
        pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").into()
    );
}

#[test]
fn payload_extra_roundtrip() {
    use r402_solana::exact::SupportedPaymentKindExtra;

    let extra = SupportedPaymentKindExtra {
        fee_payer: Address::new(pubkey!("11111111111111111111111111111111")),
        memo: Some("order-123".into()),
    };
    let json = serde_json::to_value(&extra).expect("serialize extra");
    assert_eq!(json["feePayer"], "11111111111111111111111111111111");
    assert_eq!(json["memo"], "order-123");
    let back: SupportedPaymentKindExtra = serde_json::from_value(json).expect("deserialize extra");
    assert_eq!(back, extra);
}

#[test]
fn chain_reference_converts_to_caip2() {
    let chain: r402_protocol::ChainId = SolanaChainReference::SOLANA.into();
    assert_eq!(chain.to_string(), "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
}

#[cfg(feature = "client")]
#[test]
fn find_default_solana_asset_covers_usdc_and_other_mints() {
    use r402_protocol::ChainId;
    use r402_solana::find_default_solana_asset;

    let mainnet: ChainId = "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
        .parse()
        .expect("mainnet");
    let usdc = find_default_solana_asset("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", &mainnet)
        .expect("USDC");
    assert_eq!(usdc.symbol, "USDC");
    assert_eq!(usdc.decimals, 6);
    let pyusd = find_default_solana_asset("2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo", &mainnet)
        .expect("PYUSD");
    assert_eq!(pyusd.symbol, "PYUSD");
    assert!(find_default_solana_asset("11111111111111111111111111111111", &mainnet).is_none());
}

#[cfg(feature = "facilitator")]
#[test]
fn transfer_amount_rule_allows_overpay() {
    use r402_solana::exact::facilitator::transfer_amount_meets_requirement;

    assert!(transfer_amount_meets_requirement(1_000_000, 1_000_000));
    assert!(transfer_amount_meets_requirement(1_000_001, 1_000_000));
    assert!(!transfer_amount_meets_requirement(999_999, 1_000_000));
}
