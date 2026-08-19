//! Per-fee-payer settlement queue.
//!
//! Each fee-payer account has its own `mpsc` worker so blocks are submitted
//! sequentially against that account's DAG, while different payers run in
//! parallel. Duplicate in-flight blocks (`Block::hash`) are rejected.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use keetanetwork_block::{AccountRef, Block, BlockHash, Hashable};
use keetanetwork_client::{Network, TransmitOptions, UserClient};
use r402_core::facilitator::BoxFuture;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Queue error returned to settle.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    /// The same block hash is already in flight.
    #[error("duplicate_block")]
    Duplicate,
    /// No worker exists for the requested fee payer.
    #[error("unknown fee payer: {0}")]
    UnknownFeePayer(String),
    /// Transmit failed.
    #[error("{0}")]
    Transmit(String),
}

struct Job {
    block: Block,
    reply: Option<oneshot::Sender<Result<String, String>>>,
    pending: Arc<Mutex<HashSet<BlockHash>>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        lock(&self.pending).remove(&self.block.hash());
    }
}

/// Removes `hash` from `pending` unless the job was handed to a worker.
struct EnqueueGuard {
    pending: Arc<Mutex<HashSet<BlockHash>>>,
    hash: BlockHash,
    transferred: bool,
}

impl Drop for EnqueueGuard {
    fn drop(&mut self) {
        if !self.transferred {
            lock(&self.pending).remove(&self.hash);
        }
    }
}

type SubmitFn =
    Arc<dyn Fn(String, Block) -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

enum Backend {
    Live(Network),
    Mock(SubmitFn),
}

