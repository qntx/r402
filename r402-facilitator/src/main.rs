//! Production-ready x402 Facilitator HTTP server.
//!
//! # Usage
//!
//! ```bash
//! # Run with default config (config.toml in current directory)
//! cargo run -p r402-facilitator --features bin --release
//!
//! # Run with custom config path
//! CONFIG=/path/to/config.toml cargo run -p r402-facilitator --features bin
//!
//! # Configure logging level
//! RUST_LOG=info cargo run -p r402-facilitator --features bin
//! ```
//!
//! # Environment Variables
//!
//! - `CONFIG` — Path to TOML configuration file (default: `config.toml`)
//! - `HOST` — Override bind address (default: `0.0.0.0`)
//! - `PORT` — Override port (default: `4021`)
//! - `RUST_LOG` — Log level filter (default: `info`)

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use alloy_network::EthereumWallet;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_client::RpcClient;
use alloy_signer_local::PrivateKeySigner;
use alloy_transport_http::Http;
use alloy_transport_http::reqwest::{Client as ReqwestClient, Url};
use axum::http::Method;
use axum::{Json, Router};
use r402::facilitator::X402Facilitator;
use r402_evm::chain::parse_caip2;
use r402_evm::exact::facilitator::{ExactEvmConfig, ExactEvmFacilitator};
use r402_evm::networks::known_networks;
use tower_http::cors;
use tracing_subscriber::EnvFilter;

use r402_facilitator::config::{ChainConfig, FacilitatorConfig};
use r402_facilitator::handlers::{FacilitatorState, facilitator_router};

#[tokio::main]
async fn main() {
    // Initialize tracing with RUST_LOG env filter
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    if let Err(e) = run().await {
        tracing::error!("Facilitator failed: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = FacilitatorConfig::load()?;
    tracing::info!(
        host = %config.host,
        port = config.port,
        chains = config.chains.len(),
        "Loaded configuration"
    );

    if config.chains.is_empty() {
        tracing::warn!("No chains configured — facilitator will report no supported schemes");
    }

    let evm_config = ExactEvmConfig {
        deploy_erc4337_with_eip6492: config.deploy_erc4337_with_eip6492,
    };

    let known = known_networks();
    let mut facilitator = X402Facilitator::new();

    // Initialize EVM providers for each configured chain
    for (network_id, chain_cfg) in &config.chains {
        let chain_id = match parse_caip2(network_id) {
            Some(id) => id,
            None => {
                tracing::warn!(network = %network_id, "Skipping chain: invalid CAIP-2 identifier");
                continue;
            }
        };

        let key_str = chain_cfg.signer_private_key.trim();
        if key_str.is_empty() || key_str.starts_with('$') {
            tracing::warn!(
                network = %network_id,
                "Skipping chain: signer_private_key not resolved (missing env var?)"
            );
            continue;
        }

        let signer: PrivateKeySigner = key_str
            .parse()
            .map_err(|e| format!("Invalid signer key for {network_id}: {e}"))?;
        let signer_address = signer.address();
        let wallet = EthereumWallet::from(signer);

        // Try primary URL, then fallbacks
        let provider = match create_provider(network_id, chain_cfg, chain_id, &wallet).await {
            Some(p) => p,
            None => {
                tracing::error!(network = %network_id, "All RPC endpoints failed — skipping chain");
                continue;
            }
        };

        // Filter known network configs that match this chain ID
        let networks_for_chain: Vec<_> = known
            .iter()
            .filter(|n| n.chain_id == chain_id)
            .cloned()
            .collect();

        let network_ids: Vec<String> = networks_for_chain
            .iter()
            .map(|n| n.network.clone())
            .collect();

        if network_ids.is_empty() {
            tracing::warn!(
                network = %network_id,
                chain_id,
                "No known network config found — registering with provided network ID"
            );
            let evm_fac = ExactEvmFacilitator::with_config(provider, signer_address, evm_config);
            facilitator.register(vec![network_id.clone()], Box::new(evm_fac));
        } else {
            tracing::info!(
                network = %network_id,
                signer = %signer_address,
                networks = ?network_ids,
                "Registered EVM exact scheme"
            );
            let evm_fac = ExactEvmFacilitator::with_networks(
                provider,
                signer_address,
                evm_config,
                networks_for_chain,
            );
            facilitator.register(network_ids, Box::new(evm_fac));
        }
    }

    let state: FacilitatorState = Arc::new(facilitator);

    // Build Axum router
    let app = Router::new()
        .merge(facilitator_router(Arc::clone(&state)))
        .route("/health", axum::routing::get(health))
        .layer(
            cors::CorsLayer::new()
                .allow_origin(cors::Any)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers(cors::Any),
        );

    let addr = SocketAddr::new(config.host, config.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Facilitator listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Facilitator shut down gracefully");
    Ok(())
}

/// Health check endpoint.
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Creates an EVM provider with per-chain timeout, fallback URLs, and
/// optional startup health check (`eth_chainId`).
///
/// Tries the primary `rpc_url` first, then each `fallback_rpc_urls` in order.
/// Returns `None` if all endpoints fail.
async fn create_provider(
    network_id: &str,
    chain_cfg: &ChainConfig,
    expected_chain_id: u64,
    wallet: &EthereumWallet,
) -> Option<impl Provider + use<>> {
    let timeout = Duration::from_secs(chain_cfg.timeout_seconds);

    let urls = std::iter::once(&chain_cfg.rpc_url).chain(chain_cfg.fallback_rpc_urls.iter());

    for (i, url_str) in urls.enumerate() {
        let label = if i == 0 { "primary" } else { "fallback" };

        let rpc_url: Url = match url_str.parse() {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(
                    network = %network_id, url = %url_str, label,
                    "Invalid RPC URL: {e}"
                );
                continue;
            }
        };

        // Build provider with custom timeout
        let http_client = match ReqwestClient::builder().timeout(timeout).build() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    network = %network_id, label,
                    "Failed to build HTTP client: {e}"
                );
                continue;
            }
        };
        let transport = Http::with_client(http_client, rpc_url);
        let rpc_client = RpcClient::new(transport, false);
        let provider = ProviderBuilder::new()
            .wallet(wallet.clone())
            .connect_client(rpc_client);

        // Startup health check
        if chain_cfg.health_check {
            match provider.get_chain_id().await {
                Ok(id) if id == expected_chain_id => {
                    tracing::info!(
                        network = %network_id, url = %url_str, label,
                        chain_id = id, "RPC health check passed"
                    );
                    return Some(provider);
                }
                Ok(id) => {
                    tracing::warn!(
                        network = %network_id, url = %url_str, label,
                        expected = expected_chain_id, actual = id,
                        "Chain ID mismatch"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        network = %network_id, url = %url_str, label,
                        "RPC health check failed: {e}"
                    );
                    continue;
                }
            }
        }

        // No health check — use this provider directly
        tracing::info!(
            network = %network_id, url = %url_str, label,
            "Using RPC endpoint (health check disabled)"
        );
        return Some(provider);
    }

    None
}

/// Waits for Ctrl-C or SIGTERM (Unix) to initiate graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => tracing::info!("Received Ctrl-C, shutting down..."),
            _ = sigterm.recv() => tracing::info!("Received SIGTERM, shutting down..."),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.expect("failed to listen for Ctrl-C");
        tracing::info!("Received Ctrl-C, shutting down...");
    }
}
