# r402

x402 Payment Protocol SDK for Rust. Protocol version **2** only.

```toml
[dependencies]
r402 = { version = "0.19", features = ["evm", "http"] }
```

`default = ["evm", "http"]`. Concordium is opt-in (`concordium`).

## Protect a Route (Server)

```rust
use alloy_primitives::address;
use axum::{Router, routing::get};
use r402::evm::{Eip155Exact, USDC};
use r402::http::server::X402Middleware;

let x402 = X402Middleware::try_new("https://facilitator.example.com")?
    .with_base_url("https://api.example.com".parse()?)
    .with_scheme("eip155:*".parse()?, Eip155Exact);

let app = Router::new().route(
    "/paid-content",
    get(handler).layer(
        x402.with_price_tag(Eip155Exact::price_tag(
            address!("0xYourPayToAddress"),
            USDC::base().amount(1_000_000u64),
            None,
        ))?,
    ),
);
```

## Send Payments (Client)

```rust
use alloy_signer_local::PrivateKeySigner;
use r402::evm::Eip155ExactClient;
use r402::http::{WithPayments, X402Client};
use std::sync::Arc;

let signer = Arc::new("0x...".parse::<PrivateKeySigner>()?);
let client = reqwest::Client::new().with_payments(
    X402Client::new().register(Eip155ExactClient::new(signer)),
);
let res = client.get("https://api.example.com/paid").send().await?;
```

## Settlement Modes

Configurable via [`with_settlement_mode()`](https://docs.rs/r402-http/latest/r402_http/server/struct.X402Layer.html#method.with_settlement_mode) after `with_price_tag`.

### Sequential (default)

Verify → execute → settle. On-chain settlement only after the handler succeeds. `Payment-Response` is attached.

```mermaid
sequenceDiagram
    participant C as Client
    participant S as Server
    participant F as Facilitator
    participant H as Handler

    C->>S: HTTP Request + Payment-Signature
    S->>F: verify(payment)
    F-->>S: VerifyResponse ✓
    S->>H: execute request
    Note over S,H: Balance verified but NOT locked —<br/>handler executing (variable latency)
    H-->>S: response body
    S->>F: settle(payment)
    Note over S,F: On-chain transfer (2–5 s)
    F-->>S: SettleResponse (tx_hash)
    S-->>C: 200 OK + Payment-Response header
```

### Concurrent

Verify → (settle ∥ execute) → await both. On handler error the settlement task is detached.

```mermaid
sequenceDiagram
    participant C as Client
    participant S as Server
    participant F as Facilitator
    participant H as Handler

    C->>S: HTTP Request + Payment-Signature
    S->>F: verify(payment)
    F-->>S: VerifyResponse ✓
    par settle ∥ execute
        S->>F: settle(payment)
        Note over S,F: On-chain transfer
        F-->>S: SettleResponse (tx_hash)
    and
        S->>H: execute request
        H-->>S: response body
    end
    S-->>C: 200 OK + Payment-Response header
```

### Background

Verify → spawn settle → execute → return. No `Payment-Response` header. For streaming responses.

```mermaid
sequenceDiagram
    participant C as Client
    participant S as Server
    participant F as Facilitator
    participant H as Handler

    C->>S: HTTP Request + Payment-Signature
    S->>F: verify(payment)
    F-->>S: VerifyResponse ✓
    S-)F: settle(payment) [fire-and-forget]
    S->>H: execute request
    H-->>S: response body (or stream)
    S-->>C: 200 OK (no Payment-Response header)
    Note over S,F: Settlement completes asynchronously
    F-)S: SettleResponse (logged)
```

### Comparison

| Mode | Total latency | Safety | `Payment-Response` | Best for |
| --- | --- | --- | --- | --- |
| **Sequential** | verify + handler + settle | Settlement only on handler success | ✅ Included | Standard request/response APIs |
| **Concurrent** | verify + max(handler, settle) | Settlement may occur on handler failure | ✅ Included | Latency-sensitive endpoints |
| **Background** | verify + handler | Settlement errors are non-fatal (logged) | ❌ Not attached | SSE / LLM streaming responses |

> **Note:** `UptoActualAmount` is honoured only by `SettlementMode::Sequential`. Concurrent and background modes start settlement before the handler returns and therefore charge the signed maximum.

## License

Licensed under either of MIT or Apache-2.0, at your option.
