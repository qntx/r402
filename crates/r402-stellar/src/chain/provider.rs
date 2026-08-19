//! Facilitator-side Stellar chain provider: signers + RPC.

use std::fmt::Debug;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use r402_core::chain::{ChainId, ChainProvider};
use stellar_rpc_client::{GetTransactionResponse, SimulateTransactionResponse};
use stellar_xdr::{
    FeeBumpTransaction, FeeBumpTransactionExt, FeeBumpTransactionInnerTx, Hash, SequenceNumber,
    TransactionEnvelope, TransactionExt, TransactionV1Envelope,
};

use super::rpc::{StellarJsonRpc, StellarRpc, StellarRpcError};
use super::signer::{StellarSigner, StellarSignerError};
use super::types::StellarChainReference;
use super::xdr::{
    StellarXdrError, encode_transaction_envelope, inner_transaction, muxed_account_from_str,
};
use crate::{BASE_FEE_STROOPS, DEFAULT_MAX_TRANSACTION_FEE_STROOPS};

/// Errors from the Stellar facilitator provider.
#[derive(Debug, thiserror::Error)]
pub enum StellarFacilitatorError {
    /// Identifier or key could not be parsed.
    #[error(transparent)]
    Signer(#[from] StellarSignerError),
    /// RPC construction or request failed.
    #[error(transparent)]
    Rpc(#[from] StellarRpcError),
    /// XDR rebuild failed.
    #[error(transparent)]
    Xdr(#[from] StellarXdrError),
    /// No facilitator signer is configured.
    #[error("no stellar facilitator signer configured")]
    NoSigner,
    /// Fee exceeded `u32` when rebuilding the transaction.
    #[error("stellar transaction fee does not fit in u32")]
    FeeOverflow,
}

/// Provider for interacting with a Stellar network as a facilitator.
#[derive(Debug)]
pub struct StellarChainProvider {
    chain: StellarChainReference,
    signers: Vec<StellarSigner>,
    fee_bump: Option<StellarSigner>,
    rpc: StellarJsonRpc,
    max_transaction_fee_stroops: u32,
    next_signer: AtomicUsize,
}

impl Clone for StellarChainProvider {
    fn clone(&self) -> Self {
        Self {
            chain: self.chain,
            signers: self.signers.clone(),
            fee_bump: self.fee_bump.clone(),
            rpc: self.rpc.clone(),
            max_transaction_fee_stroops: self.max_transaction_fee_stroops,
            next_signer: AtomicUsize::new(self.next_signer.load(Ordering::Relaxed)),
        }
    }
}

impl StellarChainProvider {
    /// Creates a provider for `chain` using the given signers and RPC URL.
    ///
    /// Pubnet requires a non-empty `rpc_url`.
    ///
    /// # Errors
    ///
    /// Returns [`StellarFacilitatorError`] when no signer is provided or the
    /// RPC URL is invalid.
    pub fn new(
        chain: StellarChainReference,
        signers: Vec<StellarSigner>,
        rpc_url: Option<&str>,
    ) -> Result<Self, StellarFacilitatorError> {
        if signers.is_empty() {
            return Err(StellarFacilitatorError::NoSigner);
        }
        Ok(Self {
            chain,
            signers,
            fee_bump: None,
            rpc: StellarJsonRpc::connect(chain, rpc_url)?,
            max_transaction_fee_stroops: DEFAULT_MAX_TRANSACTION_FEE_STROOPS,
            next_signer: AtomicUsize::new(0),
        })
    }

    /// Overrides the Horizon base URL.
    #[must_use]
    pub fn with_horizon_url(mut self, url: impl Into<String>) -> Self {
        self.rpc = self.rpc.with_horizon_url(url);
        self
    }

    /// Overrides the settlement-fee safety ceiling (stroops).
    #[must_use]
    pub const fn with_max_transaction_fee_stroops(mut self, fee: u32) -> Self {
        self.max_transaction_fee_stroops = fee;
        self
    }

    /// Sets an optional fee-bump signer (decouples fees from sequence).
    #[must_use]
    pub fn with_fee_bump_signer(mut self, signer: StellarSigner) -> Self {
        self.fee_bump = Some(signer);
        self
    }

    /// The Stellar network this provider is bound to.
    #[must_use]
    pub const fn chain_reference(&self) -> StellarChainReference {
        self.chain
    }

    /// Safety ceiling used during verify.
    #[must_use]
    pub const fn max_transaction_fee_stroops(&self) -> u32 {
        self.max_transaction_fee_stroops
    }

    /// Shared RPC handle.
    #[must_use]
    pub const fn rpc(&self) -> &StellarJsonRpc {
        &self.rpc
    }

    /// Facilitator G-addresses, including the fee-bump signer when distinct.
    #[must_use]
    pub fn facilitator_addresses(&self) -> Vec<String> {
        let mut addrs: Vec<String> = self
            .signers
            .iter()
            .map(|s| s.address().to_owned())
            .collect();
        if let Some(bump) = &self.fee_bump
            && !addrs.iter().any(|a| a == bump.address())
        {
            addrs.push(bump.address().to_owned());
        }
        addrs
    }

    /// Selects the next inner-transaction signer (round-robin).
    #[must_use]
    pub fn select_signer(&self) -> Option<&StellarSigner> {
        if self.signers.is_empty() {
            return None;
        }
        let index = self.next_signer.fetch_add(1, Ordering::Relaxed) % self.signers.len();
        self.signers.get(index)
    }

    /// Rebuilds the client transaction with this facilitator as source, signs
    /// it, optionally wraps a fee bump, submits, and waits for confirmation.
    ///
    /// # Errors
    ///
    /// Returns [`StellarFacilitatorError`] when rebuild, sign, or submit fails.
    pub async fn settle_envelope(
        &self,
        client_envelope: &TransactionEnvelope,
        sim: &SimulateTransactionResponse,
        max_timeout_seconds: u64,
        now_unix: u64,
    ) -> Result<GetTransactionResponse, StellarFacilitatorError> {
        let signer = self
            .select_signer()
            .ok_or(StellarFacilitatorError::NoSigner)?;
        let account = self.rpc.get_account(signer.address()).await?;
        let seq = SequenceNumber(account.seq_num.0.saturating_add(1));
        let soroban = sim
            .transaction_data()
            .map_err(|e| StellarFacilitatorError::Rpc(StellarRpcError::Rpc(e.to_string())))?;
        let resource_fee = u64::try_from(soroban.resource_fee.max(0)).unwrap_or(0);
        let fee = u32::try_from(u64::from(BASE_FEE_STROOPS).saturating_add(resource_fee))
            .map_err(|_| StellarFacilitatorError::FeeOverflow)?;
        let client_tx = inner_transaction(client_envelope)?;
        let rebuilt = TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: stellar_xdr::Transaction {
                source_account: muxed_account_from_str(signer.address())?,
                fee,
                seq_num: seq,
                cond: timeout_preconditions(max_timeout_seconds, now_unix),
                memo: client_tx.memo.clone(),
                operations: client_tx.operations.clone(),
                ext: TransactionExt::V1(soroban),
            },
            signatures: Vec::new()
                .try_into()
                .map_err(|e: stellar_xdr::Error| StellarXdrError::Xdr(e.to_string()))?,
        });
        let mut rebuilt = rebuilt;
        signer.sign_envelope(&mut rebuilt, self.chain.passphrase())?;

        let to_submit = if let Some(bump) = &self.fee_bump {
            wrap_fee_bump(&rebuilt, bump, self.chain.passphrase())?
        } else {
            rebuilt
        };

        let hash = self.rpc.send_transaction(&to_submit).await?;
        let timeout = Duration::from_secs(max_timeout_seconds.max(1));
        Ok(self.rpc.poll_transaction(&hash, timeout).await?)
    }
}

const fn timeout_preconditions(
    max_timeout_seconds: u64,
    now_unix: u64,
) -> stellar_xdr::Preconditions {
    stellar_xdr::Preconditions::Time(stellar_xdr::TimeBounds {
        min_time: stellar_xdr::TimePoint(0),
        max_time: stellar_xdr::TimePoint(now_unix.saturating_add(max_timeout_seconds)),
    })
}

fn wrap_fee_bump(
    inner: &TransactionEnvelope,
    bump: &StellarSigner,
    passphrase: &str,
) -> Result<TransactionEnvelope, StellarFacilitatorError> {
    let TransactionEnvelope::Tx(inner_v1) = inner else {
        return Err(StellarFacilitatorError::Xdr(StellarXdrError::Shape(
            "fee-bump inner transaction must be a v1 envelope".to_owned(),
        )));
    };
    let inner_fee = i64::from(inner_v1.tx.fee);
    let total_fee = inner_fee.saturating_add(i64::from(BASE_FEE_STROOPS));
    let mut envelope = TransactionEnvelope::from(FeeBumpTransaction {
        fee_source: muxed_account_from_str(bump.address())?,
        fee: total_fee,
        inner_tx: FeeBumpTransactionInnerTx::Tx(inner_v1.clone()),
        ext: FeeBumpTransactionExt::V0,
    });
    bump.sign_envelope(&mut envelope, passphrase)?;
    Ok(envelope)
}

impl ChainProvider for StellarChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        self.facilitator_addresses()
    }

    fn chain_id(&self) -> ChainId {
        self.chain.into()
    }
}

