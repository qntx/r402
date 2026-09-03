//! Axum/Tower Sequential payment gate.

mod builder;
mod fail;
mod gate;
mod hooks;
mod layer;
mod overrides;
mod pricing;
mod receipt;
mod verify;

pub use builder::{BuildError, X402Middleware};
pub use fail::{GateError, reason_to_status};
pub use gate::Gate;
pub use hooks::{DynGateHooks, GateHooks, ProtectedRequestOutcome};
pub use layer::{X402Layer, X402MiddlewareService};
pub use overrides::{
    SETTLEMENT_OVERRIDES_HEADER, SettlementOverrideError, SettlementOverrides, UptoActualAmount,
    marshal_settlement_overrides, resolve_response_settlement_amount, set_settlement_overrides,
    set_settlement_overrides_on_response, settlement_overrides_header_name,
    take_settlement_overrides_header,
};
pub use pricing::{DynamicPriceTags, PriceTagSource, StaticPriceTags};
pub use r402_facilitator::{Facilitator, FacilitatorClient, FacilitatorClientError};
pub use r402_server::{ResourceServer, SchemeNetworkServer, SettlementMode, SkipHandlerDirective};
pub use receipt::settlement_to_header;

pub use crate::headers::{
    PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE, SIGN_IN_WITH_X, X402_EXPOSED_HEADERS,
    ensure_expose_headers, merge_private, set_no_store,
};
