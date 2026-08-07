//! `bazaar` extension — optional resource discovery metadata.
//!
//! Sellers that opt into the bazaar extension advertise their paid resources
//! via the `PaymentRequired.extensions["bazaar"]` block. This gives
//! discovery services (Coinbase Bazaar, custom aggregators) a machine
//! readable description without requiring a separate HTTP endpoint.
//!
//! # Wire format
//!
//! ```json
//! {
//!   "extensions": {
//!     "bazaar": {
//!       "info": {
//!         "catalog": "https://example.com/.well-known/x402-catalog",
//!         "categories": ["data", "api"],
//!         "registered": true
//!       }
//!     }
//!   }
//! }
//! ```

#![cfg(feature = "ext-bazaar")]
#![cfg_attr(docsrs, doc(cfg(feature = "ext-bazaar")))]

use compact_str::CompactString;
use serde_json::Value;

use super::{AdvertiseContext, Extension};
use crate::wire::ExtensionEntry;

/// Server-side implementation of the `bazaar` discovery extension.
#[derive(Debug, Clone, Default)]
pub struct BazaarExtension {
    /// Optional catalog URL pointing at a machine-readable resource index.
    pub catalog: Option<CompactString>,
    /// Optional list of free-form category labels.
    pub categories: Vec<CompactString>,
    /// Whether this resource has been registered with the upstream discovery
    /// service.
    pub registered: bool,
}

impl BazaarExtension {
    /// Constructs an empty bazaar extension.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: attaches a catalog URL.
    #[must_use]
    pub fn with_catalog(mut self, catalog: impl Into<CompactString>) -> Self {
        self.catalog = Some(catalog.into());
        self
    }

    /// Builder: adds a single category label.
    #[must_use]
    pub fn with_category(mut self, category: impl Into<CompactString>) -> Self {
        self.categories.push(category.into());
        self
    }

    /// Builder: marks the resource as registered.
    #[must_use]
    pub const fn registered(mut self) -> Self {
        self.registered = true;
        self
    }
}

impl Extension for BazaarExtension {
    fn id(&self) -> &'static str {
        "bazaar"
    }

    fn advertise(&self, _ctx: &AdvertiseContext<'_>) -> Option<ExtensionEntry> {
        let mut info = serde_json::Map::new();
        if let Some(catalog) = &self.catalog {
            let _ = info.insert("catalog".to_owned(), Value::String(catalog.to_string()));
        }
        if !self.categories.is_empty() {
            let cats: Vec<Value> = self
                .categories
                .iter()
                .map(|c| Value::String(c.to_string()))
                .collect();
            let _ = info.insert("categories".to_owned(), Value::Array(cats));
        }
        let _ = info.insert("registered".to_owned(), Value::Bool(self.registered));
        Some(ExtensionEntry::info(Value::Object(info)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn advertise_includes_catalog_and_categories() {
        let ext = BazaarExtension::new()
            .with_catalog("https://example.com/catalog")
            .with_category("data")
            .with_category("api")
            .registered();
        let ctx = AdvertiseContext { requirement: None };
        let entry = ext.advertise(&ctx).unwrap();
        let info = entry.as_info().unwrap();
        assert_eq!(info["catalog"], "https://example.com/catalog");
        assert_eq!(info["registered"], true);
        assert_eq!(info["categories"], json!(["data", "api"]));
    }

    #[test]
    fn advertise_minimal_emits_registered_flag() {
        let ext = BazaarExtension::new();
        let ctx = AdvertiseContext { requirement: None };
        let entry = ext.advertise(&ctx).unwrap();
        let info = entry.as_info().unwrap();
        assert_eq!(info["registered"], false);
        assert!(info.get("catalog").is_none());
    }
}
