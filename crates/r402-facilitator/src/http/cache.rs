//! TTL cache for `GET /supported`.

use std::sync::Arc;
use std::time::Duration;

use r402_protocol::payment::SupportedResponse;
use tokio::sync::RwLock;

/// TTL cache for [`SupportedResponse`].
#[derive(Clone, Debug)]
struct SupportedCacheState {
    response: SupportedResponse,
    expires_at: std::time::Instant,
}

/// An encapsulated TTL cache for the `/supported` endpoint response.
///
/// Clones share the same cache state via `Arc`.
#[derive(Debug, Clone)]
pub struct SupportedCache {
    ttl: Duration,
    state: Arc<RwLock<Option<SupportedCacheState>>>,
}

impl SupportedCache {
    /// Creates a new cache with the given TTL.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            state: Arc::new(RwLock::new(None)),
        }
    }

    /// Returns the cached response if valid, None otherwise.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "read guard scope matches data access"
    )]
    pub async fn get(&self) -> Option<SupportedResponse> {
        let guard = self.state.read().await;
        let cache = guard.as_ref()?;
        if std::time::Instant::now() < cache.expires_at {
            Some(cache.response.clone())
        } else {
            None
        }
    }

    /// Stores a response in the cache with the configured TTL.
    pub async fn set(&self, response: SupportedResponse) {
        let mut guard = self.state.write().await;
        *guard = Some(SupportedCacheState {
            response,
            expires_at: std::time::Instant::now() + self.ttl,
        });
    }

    /// Clears the cache.
    pub async fn clear(&self) {
        let mut guard = self.state.write().await;
        *guard = None;
    }
}
