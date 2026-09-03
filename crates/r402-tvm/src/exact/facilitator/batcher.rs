//! Per-network Highload V3 settlement batcher.

#![allow(
    clippy::excessive_nesting,
    clippy::significant_drop_tightening,
    clippy::let_underscore_must_use,
    clippy::type_complexity,
    clippy::large_enum_variant,
    clippy::manual_let_else,
    clippy::needless_continue,
    clippy::indexing_slicing,
    clippy::shadow_unrelated,
    reason = "per-network worker + mock flush backend"
)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

#[cfg(test)]
use r402_facilitator::BoxFuture;
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;

use crate::chain::codec::w5::ParsedTvmSettlement;
use crate::chain::{TvmChainProvider, TvmRelayRequest};
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
    resolve: Option<oneshot::Sender<BatchResult>>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(tx) = self.resolve.take() {
            let _ = tx.send(fail("batcher dropped"));
        }
    }
}

struct NetworkSlot {
    queue: VecDeque<Pending>,
    notify: Arc<Notify>,
    started: bool,
    handle: Option<JoinHandle<()>>,
}

impl Default for NetworkSlot {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            notify: Arc::new(Notify::new()),
            started: false,
            handle: None,
        }
    }
}

enum FlushBackend {
    Live {
        provider: TvmChainProvider,
        verifier: Arc<
            dyn Fn(&serde_json::Value, &ParsedTvmSettlement) -> Result<String, String>
                + Send
                + Sync,
        >,
    },
    #[cfg(test)]
    Mock(
        Arc<
            dyn Fn(Vec<QueuedSettlement>) -> BoxFuture<'static, Result<serde_json::Value, String>>
                + Send
                + Sync,
        >,
    ),
}

/// Batches Highload V3 relays: flush at 100 items or after 1s, max 185.
#[derive(Clone)]
pub struct SettlementBatcher {
    inner: Arc<SettlementBatcherInner>,
}

struct SettlementBatcherInner {
    flush_interval: Duration,
    batch_flush_size: usize,
    confirmation_timeout_seconds: u64,
    shutdown: Arc<Notify>,
    networks: Mutex<HashMap<String, NetworkSlot>>,
    backend: FlushBackend,
}

impl Drop for SettlementBatcherInner {
    fn drop(&mut self) {
        self.shutdown.notify_waiters();
    }
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
        Self::from_backend(
            flush_interval_seconds,
            batch_flush_size,
            confirmation_timeout_seconds,
            FlushBackend::Live {
                provider,
                verifier: Arc::new(verifier),
            },
        )
    }

    fn from_backend(
        flush_interval_seconds: Option<u64>,
        batch_flush_size: Option<usize>,
        confirmation_timeout_seconds: Option<u64>,
        backend: FlushBackend,
    ) -> Self {
        Self {
            inner: Arc::new(SettlementBatcherInner {
                flush_interval: Duration::from_secs(
                    flush_interval_seconds
                        .unwrap_or(DEFAULT_SETTLEMENT_BATCH_FLUSH_INTERVAL_SECONDS),
                ),
                batch_flush_size: batch_flush_size
                    .unwrap_or(DEFAULT_SETTLEMENT_BATCH_FLUSH_SIZE)
                    .min(DEFAULT_SETTLEMENT_BATCH_MAX_SIZE),
                confirmation_timeout_seconds: confirmation_timeout_seconds
                    .unwrap_or(DEFAULT_TRACE_CONFIRMATION_TIMEOUT_SECONDS),
                shutdown: Arc::new(Notify::new()),
                networks: Mutex::new(HashMap::new()),
                backend,
            }),
        }
    }

    /// Enqueues `item` and returns when its batch has been sent and verified.
    pub async fn enqueue(&self, network: String, item: QueuedSettlement) -> BatchResult {
        let (tx, rx) = oneshot::channel();
        let notify;
        let need_spawn;
        {
            let mut map = match self.inner.networks.lock() {
                Ok(g) => g,
                Err(_) => return fail("batcher lock poisoned"),
            };
            let slot = map.entry(network.clone()).or_default();
            slot.queue.push_back(Pending {
                queued: item,
                resolve: Some(tx),
            });
            notify = Arc::clone(&slot.notify);
            need_spawn = !slot.started;
            if need_spawn {
                slot.started = true;
            }
        }
        if need_spawn {
            let weak = Arc::downgrade(&self.inner);
            let spawned_network = network.clone();
            let handle = tokio::spawn(async move { worker(weak, spawned_network).await });
            if let Ok(mut map) = self.inner.networks.lock()
                && let Some(slot) = map.get_mut(&network)
            {
                slot.handle = Some(handle);
            }
        }
        notify.notify_one();
        rx.await.unwrap_or_else(|_| fail("batcher dropped"))
    }
}