/// Serializes settle per fee payer and fans out across payers.
pub struct SettlementQueue {
    senders: Mutex<HashMap<String, mpsc::Sender<Job>>>,
    pending: Arc<Mutex<HashSet<BlockHash>>>,
    fee_payers: Vec<(String, Option<AccountRef>)>,
    next: AtomicUsize,
    backend: Backend,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl std::fmt::Debug for SettlementQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettlementQueue")
            .field(
                "fee_payers",
                &self
                    .fee_payers
                    .iter()
                    .map(|(addr, _)| addr.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl SettlementQueue {
    /// Creates a queue that transmits through `UserClient` for `network`.
    #[must_use]
    pub fn new(fee_payers: Vec<AccountRef>, network: Network) -> Self {
        let mapped = fee_payers
            .into_iter()
            .map(|account| (account.to_string(), Some(account)))
            .collect();
        Self::from_parts(mapped, Backend::Live(network))
    }

    /// Creates a queue that invokes `submit` instead of the network.
    #[must_use]
    pub fn mock<F>(fee_payer_addrs: Vec<String>, submit: F) -> Self
    where
        F: Fn(String, Block) -> BoxFuture<'static, Result<String, String>> + Send + Sync + 'static,
    {
        let mapped = fee_payer_addrs
            .into_iter()
            .map(|addr| (addr, None))
            .collect();
        Self::from_parts(mapped, Backend::Mock(Arc::new(submit)))
    }

    fn from_parts(fee_payers: Vec<(String, Option<AccountRef>)>, backend: Backend) -> Self {
        Self {
            senders: Mutex::new(HashMap::new()),
            pending: Arc::new(Mutex::new(HashSet::new())),
            fee_payers,
            next: AtomicUsize::new(0),
            backend,
            handles: Mutex::new(Vec::new()),
        }
    }

    /// Round-robin fee payer address, if any are configured.
    #[must_use]
    pub fn pick_fee_payer(&self) -> Option<String> {
        if self.fee_payers.is_empty() {
            return None;
        }
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.fee_payers.len();
        self.fee_payers.get(idx).map(|(addr, _)| addr.clone())
    }

    /// Enqueues `block` for sequential transmit on `fee_payer`.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Duplicate`] when the block hash is already
    /// in-flight, [`QueueError::UnknownFeePayer`] when `fee_payer` is not
    /// managed by this queue, or [`QueueError::Transmit`] when the node
    /// rejects the staple.
    pub async fn enqueue(&self, fee_payer: &str, block: Block) -> Result<String, QueueError> {
        let hash = block.hash();
        {
            let mut pending = lock(&self.pending);
            if pending.contains(&hash) {
                return Err(QueueError::Duplicate);
            }
            pending.insert(hash);
        }

        let mut guard = EnqueueGuard {
            pending: Arc::clone(&self.pending),
            hash,
            transferred: false,
        };

        let sender = self.worker(fee_payer)?;

        let (reply_tx, reply_rx) = oneshot::channel();
        let job = Job {
            block,
            reply: Some(reply_tx),
            pending: Arc::clone(&self.pending),
        };
        if sender.send(job).await.is_err() {
            return Err(QueueError::Transmit("settlement worker stopped".to_owned()));
        }
        guard.transferred = true;

        reply_rx
            .await
            .unwrap_or_else(|_| Err("settlement worker dropped".to_owned()))
            .map_err(QueueError::Transmit)
    }

    fn worker(&self, fee_payer: &str) -> Result<mpsc::Sender<Job>, QueueError> {
        let mut senders = lock(&self.senders);
        if let Some(existing) = senders.get(fee_payer) {
            return Ok(existing.clone());
        }
        let account = self
            .fee_payers
            .iter()
            .find(|(addr, _)| addr == fee_payer)
            .map(|(_, account)| account.as_ref().map(Arc::clone))
            .ok_or_else(|| QueueError::UnknownFeePayer(fee_payer.to_owned()))?;
        let (tx, rx) = mpsc::channel(32);
        let addr = fee_payer.to_owned();
        let handle = match &self.backend {
            Backend::Live(network) => {
                let Some(account) = account else {
                    return Err(QueueError::UnknownFeePayer(addr));
                };
                let network = *network;
                tokio::spawn(async move {
                    live_worker(rx, account, network).await;
                })
            }
            Backend::Mock(submit) => {
                let submit = Arc::clone(submit);
                tokio::spawn(async move {
                    mock_worker(rx, addr, submit).await;
                })
            }
        };
        lock(&self.handles).push(handle);
        senders.insert(fee_payer.to_owned(), tx.clone());
        drop(senders);
        Ok(tx)
    }
}

impl Drop for SettlementQueue {
    fn drop(&mut self) {
        lock(&self.senders).clear();
        for handle in lock(&self.handles).drain(..) {
            handle.abort();
        }
    }
}

struct DestroyOnDrop(UserClient);

impl Drop for DestroyOnDrop {
    fn drop(&mut self) {
        self.0.client().destroy();
    }
}

fn complete_job(mut job: Job, result: Result<String, String>) {
    lock(&job.pending).remove(&job.block.hash());
    if let Some(reply) = job.reply.take() {
        drop(reply.send(result));
    }
}

async fn live_worker(mut rx: mpsc::Receiver<Job>, fee_payer: AccountRef, network: Network) {
    let Ok(client) = UserClient::from_network(network, Some(Arc::clone(&fee_payer))) else {
        while let Some(job) = rx.recv().await {
            complete_job(job, Err("failed to bind fee-payer UserClient".to_owned()));
        }
        return;
    };
    let client = DestroyOnDrop(client);
    while let Some(job) = rx.recv().await {
        let opts = TransmitOptions::default().with_fee_signer(&fee_payer);
        let result = match client
            .0
            .transmit(std::slice::from_ref(&job.block), opts)
            .await
        {
            Ok(true) => Ok(job.block.hash().to_string()),
            Ok(false) => Err("transaction_failed".to_owned()),
            Err(err) => Err(err.to_string()),
        };
        complete_job(job, result);
    }
}

async fn mock_worker(mut rx: mpsc::Receiver<Job>, addr: String, submit: SubmitFn) {
    while let Some(job) = rx.recv().await {
        let result = submit(addr.clone(), job.block.clone()).await;
        complete_job(job, result);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
