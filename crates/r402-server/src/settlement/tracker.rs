//! In-flight counter for background settlement tasks.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

/// Shared in-flight counter for background settlement tasks.
///
/// Cloning the tracker is cheap and shares state. Drain at shutdown via
/// [`Self::wait_for_drain`].
#[derive(Clone, Debug)]
pub struct BackgroundSettlementTracker {
    inner: Arc<TrackerInner>,
}

#[derive(Debug)]
struct TrackerInner {
    in_flight: AtomicUsize,
    drained: Notify,
}

impl Default for BackgroundSettlementTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundSettlementTracker {
    /// Constructs a tracker with zero in-flight tasks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TrackerInner {
                in_flight: AtomicUsize::new(0),
                drained: Notify::new(),
            }),
        }
    }

    /// Returns the current approximate number of in-flight settlement tasks.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.inner.in_flight.load(Ordering::SeqCst)
    }

    /// Increments the in-flight counter and returns a guard that decrements it on drop.
    pub(crate) fn start(&self) -> SettlementInFlightGuard {
        let _previous = self.inner.in_flight.fetch_add(1, Ordering::SeqCst);
        SettlementInFlightGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Awaits the in-flight count to reach zero, bounded by `timeout`.
    ///
    /// # Errors
    ///
    /// Returns the count of in-flight tasks when the timeout elapses
    /// before the drain completes.
    pub async fn wait_for_drain(&self, timeout: Duration) -> Result<(), usize> {
        if self.in_flight() == 0 {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.inner.drained.notified();
            tokio::pin!(notified);
            tokio::select! {
                () = &mut notified => {}
                () = tokio::time::sleep_until(deadline) => {
                    let remaining = self.in_flight();
                    return if remaining == 0 { Ok(()) } else { Err(remaining) };
                }
            }
            if self.in_flight() == 0 {
                return Ok(());
            }
        }
    }
}

/// Drop-guard returned by [`BackgroundSettlementTracker::start`].
#[derive(Debug)]
pub(crate) struct SettlementInFlightGuard {
    inner: Arc<TrackerInner>,
}

impl Drop for SettlementInFlightGuard {
    fn drop(&mut self) {
        let previous = self.inner.in_flight.fetch_sub(1, Ordering::SeqCst);
        if previous == 1 {
            self.inner.drained.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_tracker_drains_immediately() {
        let tracker = BackgroundSettlementTracker::new();
        assert_eq!(tracker.in_flight(), 0);
        tracker.wait_for_drain(Duration::ZERO).await.unwrap();
    }

    #[tokio::test]
    async fn drain_waits_for_guard_drop() {
        let tracker = BackgroundSettlementTracker::new();
        let guard = tracker.start();
        assert_eq!(tracker.in_flight(), 1);

        let tracker_clone = tracker.clone();
        let drop_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            drop(guard);
            assert_eq!(tracker_clone.in_flight(), 0);
        });

        tracker
            .wait_for_drain(Duration::from_secs(1))
            .await
            .expect("drain should complete after the guard drops");
        drop_task.await.unwrap();
    }

    #[tokio::test]
    async fn drain_times_out_when_guards_outlive_deadline() {
        let tracker = BackgroundSettlementTracker::new();
        let _guard = tracker.start();

        let result = tracker.wait_for_drain(Duration::from_millis(20)).await;
        assert_eq!(result, Err(1), "deadline elapses with the guard alive");
    }

    #[tokio::test]
    async fn nested_guards_decrement_in_order() {
        let tracker = BackgroundSettlementTracker::new();
        let g1 = tracker.start();
        let g2 = tracker.start();
        let g3 = tracker.start();
        assert_eq!(tracker.in_flight(), 3);
        drop(g2);
        assert_eq!(tracker.in_flight(), 2);
        drop(g1);
        assert_eq!(tracker.in_flight(), 1);
        drop(g3);
        assert_eq!(tracker.in_flight(), 0);
    }
}
