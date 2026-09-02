//! Remote HTTP facilitator client (`POST /verify`, `POST /settle`, `GET /supported`).

mod auth;
mod client;
mod retry;

pub use auth::FacilitatorAuthHeaders;
pub use client::{FacilitatorClient, FacilitatorClientError, SupportedCache};
pub use retry::compute_retry_delay;
