//! Price tag sources for the x402 payment gate.
//!
//! Abstracts over static and dynamic pricing strategies via the
//! [`PriceTagSource`] trait.

use http::{HeaderMap, Uri};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use url::Url;

use super::protocol::PaygateProtocol;

/// Trait for types that can provide price tags for a request.
///
/// This trait abstracts over static and dynamic pricing strategies.
/// Implementations must be infallible - they always return price tags.
pub trait PriceTagSource {
    /// The concrete price tag type produced by this source.
    type PriceTag: PaygateProtocol;

    /// Resolves price tags for the given request context.
    ///
    /// This method is infallible - it must always return a non-empty vector of price tags.
    fn resolve(
        &self,
        headers: &HeaderMap,
        uri: &Uri,
        base_url: Option<&Url>,
    ) -> impl Future<Output = Vec<Self::PriceTag>> + Send;
}

/// Static price tag source - returns the same price tags for every request.
///
/// This is the default implementation used when calling `with_price_tag()`.
/// It simply stores a vector of price tags and returns clones on each request.
#[derive(Clone, Debug)]
pub struct StaticPriceTags<TPriceTag> {
    tags: Arc<Vec<TPriceTag>>,
}

impl<TPriceTag> StaticPriceTags<TPriceTag> {
    /// Creates a new static price tag source from a vector of price tags.
    #[must_use]
    pub fn new(tags: Vec<TPriceTag>) -> Self {
        Self {
            tags: Arc::new(tags),
        }
    }

    /// Returns a reference to the stored price tags.
    #[must_use]
    pub fn tags(&self) -> &[TPriceTag] {
        &self.tags
    }
}

impl<TPriceTag> StaticPriceTags<TPriceTag>
where
    TPriceTag: Clone,
{
    /// Adds a price tag to the source.
    #[must_use]
    pub fn with_price_tag(mut self, tag: TPriceTag) -> Self {
        let mut tags = (*self.tags).clone();
        tags.push(tag);
        self.tags = Arc::new(tags);
        self
    }
}

impl<TPriceTag> PriceTagSource for StaticPriceTags<TPriceTag>
where
    TPriceTag: PaygateProtocol,
{
    type PriceTag = TPriceTag;

    async fn resolve(
        &self,
        _headers: &HeaderMap,
        _uri: &Uri,
        _base_url: Option<&Url>,
    ) -> Vec<Self::PriceTag> {
        (*self.tags).clone()
    }
}

/// Internal type alias for the boxed dynamic pricing callback.
/// Users don't interact with this directly.
///
/// Uses higher-ranked trait bounds (HRTB) to express that the callback
/// works with any lifetime of the input references.
type BoxedDynamicPriceCallback<TPriceTag> = dyn for<'a> Fn(
        &'a HeaderMap,
        &'a Uri,
        Option<&'a Url>,
    ) -> Pin<Box<dyn Future<Output = Vec<TPriceTag>> + Send + 'a>>
    + Send
    + Sync;

/// Dynamic price tag source - computes price tags per-request via callback.
///
/// This implementation allows computing different prices based on request
/// headers, URI, or other runtime factors.
pub struct DynamicPriceTags<TPriceTag> {
    callback: Arc<BoxedDynamicPriceCallback<TPriceTag>>,
}

impl<TPriceTag> Clone for DynamicPriceTags<TPriceTag> {
    fn clone(&self) -> Self {
        Self {
            callback: Arc::clone(&self.callback),
        }
    }
}

impl<TPriceTag> std::fmt::Debug for DynamicPriceTags<TPriceTag> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicPriceTags")
            .field("callback", &"<callback>")
            .finish()
    }
}

impl<TPriceTag> DynamicPriceTags<TPriceTag> {
    /// Creates a new dynamic price source from an async closure.
    ///
    /// The closure receives request context and returns a vector of price tags.
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(&HeaderMap, &Uri, Option<&Url>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<TPriceTag>> + Send + 'static,
    {
        Self {
            callback: Arc::new(move |headers, uri, base_url| {
                Box::pin(callback(headers, uri, base_url))
            }),
        }
    }
}

impl<TPriceTag> PriceTagSource for DynamicPriceTags<TPriceTag>
where
    TPriceTag: PaygateProtocol,
{
    type PriceTag = TPriceTag;

    async fn resolve(
        &self,
        headers: &HeaderMap,
        uri: &Uri,
        base_url: Option<&Url>,
    ) -> Vec<Self::PriceTag> {
        (self.callback)(headers, uri, base_url).await
    }
}
