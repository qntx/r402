//! Facilitator server configuration.
//!
//! Loads configuration from a TOML file with support for environment variable
//! expansion in string values. Variables use `$VAR` or `${VAR}` syntax.
//!
//! # Example Configuration
//!
//! ```toml
//! host = "0.0.0.0"
//! port = 4021
//! deploy_erc4337_with_eip6492 = false
//!
//! [chains."eip155:8453"]
//! rpc_url = "https://mainnet.base.org"
//! signer_private_key = "$SIGNER_KEY_BASE"
//! ```
//!
//! # Environment Variables
//!
//! - `CONFIG` — Path to configuration file (default: `config.toml`)
//! - `HOST` — Override server bind address
//! - `PORT` — Override server port
//! - Chain-specific signer keys referenced by `$VAR` in the config file

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Top-level facilitator configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacilitatorConfig {
    /// Server bind address (default: `0.0.0.0`).
    #[serde(default = "default_host")]
    pub host: IpAddr,

    /// Server port (default: `4021`).
    #[serde(default = "default_port")]
    pub port: u16,

    /// EVM chain configurations keyed by CAIP-2 network identifier.
    #[serde(default)]
    pub chains: HashMap<String, ChainConfig>,

    /// Whether to deploy ERC-4337 smart wallets via ERC-6492 factory calls.
    #[serde(default)]
    pub deploy_erc4337_with_eip6492: bool,
}

/// Per-chain configuration for an EVM network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// HTTP RPC endpoint URL.
    pub rpc_url: String,

    /// Private key for the facilitator signer (hex, with or without `0x` prefix).
    /// Supports `$VAR` / `${VAR}` for environment variable expansion.
    pub signer_private_key: String,
}

fn default_host() -> IpAddr {
    IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0))
}

fn default_port() -> u16 {
    4021
}

impl FacilitatorConfig {
    /// Loads configuration from the path given by the `CONFIG` environment
    /// variable, falling back to `config.toml` in the current directory.
    ///
    /// After loading, all string values with `$VAR` / `${VAR}` references
    /// are expanded from the process environment. `HOST` and `PORT` env vars
    /// override the file values.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".to_owned());
        Self::load_from(&path)
    }

    /// Loads configuration from a specific file path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load_from(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = if Path::new(path).exists() {
            std::fs::read_to_string(path)?
        } else {
            // If no config file exists, use empty TOML and rely on defaults
            String::new()
        };

        // Expand environment variables in the raw TOML string
        let expanded = expand_env_vars(&content);

        let mut config: Self = toml::from_str(&expanded)?;

        // Allow HOST / PORT env overrides
        if let Ok(host) = std::env::var("HOST") {
            if let Ok(addr) = host.parse() {
                config.host = addr;
            }
        }
        if let Ok(port) = std::env::var("PORT") {
            if let Ok(p) = port.parse() {
                config.port = p;
            }
        }

        Ok(config)
    }
}

/// Expands `$VAR` and `${VAR}` patterns in a string from environment variables.
///
/// Unresolved variables are left as-is.
fn expand_env_vars(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            let braced = chars.peek() == Some(&'{');
            if braced {
                chars.next(); // consume '{'
            }

            let mut var_name = String::new();
            while let Some(&c) = chars.peek() {
                if braced {
                    if c == '}' {
                        chars.next();
                        break;
                    }
                } else if !c.is_ascii_alphanumeric() && c != '_' {
                    break;
                }
                var_name.push(c);
                chars.next();
            }

            if var_name.is_empty() {
                result.push('$');
                if braced {
                    result.push('{');
                }
            } else if let Ok(val) = std::env::var(&var_name) {
                result.push_str(&val);
            } else {
                // Leave unresolved variable as-is
                result.push('$');
                if braced {
                    result.push('{');
                }
                result.push_str(&var_name);
                if braced {
                    result.push('}');
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}