fn queue_len(inner: &SettlementBatcherInner, network: &str) -> usize {
    inner
        .networks
        .lock()
        .ok()
        .and_then(|map| map.get(network).map(|slot| slot.queue.len()))
        .unwrap_or(0)
}

fn slot_notify(inner: &SettlementBatcherInner, network: &str) -> Option<Arc<Notify>> {
    inner
        .networks
        .lock()
        .ok()
        .and_then(|map| map.get(network).map(|slot| Arc::clone(&slot.notify)))
}

async fn worker(weak: Weak<SettlementBatcherInner>, network: String) {
    loop {
        let Some(mut inner) = weak.upgrade() else {
            return;
        };
        let Some(work_n) = slot_notify(&inner, &network) else {
            return;
        };
        let shutdown_n = Arc::clone(&inner.shutdown);
        let flush_interval = inner.flush_interval;
        let batch_flush_size = inner.batch_flush_size;

        let inner = loop {
            let work_wait = work_n.notified();
            let stop_wait = shutdown_n.notified();
            if queue_len(&inner, &network) > 0 {
                break inner;
            }
            drop(inner);
            tokio::select! {
                () = work_wait => {}
                () = stop_wait => return,
            }
            let Some(again) = weak.upgrade() else {
                return;
            };
            inner = again;
        };

        run_deadline_and_flush(
            inner,
            &network,
            &work_n,
            &shutdown_n,
            flush_interval,
            batch_flush_size,
        )
        .await;
    }
}

async fn run_deadline_and_flush(
    inner: Arc<SettlementBatcherInner>,
    network: &str,
    work_n: &Notify,
    shutdown_n: &Notify,
    flush_interval: Duration,
    batch_flush_size: usize,
) {
    let deadline = tokio::time::Instant::now() + flush_interval;
    loop {
        let work_wait = work_n.notified();
        let len = queue_len(&inner, network);
        if len == 0 {
            return;
        }
        if len >= batch_flush_size {
            break;
        }
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => break,
            () = work_wait => continue,
            () = shutdown_n.notified() => return,
        }
    }
    if queue_len(&inner, network) > 0 {
        flush_network(&inner, network).await;
    }
}

