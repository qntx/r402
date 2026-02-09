//! Axum route handlers for the local facilitator service.
//!
//! Provides REST endpoints for verify, settle, and supported operations.
//! Corresponds to the facilitator HTTP API defined by the x402 protocol.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use r402::facilitator::X402FacilitatorBase;
use r402_proto::{SettleResponse, SupportedResponse, VerifyResponse};

use crate::error::FacilitatorError;

/// Shared application state for the facilitator service.
pub type FacilitatorState = Arc<X402FacilitatorBase>;

/// `GET /supported` — Returns the list of supported payment kinds.
pub async fn get_supported(State(fac): State<FacilitatorState>) -> Json<SupportedResponse> {
    Json(fac.get_supported())
}

/// `POST /verify` — Verifies a V2 payment payload.
///
/// # Errors
///
/// Returns 404 if no scheme handler is registered, or 400 on bad input.
pub async fn post_verify(
    State(fac): State<FacilitatorState>,
    Json(body): Json<r402_proto::v2::VerifyRequest>,
) -> Result<Json<VerifyResponse>, FacilitatorError> {
    let result = fac.verify_v2(&body.payment_payload, &body.payment_requirements)?;
    Ok(Json(result))
}

/// `POST /settle` — Settles a V2 payment on-chain.
///
/// # Errors
///
/// Returns 404 if no scheme handler is registered, or 400 on bad input.
pub async fn post_settle(
    State(fac): State<FacilitatorState>,
    Json(body): Json<r402_proto::v2::SettleRequest>,
) -> Result<Json<SettleResponse>, FacilitatorError> {
    let result = fac.settle_v2(&body.payment_payload, &body.payment_requirements)?;
    Ok(Json(result))
}

/// Creates an Axum [`axum::Router`] with all facilitator endpoints.
///
/// Endpoints:
/// - `GET /supported` — list supported payment kinds
/// - `POST /verify` — verify a payment
/// - `POST /settle` — settle a payment
pub fn facilitator_router(state: FacilitatorState) -> axum::Router {
    axum::Router::new()
        .route("/supported", axum::routing::get(get_supported))
        .route("/verify", axum::routing::post(post_verify))
        .route("/settle", axum::routing::post(post_settle))
        .with_state(state)
}
