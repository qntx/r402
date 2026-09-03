//! Remote HTTP facilitator client (`POST /verify`, `POST /settle`, `GET /supported`).

mod auth;
mod cache;
mod client;
mod extension;
mod retry;

pub use auth::FacilitatorAuthHeaders;
pub use cache::SupportedCache;
pub use client::{FacilitatorClient, FacilitatorClientError};
pub use retry::compute_retry_delay;