async fn flush_network(inner: &SettlementBatcherInner, network: &str) {
    let (batch, remainder_notify): (Vec<Pending>, Option<Arc<Notify>>) = {
        let mut map = match inner.networks.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(slot) = map.get_mut(network) else {
            return;
        };
        let take = slot.queue.len().min(DEFAULT_SETTLEMENT_BATCH_MAX_SIZE);
        let batch: Vec<Pending> = slot.queue.drain(..take).collect();
        let remainder_notify = if slot.queue.is_empty() {
            None
        } else {
            Some(Arc::clone(&slot.notify))
        };
        (batch, remainder_notify)
    };
    if batch.is_empty() {
        return;
    }
    match &inner.backend {
        FlushBackend::Live { provider, verifier } => {
            let relays: Vec<TvmRelayRequest> = batch
                .iter()
                .map(|p| p.queued.relay_request.clone())
                .collect();
            let send_result = async {
                let boc = provider
                    .build_relay_external_boc_batch(&relays, false)
                    .await?;
                let hash = provider.send_external_message(&boc).await?;
                provider
                    .wait_for_trace_confirmation(&hash, inner.confirmation_timeout_seconds)
                    .await
            }
            .await;
            match send_result {
                Ok(trace) => {
                    for mut pending in batch {
                        let result = match verifier(&trace, &pending.queued.settlement) {
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
                        if let Some(tx) = pending.resolve.take() {
                            let _ = tx.send(result);
                        }
                    }
                }
                Err(err) => {
                    for mut pending in batch {
                        if let Some(tx) = pending.resolve.take() {
                            let _ = tx.send(fail(&err.to_string()));
                        }
                    }
                }
            }
        }
        #[cfg(test)]
        FlushBackend::Mock(flush) => {
            let items: Vec<QueuedSettlement> = batch
                .iter()
                .map(|p| QueuedSettlement {
                    settlement_hash: p.queued.settlement_hash.clone(),
                    settlement: p.queued.settlement.clone(),
                    relay_request: p.queued.relay_request.clone(),
                })
                .collect();
            match flush(items).await {
                Ok(_) => {
                    for mut pending in batch {
                        let hash = pending.queued.settlement_hash.clone();
                        if let Some(tx) = pending.resolve.take() {
                            let _ = tx.send(BatchResult {
                                success: true,
                                transaction: Some(hash),
                                error_reason: None,
                                error_message: None,
                            });
                        }
                    }
                }
                Err(err) => {
                    for mut pending in batch {
                        if let Some(tx) = pending.resolve.take() {
                            let _ = tx.send(fail(&err));
                        }
                    }
                }
            }
        }
    }
    if let Some(notify) = remainder_notify {
        notify.notify_one();
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

#[cfg(test)]
impl SettlementBatcher {
    fn mock(
        flush_interval: Duration,
        batch_flush_size: usize,
        flush: impl Fn(Vec<QueuedSettlement>) -> BoxFuture<'static, Result<serde_json::Value, String>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self::from_backend(
            Some(flush_interval.as_secs().max(1)),
            Some(batch_flush_size),
            None,
            FlushBackend::Mock(Arc::new(flush)),
        )
        .with_flush_interval(flush_interval)
    }

    fn with_flush_interval(self, interval: Duration) -> Self {
        // `from_backend` stores seconds; tests need sub-second idle.
        // Rebuild inner is impossible through Arc; set via a test-only field.
        // The constructor above used max(1) seconds — override the Duration directly
        // by swapping the Arc (tests own the only clone).
        let inner = Arc::try_unwrap(self.inner).unwrap_or_else(|arc| (*arc).clone_inner_for_test());
        inner_set_interval(inner, interval)
    }
}

#[cfg(test)]
impl SettlementBatcherInner {
    fn clone_inner_for_test(&self) -> Self {
        Self {
            flush_interval: self.flush_interval,
            batch_flush_size: self.batch_flush_size,
            confirmation_timeout_seconds: self.confirmation_timeout_seconds,
            shutdown: Arc::clone(&self.shutdown),
            networks: Mutex::new(HashMap::new()),
            backend: match &self.backend {
                FlushBackend::Live { .. } => unreachable!("test mock only"),
                FlushBackend::Mock(f) => FlushBackend::Mock(Arc::clone(f)),
            },
        }
    }
}

#[cfg(test)]
fn inner_set_interval(mut inner: SettlementBatcherInner, interval: Duration) -> SettlementBatcher {
    inner.flush_interval = interval;
    SettlementBatcher {
        inner: Arc::new(inner),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "batcher unit tests"
)]
mod tests {
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tonlib_core::cell::{Cell, CellBuilder};
    use tonlib_core::types::TonHash;

    use super::*;
    use crate::chain::TvmAddress;
    use crate::chain::codec::jetton::ParsedJettonTransfer;
    use crate::chain::codec::w5::ParsedTvmSettlement;

    fn empty_cell() -> Cell {
        CellBuilder::new().build().expect("empty cell")
    }

    fn dummy_item(hash: &str) -> QueuedSettlement {
        let cell = empty_cell();
        let addr: TvmAddress = "0:0000000000000000000000000000000000000000000000000000000000000000"
            .parse()
            .expect("zero tvm address");
        let ton_hash = TonHash::from([0u8; 32]);
        QueuedSettlement {
            settlement_hash: hash.to_owned(),
            settlement: ParsedTvmSettlement {
                payer: addr.clone(),
                wallet_id: 0,
                valid_until: 0,
                seqno: 0,
                settlement_hash: hash.to_owned(),
                body: cell.clone(),
                signed_slice_hash: ton_hash.clone(),
                signature: [0u8; 64],
                state_init: None,
                transfer: ParsedJettonTransfer {
                    source_wallet: addr.clone(),
                    destination: addr.clone(),
                    response_destination: None,
                    jetton_amount: 0,
                    attached_ton_amount: 0,
                    forward_ton_amount: 0,
                    forward_payload: cell.clone(),
                    body_hash: ton_hash,
                },
            },
            relay_request: TvmRelayRequest {
                destination: addr,
                body: cell,
                state_init: None,
                forward_ton_amount: 0,
                relay_amount: None,
            },
        }
    }

    fn instant_ok_flush()
    -> impl Fn(Vec<QueuedSettlement>) -> BoxFuture<'static, Result<serde_json::Value, String>> {
        |_items| Box::pin(async { Ok(serde_json::json!({})) })
    }

    #[tokio::test]
    async fn idle_flush_after_single_item() {
        let batcher = SettlementBatcher::mock(Duration::from_millis(30), 100, instant_ok_flush());
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            batcher.enqueue("tvm:-3".into(), dummy_item("a")),
        )
        .await
        .expect("idle flush timed out");
        assert!(result.success, "{result:?}");
        assert_eq!(result.transaction.as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn two_items_below_size_wait_for_idle() {
        let flushes = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
        let flushes_cb = Arc::clone(&flushes);
        let batcher = SettlementBatcher::mock(Duration::from_millis(40), 100, move |items| {
            let hashes: Vec<String> = items.iter().map(|i| i.settlement_hash.clone()).collect();
            flushes_cb.lock().unwrap().push(hashes);
            Box::pin(async { Ok(serde_json::json!({})) })
        });
        let a = tokio::spawn({
            let b = batcher.clone();
            async move { b.enqueue("n".into(), dummy_item("1")).await }
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let b = tokio::spawn({
            let b = batcher.clone();
            async move { b.enqueue("n".into(), dummy_item("2")).await }
        });
        let ra = a.await.unwrap();
        let rb = b.await.unwrap();
        assert!(ra.success && rb.success, "{ra:?} {rb:?}");
        let recorded = flushes.lock().unwrap().clone();
        assert_eq!(
            recorded.len(),
            1,
            "expected one idle flush, got {recorded:?}"
        );
        assert_eq!(recorded[0].len(), 2);
    }

    #[tokio::test]
    async fn concurrent_size_and_idle_do_not_double_send() {
        let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
        let seen_cb = Arc::clone(&seen);
        let batcher = SettlementBatcher::mock(Duration::from_millis(20), 2, move |items| {
            for item in &items {
                seen_cb.lock().unwrap().push(item.settlement_hash.clone());
            }
            Box::pin(async { Ok(serde_json::json!({})) })
        });
        let mut joins = Vec::new();
        for hash in ["a", "b", "c", "d"] {
            let b = batcher.clone();
            joins.push(tokio::spawn(async move {
                b.enqueue("n".into(), dummy_item(hash)).await
            }));
        }
        for j in joins {
            assert!(j.await.unwrap().success);
        }
        let mut hashes = seen.lock().unwrap().clone();
        hashes.sort();
        hashes.dedup();
        assert_eq!(hashes, vec!["a", "b", "c", "d"]);
        assert_eq!(seen.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn size_flush_does_not_abort_idle_send() {
        let first_started = Arc::new(Notify::new());
        let first_release = Arc::new(Notify::new());
        let flush_count = Arc::new(AtomicUsize::new(0));
        let first_started_cb = Arc::clone(&first_started);
        let first_release_cb = Arc::clone(&first_release);
        let flush_count_cb = Arc::clone(&flush_count);
        let batcher = SettlementBatcher::mock(Duration::from_millis(20), 100, move |items| {
            let n = flush_count_cb.fetch_add(1, Ordering::SeqCst);
            let started = Arc::clone(&first_started_cb);
            let release = Arc::clone(&first_release_cb);
            Box::pin(async move {
                if n == 0 {
                    started.notify_one();
                    release.notified().await;
                }
                let _ = items;
                Ok(serde_json::json!({}))
            })
        });
        let first = tokio::spawn({
            let b = batcher.clone();
            async move { b.enqueue("n".into(), dummy_item("idle")).await }
        });
        first_started.notified().await;
        let mut rest = Vec::new();
        for i in 0..100 {
            let b = batcher.clone();
            rest.push(tokio::spawn(async move {
                b.enqueue("n".into(), dummy_item(&format!("s{i}"))).await
            }));
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        first_release.notify_waiters();
        let idle = first.await.unwrap();
        assert!(idle.success, "idle flush aborted: {idle:?}");
        assert_ne!(idle.error_message.as_deref(), Some("batcher dropped"));
        for j in rest {
            let r = j.await.unwrap();
            assert!(r.success, "{r:?}");
            assert_ne!(r.error_message.as_deref(), Some("batcher dropped"));
        }
    }

    #[tokio::test]
    async fn drop_during_send_resolves_oneshots() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let started_cb = Arc::clone(&started);
        let release_cb = Arc::clone(&release);
        let batcher = SettlementBatcher::mock(Duration::from_millis(10), 100, move |items| {
            let started = Arc::clone(&started_cb);
            let release = Arc::clone(&release_cb);
            Box::pin(async move {
                started.notify_one();
                release.notified().await;
                let _ = items;
                Ok(serde_json::json!({}))
            })
        });
        let join = tokio::spawn({
            let b = batcher.clone();
            async move { b.enqueue("n".into(), dummy_item("x")).await }
        });
        started.notified().await;
        drop(batcher);
        release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(1), join)
            .await
            .expect("oneshot hung")
            .unwrap();
        assert!(
            result.success || result.error_message.as_deref() == Some("batcher dropped"),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn remainder_above_max_flushes_again() {
        let flushes = Arc::new(AtomicUsize::new(0));
        let flushes_cb = Arc::clone(&flushes);
        let batcher = SettlementBatcher::mock(Duration::from_millis(20), 185, move |items| {
            flushes_cb.fetch_add(1, Ordering::SeqCst);
            let _ = items;
            Box::pin(async { Ok(serde_json::json!({})) })
        });
        let mut joins = Vec::new();
        for i in 0..=DEFAULT_SETTLEMENT_BATCH_MAX_SIZE {
            let b = batcher.clone();
            joins.push(tokio::spawn(async move {
                b.enqueue("n".into(), dummy_item(&format!("{i}"))).await
            }));
        }
        for j in joins {
            assert!(j.await.unwrap().success);
        }
        assert!(flushes.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn two_first_enqueues_spawn_one_worker() {
        let flushes = Arc::new(AtomicUsize::new(0));
        let flushes_cb = Arc::clone(&flushes);
        let batcher = SettlementBatcher::mock(Duration::from_millis(20), 100, move |items| {
            flushes_cb.fetch_add(1, Ordering::SeqCst);
            let _ = items;
            Box::pin(async { Ok(serde_json::json!({})) })
        });
        let a = tokio::spawn({
            let b = batcher.clone();
            async move { b.enqueue("n".into(), dummy_item("a")).await }
        });
        let b = tokio::spawn({
            let b = batcher.clone();
            async move { b.enqueue("n".into(), dummy_item("b")).await }
        });
        assert!(a.await.unwrap().success);
        assert!(b.await.unwrap().success);
        assert_eq!(flushes.load(Ordering::SeqCst), 1);
    }
}
