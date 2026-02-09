//! Per-route Axum payment gate middleware.
//!
//! Provides [`PaymentGate`] for creating per-route payment layers that
//! integrate natively with Axum's `.layer()` method. Unlike
//! [`super::server::PaymentGateLayer`] which uses a global route map with
//! string-based matching, this module lets Axum handle routing while each
//! route independently configures its payment requirements.
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use r402::server::X402ResourceServer;
//! use r402_http::paygate::PaymentGate;
//! use r402_http::types::{PaymentOption, RouteConfig};
//! use axum::{Router, routing::get};
//!
//! let server = Arc::new(X402ResourceServer::new());
//! let gate = PaymentGate::new(server);
//!
//! let app = Router::new()
//!     .route("/weather", get(weather_handler).layer(
//!         gate.route(RouteConfig::single(PaymentOption {
//!             scheme: "exact".into(),
//!             pay_to: "0xRecipient".into(),
//!             price: serde_json::json!("0.01"),
//!             network: "eip155:8453".into(),
//!             max_timeout_seconds: None,
//!             extra: None,
//!         }))
//!         .with_description("Weather forecast data")
//!     ))
//!     .route("/premium", get(premium_handler).layer(
//!         gate.route(RouteConfig::single(PaymentOption {
//!             scheme: "exact".into(),
//!             pay_to: "0xRecipient".into(),
//!             price: serde_json::json!("1.00"),
//!             network: "eip155:8453".into(),
//!             max_timeout_seconds: None,
//!             extra: None,
//!         }))
//!         .with_description("Premium content")
//!         .with_mime_type("application/json")
//!     ));
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum_core::body::Body;
use http::{Request, Response, StatusCode};
use r402::config::ResourceConfig;
use r402::proto::{PaymentPayload, PaymentRequirements, ResourceInfo};
use r402::server::X402ResourceServer;
use tower::{Layer, Service};

use crate::constants::{PAYMENT_REQUIRED_HEADER, PAYMENT_SIGNATURE_HEADER};
use crate::headers::{decode_payment_payload, encode_payment_required, encode_payment_response};
use crate::types::RouteConfig;

/// Per-route payment gate factory.
///
/// Holds a shared reference to the [`X402ResourceServer`] and provides
/// [`PaymentGate::route`] to create per-route [`PaymentRouteLayer`] instances.
#[derive(Clone, Debug)]
pub struct PaymentGate {
    server: Arc<X402ResourceServer>,
}

impl PaymentGate {
    /// Creates a new payment gate backed by the given resource server.
    #[must_use]
    pub fn new(server: Arc<X402ResourceServer>) -> Self {
        Self { server }
    }

    /// Creates a per-route layer for the given route configuration.
    ///
    /// The returned [`PaymentRouteLayer`] implements [`Layer`] and can be
    /// applied to individual Axum routes via `.layer()`.
    #[must_use]
    pub fn route(&self, config: RouteConfig) -> PaymentRouteLayer {
        PaymentRouteLayer {
            shared: Arc::new(PaymentRouteShared {
                server: Arc::clone(&self.server),
                config,
            }),
        }
    }
}

/// Shared state for a single payment-protected route.
struct PaymentRouteShared {
    server: Arc<X402ResourceServer>,
    config: RouteConfig,
}

impl std::fmt::Debug for PaymentRouteShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentRouteShared")
            .field("server", &self.server)
            .field("accepts_count", &self.config.accepts.len())
            .finish_non_exhaustive()
    }
}

/// Per-route Tower [`Layer`] that enforces x402 payment requirements.
///
/// Created by [`PaymentGate::route`]. Supports fluent builder methods
/// for resource metadata before being applied as a layer.
#[derive(Clone, Debug)]
pub struct PaymentRouteLayer {
    shared: Arc<PaymentRouteShared>,
}

impl PaymentRouteLayer {
    /// Sets a human-readable description of the protected resource.
    #[must_use]
    pub fn with_description(self, desc: impl Into<String>) -> Self {
        let shared = (*self.shared).clone_with_description(Some(desc.into()));
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Sets the MIME type of the protected resource.
    #[must_use]
    pub fn with_mime_type(self, mime: impl Into<String>) -> Self {
        let shared = (*self.shared).clone_with_mime_type(Some(mime.into()));
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Sets the resource URL override.
    #[must_use]
    pub fn with_resource(self, url: impl Into<String>) -> Self {
        let shared = (*self.shared).clone_with_resource(Some(url.into()));
        Self {
            shared: Arc::new(shared),
        }
    }
}

impl PaymentRouteShared {
    fn clone_with_description(&self, desc: Option<String>) -> Self {
        let mut config = self.config.clone();
        config.description = desc;
        Self {
            server: Arc::clone(&self.server),
            config,
        }
    }

    fn clone_with_mime_type(&self, mime: Option<String>) -> Self {
        let mut config = self.config.clone();
        config.mime_type = mime;
        Self {
            server: Arc::clone(&self.server),
            config,
        }
    }

    fn clone_with_resource(&self, url: Option<String>) -> Self {
        let mut config = self.config.clone();
        config.resource = url;
        Self {
            server: Arc::clone(&self.server),
            config,
        }
    }
}

impl<S> Layer<S> for PaymentRouteLayer {
    type Service = PaymentRouteService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PaymentRouteService {
            inner,
            shared: Arc::clone(&self.shared),
        }
    }
}

/// Per-route Tower [`Service`] that enforces x402 payment requirements.
///
/// Created by [`PaymentRouteLayer`]. Should not be constructed directly.
#[derive(Clone)]
pub struct PaymentRouteService<S> {
    inner: S,
    shared: Arc<PaymentRouteShared>,
}

impl<S> std::fmt::Debug for PaymentRouteService<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentRouteService")
            .field("shared", &self.shared)
            .finish_non_exhaustive()
    }
}

