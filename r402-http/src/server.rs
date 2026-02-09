//! Axum/Tower server middleware for automatic x402 payment gating.
//!
//! Provides [`PaymentGateLayer`] which wraps an inner service to intercept
//! requests to payment-protected routes, enforce 402 Payment Required
//! responses, verify payments, and settle after successful responses.
//!
//! Corresponds to Python SDK's `http/x402_http_server.py` +
//! `http/x402_http_server_base.py`.

use std::collections::HashMap;
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
use crate::types::{CompiledRoute, PaywallConfig, RouteConfig, parse_route_pattern};

/// Route configuration map: pattern → [`RouteConfig`].
///
/// Keys are route patterns like `"GET /weather"` or `"/api/*"`.
pub type RoutesConfig = HashMap<String, RouteConfig>;

/// Tower [`Layer`] that adds x402 payment gating to an inner service.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use std::collections::HashMap;
/// use r402::server::X402ResourceServer;
/// use r402_http::server::PaymentGateLayer;
/// use r402_http::types::{RouteConfig, PaymentOption};
///
/// let server = Arc::new(X402ResourceServer::new());
/// let mut routes = HashMap::new();
/// routes.insert("GET /weather".into(), RouteConfig::single(PaymentOption {
///     scheme: "exact".into(),
///     pay_to: "0xRecipient".into(),
///     price: serde_json::json!("0.01"),
///     network: "eip155:8453".into(),
///     max_timeout_seconds: None,
///     extra: None,
/// }));
///
/// let layer = PaymentGateLayer::new(server, routes);
/// // Apply to Axum router: app.layer(layer)
/// ```
///
/// Corresponds to Python SDK's `x402HTTPResourceServer`.
#[derive(Clone)]
pub struct PaymentGateLayer {
    shared: Arc<PaymentGateShared>,
}

/// Shared state for the payment gate middleware.
struct PaymentGateShared {
    server: Arc<X402ResourceServer>,
    compiled_routes: Vec<CompiledRoute>,
    #[allow(dead_code)]
    paywall_config: Option<PaywallConfig>,
}

impl std::fmt::Debug for PaymentGateShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentGateShared")
            .field("server", &self.server)
            .field("routes_count", &self.compiled_routes.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PaymentGateLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentGateLayer")
            .field("shared", &self.shared)
            .finish()
    }
}

impl PaymentGateLayer {
    /// Creates a new payment gate layer with the given server and routes.
    #[must_use]
    pub fn new(server: Arc<X402ResourceServer>, routes: RoutesConfig) -> Self {
        let compiled_routes = routes
            .into_iter()
            .map(|(pattern, config)| {
                let (method, path) = parse_route_pattern(&pattern);
                CompiledRoute {
                    method,
                    path_pattern: path,
                    config,
                }
            })
            .collect();

        Self {
            shared: Arc::new(PaymentGateShared {
                server,
                compiled_routes,
                paywall_config: None,
            }),
        }
    }

    /// Creates a layer with optional paywall configuration.
    #[must_use]
    pub fn with_paywall(
        server: Arc<X402ResourceServer>,
        routes: RoutesConfig,
        paywall_config: PaywallConfig,
    ) -> Self {
        let compiled_routes = routes
            .into_iter()
            .map(|(pattern, config)| {
                let (method, path) = parse_route_pattern(&pattern);
                CompiledRoute {
                    method,
                    path_pattern: path,
                    config,
                }
            })
            .collect();

        Self {
            shared: Arc::new(PaymentGateShared {
                server,
                compiled_routes,
                paywall_config: Some(paywall_config),
            }),
        }
    }
}

impl<S> Layer<S> for PaymentGateLayer {
    type Service = PaymentGateService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PaymentGateService {
            inner,
            shared: Arc::clone(&self.shared),
        }
    }
}

/// Tower [`Service`] that enforces x402 payment requirements.
///
/// Created by [`PaymentGateLayer`]. Should not be constructed directly.
#[derive(Clone)]
pub struct PaymentGateService<S> {
    inner: S,
    shared: Arc<PaymentGateShared>,
}

impl<S> std::fmt::Debug for PaymentGateService<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentGateService")
            .field("shared", &self.shared)
            .finish_non_exhaustive()
    }
}

impl<S> Service<Request<Body>> for PaymentGateService<S>
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
            let method = req.method().as_str().to_uppercase();
            let path = req.uri().path().to_owned();

            // Find matching route
            let route = shared
                .compiled_routes
                .iter()
                .find(|r| r.matches(&method, &path));

            let route_config = match route {
                Some(r) => &r.config,
                None => {
                    // No payment required — pass through
                    return inner.call(req).await.map_err(Into::into);
                }
            };

            // Extract payment signature from headers
            let payment_payload = extract_payment_payload(&req);

            // Build payment requirements from route config
            let requirements = match build_requirements(&shared.server, route_config, &path) {
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
                url: route_config
                    .resource
                    .clone()
                    .unwrap_or_else(|| path.clone()),
                description: route_config.description.clone(),
                mime_type: route_config.mime_type.clone(),
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
                    // Payment valid — pass request to inner service
                    let mut response = inner.call(req).await.map_err(Into::into)?;

                    // Settle payment after serving the resource
                    settle_and_add_headers(&shared.server, &payload, &matching_reqs, &mut response)
                        .await;

                    Ok(response)
                }
                Ok(vr) => {
                    // Verification returned invalid
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
    _path: &str,
) -> Result<Vec<PaymentRequirements>, r402::scheme::SchemeError> {
    let mut all_requirements = Vec::new();

    for option in &route_config.accepts {
        let config = ResourceConfig {
            scheme: option.scheme.clone(),
            pay_to: option.pay_to.clone(),
            price: option.price.clone(),
            network: option.network.clone(),
            max_timeout_seconds: option.max_timeout_seconds,
        };

        let reqs = server.build_payment_requirements(&config)?;
        all_requirements.extend(reqs);
    }

    Ok(all_requirements)
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
        Ok(_) | Err(_) => {
            // Settlement failed — log but don't fail the response
            // (resource was already served)
        }
    }
}
