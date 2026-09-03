//! Static and dynamic price-tag sources.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::{HeaderMap, Uri};
use r402_protocol::payment::PriceTag;
use url::Url;

/// Resolves V2 price tags for one request.
pub trait PriceTagSource: Clone + Send + Sync + 'static {
    /// Always returns a (possibly empty) tag list. Empty means layer bypass.
    fn resolve(
        &self,
        headers: &HeaderMap,
        uri: &Uri,
        base_url: &Url,
    ) -> impl Future<Output = Vec<PriceTag>> + Send;
}

/// Fixed tag list for every request.
#[derive(Clone, Debug)]
pub struct StaticPriceTags {
    tags: Arc<[PriceTag]>,
}

impl StaticPriceTags {
    /// Stores `tags`.
    #[must_use]
    pub fn new(tags: Vec<PriceTag>) -> Self {
        Self { tags: tags.into() }
    }

    /// Stored tags.
    #[must_use]
    pub fn tags(&self) -> &[PriceTag] {
        &self.tags
    }

    /// Appends one tag.
    #[must_use]
    pub fn with_price_tag(mut self, tag: PriceTag) -> Self {
        let mut tags = self.tags.to_vec();
        tags.push(tag);
        self.tags = tags.into();
        self
    }
}

impl PriceTagSource for StaticPriceTags {
    fn resolve(
        &self,
        _headers: &HeaderMap,
        _uri: &Uri,
        _base_url: &Url,
    ) -> impl Future<Output = Vec<PriceTag>> + Send {
        std::future::ready(self.tags.to_vec())
    }
}

type BoxedDynamicPriceCallback = dyn for<'a> Fn(
        &'a HeaderMap,
        &'a Uri,
        &'a Url,
    ) -> Pin<Box<dyn Future<Output = Vec<PriceTag>> + Send + 'a>>
    + Send
    + Sync;

/// Per-request price tags from an async callback.
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
    /// Wraps `callback`.
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(&HeaderMap, &Uri, &Url) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<PriceTag>> + Send + 'static,
    {
        Self {
            callback: Arc::new(move |headers, uri, base_url| {
                Box::pin(callback(headers, uri, base_url))
            }),
        }
    }
}

impl PriceTagSource for DynamicPriceTags {
    async fn resolve(&self, headers: &HeaderMap, uri: &Uri, base_url: &Url) -> Vec<PriceTag> {
        (self.callback)(headers, uri, base_url).await
    }
}