impl StellarRpc for StellarChainProvider {
    fn latest_ledger(&self) -> impl Future<Output = Result<u32, StellarRpcError>> + Send {
        self.rpc.latest_ledger()
    }

    fn get_account(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<stellar_xdr::AccountEntry, StellarRpcError>> + Send {
        self.rpc.get_account(address)
    }

    fn simulate_transaction(
        &self,
        tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<SimulateTransactionResponse, StellarRpcError>> + Send {
        self.rpc.simulate_transaction(tx)
    }

    fn send_transaction(
        &self,
        tx: &TransactionEnvelope,
    ) -> impl Future<Output = Result<Hash, StellarRpcError>> + Send {
        self.rpc.send_transaction(tx)
    }

    fn get_transaction(
        &self,
        hash: &Hash,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send {
        self.rpc.get_transaction(hash)
    }

    fn poll_transaction(
        &self,
        hash: &Hash,
        timeout: Duration,
    ) -> impl Future<Output = Result<GetTransactionResponse, StellarRpcError>> + Send {
        self.rpc.poll_transaction(hash, timeout)
    }

    fn estimated_ledger_seconds(&self) -> impl Future<Output = u64> + Send {
        self.rpc.estimated_ledger_seconds()
    }
}

/// Encodes an envelope; used by tests and diagnostics.
///
/// # Errors
///
/// Returns [`StellarXdrError`] when encoding fails.
pub fn envelope_base64(envelope: &TransactionEnvelope) -> Result<String, StellarXdrError> {
    encode_transaction_envelope(envelope)
}
