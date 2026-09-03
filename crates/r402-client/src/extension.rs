//! Dyn-safe client extensions (payload enrich + HTTP 402 header hook).

use std::fmt::{self, Debug, Formatter};
use std::future::Future;

use http::HeaderMap;
use r402_protocol::{ClientError, PaymentRequired};

use crate::hooks::BoxFuture;
use crate::register::PaymentClient;
use crate::select::PaymentSelector;

/// Client-side extension. Copy of the [`crate::ClientHooks`] / [`DynClientExtension`] split:
/// RPITIT is not object-safe, so stored values are [`DynClientExtension`].
///
/// SIWX implements [`Self::on_payment_required`] only (HTTP `SIGN-IN-WITH-X`).
/// Builder-code implements [`Self::enrich_payment_payload`]. Do not fold SIWX into enrich.
pub trait ClientExtension: Send + Sync {
    /// Wire key (`"sign-in-with-x"`, `"builder-code"`, …).
    fn key(&self) -> &'static str;

    /// Called after scheme signing. Default is a no-op (returns `payload_b64`).
    ///
    /// Extensions that require a server declaration must no-op internally when
    /// `payment_required.extensions` does not advertise `key`.
    fn enrich_payment_payload<'a>(
        &'a self,
        payload_b64: &'a str,
        _payment_required: &'a PaymentRequired,
    ) -> impl Future<Output = Result<String, ClientError>> + Send + 'a {
        std::future::ready(Ok(payload_b64.to_owned()))
    }

    /// Official `transportHooks.http.onPaymentRequired`. Default empty.
    ///
    /// SIWX returns `SIGN-IN-WITH-X` here. Origin is the configured public
    /// origin on the 402 challenge, never `Host`.
    fn on_payment_required<'a>(
        &'a self,
        _payment_required: &'a PaymentRequired,
    ) -> impl Future<Output = HeaderMap> + Send + 'a {
        std::future::ready(HeaderMap::new())
    }
}

/// Object-safe erasure of [`ClientExtension`].
pub trait DynClientExtension: Send + Sync {
    /// See [`ClientExtension::key`].
    fn key(&self) -> &'static str;

    /// See [`ClientExtension::enrich_payment_payload`].
    fn enrich_payment_payload<'a>(
        &'a self,
        payload_b64: &'a str,
        payment_required: &'a PaymentRequired,
    ) -> BoxFuture<'a, Result<String, ClientError>>;

    /// See [`ClientExtension::on_payment_required`].
    fn on_payment_required<'a>(
        &'a self,
        payment_required: &'a PaymentRequired,
    ) -> BoxFuture<'a, HeaderMap>;
}

impl<T: ClientExtension + ?Sized> DynClientExtension for T {
    fn key(&self) -> &'static str {
        <Self as ClientExtension>::key(self)
    }

    fn enrich_payment_payload<'a>(
        &'a self,
        payload_b64: &'a str,
        payment_required: &'a PaymentRequired,
    ) -> BoxFuture<'a, Result<String, ClientError>> {
        Box::pin(<Self as ClientExtension>::enrich_payment_payload(
            self,
            payload_b64,
            payment_required,
        ))
    }

    fn on_payment_required<'a>(
        &'a self,
        payment_required: &'a PaymentRequired,
    ) -> BoxFuture<'a, HeaderMap> {
        Box::pin(<Self as ClientExtension>::on_payment_required(
            self,
            payment_required,
        ))
    }
}

impl Debug for dyn DynClientExtension {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("DynClientExtension")
    }
}

impl<S: PaymentSelector> PaymentClient<S> {
    /// Runs [`ClientExtension::enrich_payment_payload`] for every registered extension.
    pub(crate) async fn enrich_signed_payload(
        &self,
        payload_b64: &str,
        payment_required: &PaymentRequired,
    ) -> Result<String, ClientError> {
        let mut payload = payload_b64.to_owned();
        for extension in &self.extensions {
            payload = extension
                .enrich_payment_payload(&payload, payment_required)
                .await?;
        }
        Ok(payload)
    }

    /// Merges [`ClientExtension::on_payment_required`] headers for advertised keys.
    ///
    /// Unadvertised keys are skipped (official HTTP client only invokes a
    /// transport hook when `extension.key` is in `paymentRequired.extensions`).
    pub async fn extension_headers(&self, payment_required: &PaymentRequired) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for extension in &self.extensions {
            if payment_required.extensions.get(extension.key()).is_none() {
                continue;
            }
            headers.extend(extension.on_payment_required(payment_required).await);
        }
        headers
    }
}
