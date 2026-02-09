//! EVM server-side "exact" scheme implementation.
//!
//! Implements [`SchemeServer`] for the `exact` scheme using ERC-3009
//! `transferWithAuthorization`. Handles price parsing (money → atomic USDC)
//! and requirement enhancement (adding EIP-712 domain parameters).
//!
//! Corresponds to Python SDK's `mechanisms/evm/exact/server.py`.

use r402::proto::{PaymentRequirements, SupportedKind};
use r402::scheme::{AssetAmount, SchemeError, SchemeServer};
use serde_json::Value;

use crate::chain::{AssetInfo, NetworkConfig};
use crate::exact::types::SCHEME_EXACT;
use crate::networks::known_networks;

/// EVM server implementation for the "exact" payment scheme.
///
/// Parses prices and enhances payment requirements with EIP-712 domain
/// parameters needed by EVM clients to construct valid signatures.
///
/// Corresponds to Python SDK's `ExactEvmScheme` in `exact/server.py`.
pub struct ExactEvmServer {
    networks: Vec<NetworkConfig>,
}

impl ExactEvmServer {
    /// Creates a new server scheme with all known EVM networks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            networks: known_networks(),
        }
    }

    /// Creates a server scheme with custom network configurations.
    #[must_use]
    pub const fn with_networks(networks: Vec<NetworkConfig>) -> Self {
        Self { networks }
    }

    /// Finds the network config for a CAIP-2 identifier.
    fn find_network(&self, network: &str) -> Option<&NetworkConfig> {
        self.networks.iter().find(|n| n.network == network)
    }

    /// Finds asset info by address within a network config.
    fn find_asset<'a>(config: &'a NetworkConfig, asset_address: &str) -> Option<&'a AssetInfo> {
        let addr = asset_address.parse().ok()?;
        config.find_asset(addr)
    }

    /// Default money-to-USDC conversion.
    ///
    /// Converts a decimal amount (e.g., `1.50`) to the atomic USDC amount
    /// (e.g., `"1500000"`) using the first asset on the network.
    fn default_money_conversion(
        &self,
        amount: f64,
        network: &str,
    ) -> Result<AssetAmount, SchemeError> {
        let config = self
            .find_network(network)
            .ok_or_else(|| -> SchemeError { format!("Unknown network: {network}").into() })?;

        let asset = config
            .assets
            .first()
            .ok_or_else(|| -> SchemeError { format!("No default asset for {network}").into() })?;

        let multiplier = 10u128.pow(u32::from(asset.decimals));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let token_amount = (amount * multiplier as f64) as u128;

        Ok(AssetAmount {
            amount: token_amount.to_string(),
            asset: format!("{:?}", asset.address),
            extra: Some(serde_json::json!({
                "name": asset.name,
                "version": asset.version,
            })),
        })
    }
}

impl Default for ExactEvmServer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ExactEvmServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExactEvmServer")
            .field("networks_count", &self.networks.len())
            .finish_non_exhaustive()
    }
}

impl SchemeServer for ExactEvmServer {
    fn scheme(&self) -> &str {
        SCHEME_EXACT
    }

    fn parse_price(&self, price: &Value, network: &str) -> Result<AssetAmount, SchemeError> {
        // Already an AssetAmount (object with "amount" key)
        if let Some(obj) = price.as_object()
            && let Some(amount) = obj.get("amount")
        {
            let asset =
                obj.get("asset")
                    .and_then(Value::as_str)
                    .ok_or_else(|| -> SchemeError {
                        format!("Asset address required for AssetAmount on {network}").into()
                    })?;

            return Ok(AssetAmount {
                amount: amount
                    .as_str()
                    .map_or_else(|| amount.to_string(), String::from),
                asset: asset.to_owned(),
                extra: obj.get("extra").cloned(),
            });
        }

        // Money string (e.g., "1.50" or "$1.50")
        let money_str = price
            .as_str()
            .or_else(|| price.as_f64().map(|_| ""))
            .ok_or_else(|| -> SchemeError { "Invalid price format".into() })?;

        let decimal_amount = if money_str.is_empty() {
            price.as_f64().unwrap_or(0.0)
        } else {
            parse_money_string(money_str)?
        };

        self.default_money_conversion(decimal_amount, network)
    }

    fn enhance_payment_requirements(
        &self,
        mut requirements: PaymentRequirements,
        _supported_kind: &SupportedKind,
        _extensions: &[String],
    ) -> PaymentRequirements {
        let Some(config) = self.find_network(&requirements.network) else {
            return requirements;
        };

        // Default asset if empty
        if requirements.asset.is_empty()
            && let Some(default_asset) = config.assets.first()
        {
            requirements.asset = format!("{:?}", default_asset.address);
        }

        // Find asset info for EIP-712 domain params
        let asset_info = Self::find_asset(config, &requirements.asset);

        // Convert decimal amount to smallest unit if needed
        if requirements.amount.contains('.')
            && let Some(info) = asset_info
            && let Ok(atomic) = parse_decimal_to_atomic(&requirements.amount, info.decimals)
        {
            requirements.amount = atomic;
        }

        // Add EIP-712 domain params to extra
        if let Some(info) = asset_info {
            let extra = requirements
                .extra
                .as_object_mut()
                .expect("extra should be an object");

            if !extra.contains_key("name") {
                extra.insert("name".to_owned(), Value::String(info.name.clone()));
            }
            if !extra.contains_key("version") {
                extra.insert("version".to_owned(), Value::String(info.version.clone()));
            }
        }

        requirements
    }
}

/// Parses a money string (e.g., `"1.50"`, `"$1.50"`, `"0.01"`) into `f64`.
fn parse_money_string(s: &str) -> Result<f64, SchemeError> {
    let cleaned = s.trim().trim_start_matches('$').trim();
    cleaned
        .parse::<f64>()
        .map_err(|e| -> SchemeError { format!("Invalid money string '{s}': {e}").into() })
}

/// Converts a decimal string to atomic units.
///
/// Example: `"1.50"` with 6 decimals → `"1500000"`.
fn parse_decimal_to_atomic(amount: &str, decimals: u8) -> Result<String, SchemeError> {
    let parts: Vec<&str> = amount.split('.').collect();
    let (whole, frac) = match parts.len() {
        1 => (parts[0], ""),
        2 => (parts[0], parts[1]),
        _ => return Err(format!("Invalid decimal amount: {amount}").into()),
    };

    let whole_val: u128 = whole
        .parse()
        .map_err(|e| -> SchemeError { format!("Invalid amount '{amount}': {e}").into() })?;

    let decimal_places = u32::from(decimals);
    let multiplier = 10u128.pow(decimal_places);

    let frac_val = if frac.is_empty() {
        0u128
    } else {
        let padded = format!("{frac:0<width$}", width = decimal_places as usize);
        let truncated = &padded[..decimal_places as usize];
        truncated
            .parse()
            .map_err(|e| -> SchemeError { format!("Invalid fractional amount: {e}").into() })?
    };

    let total = whole_val * multiplier + frac_val;
    Ok(total.to_string())
}
