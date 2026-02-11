//! Facilitator-side scheme handler builder trait.
//!
//! This module provides [`SchemeHandlerBuilder`] for constructing [`Facilitator`]
//! instances from chain providers.

use crate::facilitator::Facilitator;

/// Trait for building facilitator instances from chain providers.
///
/// The type parameter `P` represents the chain provider type.
pub trait SchemeHandlerBuilder<P> {
    /// Creates a new facilitator for the given chain provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the facilitator cannot be built from the provider.
    fn build(
        &self,
        provider: P,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn Facilitator>, Box<dyn std::error::Error>>;
}
