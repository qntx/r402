//! REST client abstraction for TON preflight reads and settlement.

use crate::chain::TvmAddress;
use crate::codecs::w5::StateInitCells;

/// Account status bits needed by W5 / Highload preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvmAccountState {
    /// Raw address.
    pub address: TvmAddress,
    /// Native TON balance in nanotons.
    pub balance: u128,
    /// `status == active`.
    pub is_active: bool,
    /// `status == uninit` or `nonexist`.
    pub is_uninitialized: bool,
    /// `status == frozen`.
    pub is_frozen: bool,
    /// Code + data when the account is active.
    pub state_init: Option<StateInitCells>,
}

/// TEP-74 jetton wallet getter result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TvmJettonWalletData {
    /// Jetton wallet address.
    pub address: TvmAddress,
    /// Jetton balance in atomic units.
    pub balance: u128,
    /// Wallet owner.
    pub owner: TvmAddress,
    /// Jetton master.
    pub jetton_minter: TvmAddress,
}

/// Errors from TON REST reads and submits.
#[derive(Debug, thiserror::Error)]
pub enum TvmRpcError {
    /// Transport or HTTP failure.
    #[error("tvm rpc error: {0}")]
    Rpc(String),
    /// Response JSON could not be parsed.
    #[error("tvm rpc parse error: {0}")]
    Parse(String),
}

/// REST provider selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TvmProviderKind {
    /// Toncenter REST (`/api/v3`, `/api/emulate`).
    Toncenter,
    /// TonAPI REST (`/v2`).
    Tonapi,
}

impl TvmProviderKind {
    /// Parses a provider name (`toncenter` / `tonapi`).
    ///
    /// # Errors
    ///
    /// Returns [`TvmRpcError::Parse`] if `name` is not a known provider.
    pub fn parse(name: &str) -> Result<Self, TvmRpcError> {
        match name.trim().to_ascii_lowercase().as_str() {
            "toncenter" => Ok(Self::Toncenter),
            "tonapi" => Ok(Self::Tonapi),
            other => Err(TvmRpcError::Parse(format!(
                "Unsupported TVM provider: {other}"
            ))),
        }
    }
}

/// Read/write TON RPC operations used by the client and facilitator.
pub trait TvmRpc: Send + Sync {
    /// Account state, including code/data BoCs when active.
    fn get_account_state(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmAccountState, TvmRpcError>> + Send;

    /// `get_wallet_address` on the jetton minter.
    fn get_jetton_wallet(
        &self,
        asset: &str,
        owner: &str,
    ) -> impl Future<Output = Result<TvmAddress, TvmRpcError>> + Send;

    /// `get_wallet_data` on a jetton wallet.
    fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmJettonWalletData, TvmRpcError>> + Send;

    /// Emulate an external message and return a Toncenter-shaped trace.
    fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send;

    /// Broadcast an external message. Returns `message_hash_norm`.
    fn send_message(&self, boc: &[u8]) -> impl Future<Output = Result<String, TvmRpcError>> + Send;

    /// Fetch a trace by external message hash.
    fn get_trace_by_message_hash(
        &self,
        message_hash: &str,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send;
}

impl<T: TvmRpc + Send + Sync> TvmRpc for &T {
    fn get_account_state(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmAccountState, TvmRpcError>> + Send {
        (**self).get_account_state(address)
    }

    fn get_jetton_wallet(
        &self,
        asset: &str,
        owner: &str,
    ) -> impl Future<Output = Result<TvmAddress, TvmRpcError>> + Send {
        (**self).get_jetton_wallet(asset, owner)
    }

    fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmJettonWalletData, TvmRpcError>> + Send {
        (**self).get_jetton_wallet_data(address)
    }

    fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send {
        (**self).emulate_trace(boc, ignore_chksig, timeout_seconds)
    }

    fn send_message(&self, boc: &[u8]) -> impl Future<Output = Result<String, TvmRpcError>> + Send {
        (**self).send_message(boc)
    }

    fn get_trace_by_message_hash(
        &self,
        message_hash: &str,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send {
        (**self).get_trace_by_message_hash(message_hash)
    }
}

impl<T: TvmRpc + Send + Sync> TvmRpc for std::sync::Arc<T> {
    fn get_account_state(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmAccountState, TvmRpcError>> + Send {
        (**self).get_account_state(address)
    }

    fn get_jetton_wallet(
        &self,
        asset: &str,
        owner: &str,
    ) -> impl Future<Output = Result<TvmAddress, TvmRpcError>> + Send {
        (**self).get_jetton_wallet(asset, owner)
    }

    fn get_jetton_wallet_data(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<TvmJettonWalletData, TvmRpcError>> + Send {
        (**self).get_jetton_wallet_data(address)
    }

    fn emulate_trace(
        &self,
        boc: &[u8],
        ignore_chksig: bool,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send {
        (**self).emulate_trace(boc, ignore_chksig, timeout_seconds)
    }

    fn send_message(&self, boc: &[u8]) -> impl Future<Output = Result<String, TvmRpcError>> + Send {
        (**self).send_message(boc)
    }

    fn get_trace_by_message_hash(
        &self,
        message_hash: &str,
    ) -> impl Future<Output = Result<serde_json::Value, TvmRpcError>> + Send {
        (**self).get_trace_by_message_hash(message_hash)
    }
}
