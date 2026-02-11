//! Price tag sources for the x402 payment gate.
//!
//! Abstracts over static and dynamic pricing strategies via the
//! [`PriceTagSource`] trait. All sources produce [`v2::PriceTag`] values
//! (V2-only server layer).

use http::{HeaderMap, Uri};
use r402::proto::v2;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use url::Url;

/// Trait for types that can provide V2 price tags for a request.
///
/// This trait abstracts over static and dynamic pricing strategies.
/// Implementations must be infallible - they always return price tags.
pub trait PriceTagSource: Clone + Send + Sync + 'static {
    /// Resolves price tags for the given request context.
    ///
    /// This method is infallible - it must always return a non-empty vector of price tags.
    fn resolve(
        &self,
        headers: &HeaderMap,
        uri: &Uri,
        base_url: Option<&Url>,
    ) -> impl Future<Output = Vec<v2::PriceTag>> + Send;
}

/// Static price tag source - returns the same price tags for every request.
///
/// This is the default implementation used when calling `with_price_tag()`.
/// It simply stores a vector of price tags and returns clones on each request.
#[derive(Clone, Debug)]
pub struct StaticPriceTags {
    tags: Arc<Vec<v2::PriceTag>>,
}

impl StaticPriceTags {
    /// Creates a new static price tag source from a vector of price tags.
    #[must_use]
    pub fn new(tags: Vec<v2::PriceTag>) -> Self {
        Self {
            tags: Arc::new(tags),
        }
    }

    /// Returns a reference to the stored price tags.
    #[must_use]
    pub fn tags(&self) -> &[v2::PriceTag] {
        &self.tags
    }

    /// Adds a price tag to the source.
    #[must_use]
    pub fn with_price_tag(mut self, tag: v2::PriceTag) -> Self {
        let mut tags = (*self.tags).clone();
        tags.push(tag);
        self.tags = Arc::new(tags);
        self
    }
}

impl PriceTagSource for StaticPriceTags {
    async fn resolve(
        &self,
        _headers: &HeaderMap,
        _uri: &Uri,
        _base_url: Option<&Url>,
    ) -> Vec<v2::PriceTag> {
        (*self.tags).clone()
    }
}

/// Internal type alias for the boxed dynamic pricing callback.
type BoxedDynamicPriceCallback = dyn for<'a> Fn(
        &'a HeaderMap,
        &'a Uri,
        Option<&'a Url>,
    ) -> Pin<Box<dyn Future<Output = Vec<v2::PriceTag>> + Send + 'a>>
    + Send
    + Sync;

/// Dynamic price tag source - computes price tags per-request via callback.
///
/// This implementation allows computing different prices based on request
/// headers, URI, or other runtime factors.
pub struct DynamicPriceTags {
    callback: Arc<BoxedDynamicPriceCallback>,
}

impl Clone for DynamicPriceTags {
    fn clone(&self) -> Self {
        Self {
            callback: Arc::clone(&self.callback),
        }
    }
}

impl std::fmt::Debug for DynamicPriceTags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicPriceTags")
            .field("callback", &"<callback>")
            .finish()
    }
}

impl DynamicPriceTags {
    /// Creates a new dynamic price source from an async closure.
    ///
    /// The closure receives request context and returns a vector of price tags.
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(&HeaderMap, &Uri, Option<&Url>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<v2::PriceTag>> + Send + 'static,
    {
        Self {
            callback: Arc::new(move |headers, uri, base_url| {
                Box::pin(callback(headers, uri, base_url))
            }),
        }
    }
}

impl PriceTagSource for DynamicPriceTags {
    async fn resolve(
        &self,
        headers: &HeaderMap,
        uri: &Uri,
        base_url: Option<&Url>,
    ) -> Vec<v2::PriceTag> {
        (self.callback)(headers, uri, base_url).await
    }
}