impl<S> Service<Request<Body>> for PaymentRouteService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    type Response = Response<Body>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let shared = Arc::clone(&self.shared);
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let path = req.uri().path().to_owned();

            // Extract payment signature from headers
            let payment_payload = extract_payment_payload(&req);

            // Build payment requirements from route config
            let requirements = match build_requirements(&shared.server, &shared.config) {
                Ok(reqs) => reqs,
                Err(e) => {
                    return Ok(error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("Failed to build payment requirements: {e}"),
                    ));
                }
            };

            // Build resource info
            let resource_info = ResourceInfo {
                url: shared
                    .config
                    .resource
                    .clone()
                    .unwrap_or_else(|| path.clone()),
                description: shared.config.description.clone(),
                mime_type: shared.config.mime_type.clone(),
            };

            // No payment provided → return 402
            let payload = match payment_payload {
                Some(p) => p,
                None => {
                    let payment_required = shared.server.create_payment_required(
                        requirements,
                        Some(resource_info),
                        Some("Payment required".to_owned()),
                        None,
                    );
                    return Ok(payment_required_response(&payment_required));
                }
            };

            // Find matching requirements for this payload
            let matching_reqs = match shared
                .server
                .find_matching_requirements(&requirements, &payload)
            {
                Some(reqs) => reqs.clone(),
                None => {
                    let payment_required = shared.server.create_payment_required(
                        requirements,
                        Some(resource_info),
                        Some("No matching payment requirements".to_owned()),
                        None,
                    );
                    return Ok(payment_required_response(&payment_required));
                }
            };

            // Verify payment via facilitator
            let verify_result = shared.server.verify_payment(&payload, &matching_reqs).await;

            match verify_result {
                Ok(ref vr) if vr.is_valid => {
                    let mut response = inner.call(req).await.map_err(Into::into)?;
                    settle_and_add_headers(&shared.server, &payload, &matching_reqs, &mut response)
                        .await;
                    Ok(response)
                }
                Ok(vr) => {
                    let payment_required = shared.server.create_payment_required(
                        requirements,
                        Some(resource_info),
                        vr.invalid_reason.clone(),
                        None,
                    );
                    Ok(payment_required_response(&payment_required))
                }
                Err(e) => {
                    let payment_required = shared.server.create_payment_required(
                        requirements,
                        Some(resource_info),
                        Some(e.to_string()),
                        None,
                    );
                    Ok(payment_required_response(&payment_required))
                }
            }
        })
    }
}

/// Extracts and decodes a V2 payment payload from the `PAYMENT-SIGNATURE` header.
fn extract_payment_payload(req: &Request<Body>) -> Option<PaymentPayload> {
    let header_value = req.headers().get(PAYMENT_SIGNATURE_HEADER).or_else(|| {
        req.headers()
            .get(PAYMENT_SIGNATURE_HEADER.to_lowercase().as_str())
    })?;
    let value_str = header_value.to_str().ok()?;
    let parsed = decode_payment_payload(value_str).ok()?;
    match parsed {
        r402::proto::helpers::PaymentPayloadEnum::V2(p) => Some(*p),
        r402::proto::helpers::PaymentPayloadEnum::V1(_) => None,
    }
}

/// Builds payment requirements from route config payment options.
fn build_requirements(
    server: &X402ResourceServer,
    route_config: &RouteConfig,
) -> Result<Vec<PaymentRequirements>, r402::scheme::SchemeError> {
    let mut all = Vec::new();
    for option in &route_config.accepts {
        let config = ResourceConfig {
            scheme: option.scheme.clone(),
            pay_to: option.pay_to.clone(),
            price: option.price.clone(),
            network: option.network.clone(),
            max_timeout_seconds: option.max_timeout_seconds,
        };
        let reqs = server.build_payment_requirements(&config)?;
        all.extend(reqs);
    }
    Ok(all)
}

/// Creates a 402 Payment Required HTTP response with the encoded header.
fn payment_required_response(payment_required: &r402::proto::PaymentRequired) -> Response<Body> {
    let encoded = encode_payment_required(payment_required).unwrap_or_default();
    let body_json = serde_json::to_string(payment_required).unwrap_or_default();

    Response::builder()
        .status(StatusCode::PAYMENT_REQUIRED)
        .header(PAYMENT_REQUIRED_HEADER, &encoded)
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(
            http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
            PAYMENT_REQUIRED_HEADER,
        )
        .body(Body::from(body_json))
        .expect("valid 402 response")
}

/// Creates a JSON error response.
fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({ "error": message });
    Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("valid error response")
}

/// Settles the payment and adds `PAYMENT-RESPONSE` header to the response.
async fn settle_and_add_headers(
    server: &X402ResourceServer,
    payload: &PaymentPayload,
    requirements: &PaymentRequirements,
    response: &mut Response<Body>,
) {
    match server.settle_payment(payload, requirements).await {
        Ok(settle_response) if settle_response.success => {
            if let Ok(encoded) = encode_payment_response(&settle_response) {
                response.headers_mut().insert(
                    http::header::HeaderName::from_static("payment-response"),
                    http::header::HeaderValue::from_str(&encoded)
                        .unwrap_or_else(|_| http::header::HeaderValue::from_static("")),
                );
                response.headers_mut().insert(
                    http::header::HeaderName::from_static("access-control-expose-headers"),
                    http::header::HeaderValue::from_static("PAYMENT-RESPONSE"),
                );
            }
        }
        Ok(_) | Err(_) => {}
    }
}
