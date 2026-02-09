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
    /// Primary HTTP RPC endpoint URL.
    pub rpc_url: String,

    /// Fallback RPC endpoint URLs tried in order when the primary fails
    /// the startup health check.
    #[serde(default)]
    pub fallback_rpc_urls: Vec<String>,

    /// Private key for the facilitator signer (hex, with or without `0x` prefix).
    /// Supports `$VAR` / `${VAR}` for environment variable expansion.
    ///
    /// For backward compatibility, this field is used when `signer_private_keys`
    /// is empty. Prefer `signer_private_keys` for multi-signer setups.
    #[serde(default)]
    pub signer_private_key: String,

    /// Multiple signer private keys for round-robin transaction submission.
    /// Each key is a hex string (with or without `0x` prefix) and supports
    /// `$VAR` / `${VAR}` environment variable expansion.
    ///
    /// When populated, `signer_private_key` is ignored.
    #[serde(default)]
    pub signer_private_keys: Vec<String>,

    /// Per-chain HTTP request timeout in seconds (default: 30).
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,

    /// Whether to verify chain connectivity at startup by calling
    /// `eth_chainId` (default: `true`).
    #[serde(default = "default_true")]
    pub health_check: bool,

    /// Whether the chain supports EIP-1559 gas pricing (default: `true`).
    #[serde(default = "default_true")]
    pub eip1559: bool,

    /// Whether the chain uses flashblocks for immediate finality (default: `false`).
    #[serde(default)]
    pub flashblocks: bool,

    /// Seconds to wait for a transaction receipt after submission (default: 30).
    #[serde(default = "default_timeout")]
    pub receipt_timeout_secs: u64,
}

impl ChainConfig {
    /// Returns the effective list of signer private keys.
    ///
    /// If `signer_private_keys` is non-empty, returns that list.
    /// Otherwise falls back to the single `signer_private_key`.
    #[must_use]
    pub fn effective_signer_keys(&self) -> Vec<&str> {
        if !self.signer_private_keys.is_empty() {
            self.signer_private_keys
                .iter()
                .map(String::as_str)
                .collect()
        } else if !self.signer_private_key.is_empty() {
            vec![self.signer_private_key.as_str()]
        } else {
            vec![]
        }
    }
}

const fn default_timeout() -> u64 {
    30
}

const fn default_true() -> bool {
    true
}

const fn default_host() -> IpAddr {
    IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
}

const fn default_port() -> u16 {
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
        if let Ok(host) = std::env::var("HOST")
            && let Ok(addr) = host.parse()
        {
            config.host = addr;
        }
        if let Ok(port) = std::env::var("PORT")
            && let Ok(p) = port.parse()
        {
            config.port = p;
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
