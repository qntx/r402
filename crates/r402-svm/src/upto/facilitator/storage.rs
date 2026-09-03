//! Channel storage for rent-cleanup indexing.
#![allow(
    unreachable_pub,
    missing_copy_implementations,
    reason = "storage types are used by the parent facilitator module"
)]

use std::collections::HashMap;
use std::sync::Mutex;

/// Indexed channel record.
#[derive(Debug, Clone)]
pub struct UptoChannelRecord {
    /// Channel PDA (base58).
    pub channel_id: String,
    /// CAIP-2 network.
    pub network: String,
    /// `payTo` recipient.
    pub pay_to: String,
    /// Token program id.
    pub token_program: String,
    /// Payload `expiresAt`.
    pub expires_at: i64,
}

/// Pluggable channel index. Payment results do not depend on storage errors
/// after a successful on-chain settle.
pub trait UptoChannelStorage: Send + Sync {
    /// Insert or replace a channel record.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot persist the record.
    fn upsert(&self, record: UptoChannelRecord) -> Result<(), StorageError>;

    /// Snapshot of tracked channels (rent cleanup).
    fn list(&self) -> Vec<UptoChannelRecord>;
}

/// In-memory [`UptoChannelStorage`].
#[derive(Debug, Default)]
pub struct InMemoryChannelStorage {
    inner: Mutex<HashMap<String, UptoChannelRecord>>,
}

impl InMemoryChannelStorage {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl UptoChannelStorage for InMemoryChannelStorage {
    fn upsert(&self, record: UptoChannelRecord) -> Result<(), StorageError> {
        self.inner
            .lock()
            .map_err(|_| StorageError::Poisoned)?
            .insert(record.channel_id.clone(), record);
        Ok(())
    }

    fn list(&self) -> Vec<UptoChannelRecord> {
        self.inner
            .lock()
            .map(|guard| guard.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// Channel-storage failure.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Mutex poisoned.
    #[error("channel storage lock poisoned")]
    Poisoned,
}
