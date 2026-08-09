//! Channel accounting store for batch-settlement servers/facilitators.
//!
//! Integrators replace [`MemoryChannelStore`] with durable backends; the trait
//! is the only extension point (no plugin framework).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy_primitives::B256;

use super::types::ChannelState;
use crate::chain::TokenAmount;

/// Read/write channel accounting keyed by channel id.
pub trait ChannelStore: Send + Sync {
    /// Loads channel state (default empty if unknown).
    fn get(&self, channel_id: &B256) -> ChannelState;

    /// Persists channel state.
    fn put(&self, channel_id: B256, state: ChannelState);

    /// Atomically reserves a charge: requires
    /// `max_claimable >= charged_cumulative + charge` and then
    /// `charged_cumulative += charge`.
    ///
    /// # Errors
    ///
    /// Returns `false` when the voucher ceiling is insufficient.
    fn try_charge(
        &self,
        channel_id: B256,
        charge: TokenAmount,
        max_claimable: TokenAmount,
    ) -> bool {
        let mut state = self.get(&channel_id);
        let next = state.charged_cumulative.0.saturating_add(charge.0);
        if next > max_claimable.0 {
            return false;
        }
        state.charged_cumulative = TokenAmount::from(next);
        self.put(channel_id, state);
        true
    }
}

/// Process-local in-memory store (tests and single-node demos).
#[derive(Debug, Default, Clone)]
pub struct MemoryChannelStore {
    inner: Arc<Mutex<HashMap<B256, ChannelState>>>,
}

impl MemoryChannelStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ChannelStore for MemoryChannelStore {
    fn get(&self, channel_id: &B256) -> ChannelState {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(channel_id)
            .copied()
            .unwrap_or_default()
    }

    fn put(&self, channel_id: B256, state: ChannelState) {
        let _ = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(channel_id, state);
    }

    fn try_charge(
        &self,
        channel_id: B256,
        charge: TokenAmount,
        max_claimable: TokenAmount,
    ) -> bool {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = guard.entry(channel_id).or_default();
        let next = entry.charged_cumulative.0.saturating_add(charge.0);
        if next > max_claimable.0 {
            return false;
        }
        entry.charged_cumulative = TokenAmount::from(next);
        drop(guard);
        true
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;

    use super::*;

    #[test]
    fn charge_monotonic() {
        let store = MemoryChannelStore::new();
        let id = B256::repeat_byte(0xab);
        assert!(store.try_charge(
            id,
            TokenAmount::from(U256::from(10_u64)),
            TokenAmount::from(U256::from(100_u64))
        ));
        assert_eq!(store.get(&id).charged_cumulative.0, U256::from(10_u64));
        assert!(!store.try_charge(
            id,
            TokenAmount::from(U256::from(100_u64)),
            TokenAmount::from(U256::from(100_u64))
        ));
        assert!(store.try_charge(
            id,
            TokenAmount::from(U256::from(90_u64)),
            TokenAmount::from(U256::from(100_u64))
        ));
    }
}
