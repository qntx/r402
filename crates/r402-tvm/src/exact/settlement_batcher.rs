//! Per-network Highload V3 settlement batcher.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::chain::{TvmChainProvider, TvmRelayRequest};
use crate::codecs::w5::ParsedTvmSettlement;
use crate::exact::error::ERR_EXACT_TVM_TRANSACTION_FAILED;
use crate::{
    DEFAULT_SETTLEMENT_BATCH_FLUSH_INTERVAL_SECONDS, DEFAULT_SETTLEMENT_BATCH_FLUSH_SIZE,
    DEFAULT_SETTLEMENT_BATCH_MAX_SIZE, DEFAULT_TRACE_CONFIRMATION_TIMEOUT_SECONDS,
};

/// Outcome of one batched settlement.
#[derive(Debug, Clone)]
pub struct BatchResult {
    /// Whether the inner jetton transfer landed.
    pub success: bool,
    /// Payer wallet transaction hash (hex).
    pub transaction: Option<String>,
    /// Wire error reason.
    pub error_reason: Option<String>,
    /// Human-readable error.
    pub error_message: Option<String>,
}

/// One queued settlement waiting for a Highload flush.
#[derive(Debug)]
pub struct QueuedSettlement {
    /// Settlement BoC hash (cache key).
    pub settlement_hash: String,
    /// Parsed client payload.
    pub settlement: ParsedTvmSettlement,
    /// Relay request built during verify.
    pub relay_request: TvmRelayRequest,
}

struct Pending {
    queued: QueuedSettlement,
    resolve: oneshot::Sender<BatchResult>,
}

/// Batches Highload V3 relays: flush at 100 items or after 1s, max 185.
#[derive(Clone)]
pub struct SettlementBatcher {
    inner: Arc<SettlementBatcherInner>,
}

struct SettlementBatcherInner {
    provider: TvmChainProvider,
    flush_interval: Duration,
    batch_flush_size: usize,
    confirmation_timeout_seconds: u64,
    queues: Mutex<HashMap<String, VecDeque<Pending>>>,
    timers: Mutex<HashMap<String, tokio::task::AbortHandle>>,
    verifier: Arc<
        dyn Fn(&serde_json::Value, &ParsedTvmSettlement) -> Result<String, String> + Send + Sync,
    >,
}

impl std::fmt::Debug for SettlementBatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettlementBatcher")
            .field("batch_flush_size", &self.inner.batch_flush_size)
            .finish_non_exhaustive()
    }
}

impl SettlementBatcher {
    /// Creates a batcher bound to `provider`.
    pub fn new(
        provider: TvmChainProvider,
        flush_interval_seconds: Option<u64>,
        batch_flush_size: Option<usize>,
        confirmation_timeout_seconds: Option<u64>,
        verifier: impl Fn(&serde_json::Value, &ParsedTvmSettlement) -> Result<String, String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(SettlementBatcherInner {
                provider,
                flush_interval: Duration::from_secs(
                    flush_interval_seconds
                        .unwrap_or(DEFAULT_SETTLEMENT_BATCH_FLUSH_INTERVAL_SECONDS),
                ),
                batch_flush_size: batch_flush_size
                    .unwrap_or(DEFAULT_SETTLEMENT_BATCH_FLUSH_SIZE)
                    .min(DEFAULT_SETTLEMENT_BATCH_MAX_SIZE),
                confirmation_timeout_seconds: confirmation_timeout_seconds
                    .unwrap_or(DEFAULT_TRACE_CONFIRMATION_TIMEOUT_SECONDS),
                queues: Mutex::new(HashMap::new()),
                timers: Mutex::new(HashMap::new()),
                verifier: Arc::new(verifier),
            }),
        }
    }

    /// Enqueues `item` and returns when its batch has been sent and verified.
    pub async fn enqueue(&self, network: String, item: QueuedSettlement) -> BatchResult {
        let (tx, rx) = oneshot::channel();
        let should_flush_now;
        {
            let mut queues = match self.inner.queues.lock() {
                Ok(g) => g,
                Err(_) => {
                    return fail("batcher lock poisoned");
                }
            };
            let queue = queues.entry(network.clone()).or_default();
            queue.push_back(Pending {
                queued: item,
                resolve: tx,
            });
            should_flush_now = queue.len() >= self.inner.batch_flush_size;
        }
        if should_flush_now {
            self.clear_timer(&network);
            self.flush_network(&network).await;
        } else {
            self.arm_timer(network);
        }
        rx.await.unwrap_or_else(|_| fail("batcher dropped"))
    }

    fn arm_timer(&self, network: String) {
        {
            let timers = match self.inner.timers.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if timers.contains_key(&network) {
                return;
            }
        }
        let this = self.clone();
        let interval = self.inner.flush_interval;
        let handle = tokio::spawn({
            let network = network.clone();
            async move {
                tokio::time::sleep(interval).await;
                this.flush_network(&network).await;
            }
        });
        if let Ok(mut timers) = self.inner.timers.lock() {
            timers.insert(network, handle.abort_handle());
        }
    }

    fn clear_timer(&self, network: &str) {
        if let Ok(mut timers) = self.inner.timers.lock()
            && let Some(handle) = timers.remove(network)
        {
            handle.abort();
        }
    }

    async fn flush_network(&self, network: &str) {
        self.clear_timer(network);
        loop {
            let (batch, had_remainder): (Vec<Pending>, bool) = {
                let mut queues = match self.inner.queues.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                let Some(queue) = queues.get_mut(network) else {
                    return;
                };
                let take = queue.len().min(DEFAULT_SETTLEMENT_BATCH_MAX_SIZE);
                let batch = queue.drain(..take).collect();
                let had_remainder = !queue.is_empty();
                if !had_remainder {
                    queues.remove(network);
                }
                (batch, had_remainder)
            };
            if batch.is_empty() {
                return;
            }
            let relays: Vec<TvmRelayRequest> = batch
                .iter()
                .map(|p| p.queued.relay_request.clone())
                .collect();
            let send_result = async {
                let boc = self
                    .inner
                    .provider
                    .build_relay_external_boc_batch(&relays, false)
                    .await?;
                let hash = self.inner.provider.send_external_message(&boc).await?;
                self.inner
                    .provider
                    .wait_for_trace_confirmation(&hash, self.inner.confirmation_timeout_seconds)
                    .await
            }
            .await;
            match send_result {
                Ok(trace) => {
                    for pending in batch {
                        let result = match (self.inner.verifier)(&trace, &pending.queued.settlement)
                        {
                            Ok(transaction) => BatchResult {
                                success: true,
                                transaction: Some(transaction),
                                error_reason: None,
                                error_message: None,
                            },
                            Err(err) => BatchResult {
                                success: false,
                                transaction: None,
                                error_reason: Some(ERR_EXACT_TVM_TRANSACTION_FAILED.to_owned()),
                                error_message: Some(err),
                            },
                        };
                        drop(pending.resolve.send(result));
                    }
                }
                Err(err) => {
                    for pending in batch {
                        drop(pending.resolve.send(fail(&err.to_string())));
                    }
                }
            }
            if !had_remainder {
                return;
            }
        }
    }
}

fn fail(message: &str) -> BatchResult {
    BatchResult {
        success: false,
        transaction: None,
        error_reason: Some(ERR_EXACT_TVM_TRANSACTION_FAILED.to_owned()),
        error_message: Some(message.to_owned()),
    }
}
