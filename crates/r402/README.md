# r402

x402 Payment Protocol SDK for Rust. Protocol version **2** only.

```toml
[dependencies]
r402 = { version = "0.18", features = ["evm", "http"] }
```

`default = ["evm", "http"]` opens each crate's `client` and `server` features.
Do not pass `client` / `server` on the umbrella; those flags live on the
member crates.

## Protect a route

`Eip155Exact::price_tag` is three arguments (`pay_to`, `asset`,
`Option<AssetTransferMethod>`). `with_price_tag` returns
`Result<X402Layer, BuildError>`. Missing `base_url` fails at construct time
(`BuildError::MissingBaseUrl`). `with_resource` does not waive `base_url`.
There is no `X402Route` and no `into_layer`. Tower `Layer` is implemented
only on `X402Layer`.

Consumer binaries typically return `anyhow::Result`.

```rust,no_run
fn paid_router() -> Result<axum::Router, Box<dyn std::error::Error>> {
    use alloy_primitives::address;
    use axum::{Router, routing::get};
    use r402::evm::{Eip155Exact, USDC};
    use r402::http::server::{SettlementMode, X402Middleware};

    async fn handler() -> &'static str {
        "ok"
    }

    let x402 = X402Middleware::try_new("https://facilitator.example.com")?
        .with_base_url("https://api.example.com".parse()?)
        .with_scheme("eip155:*".parse()?, Eip155Exact);

    let app = Router::new().route(
        "/paid-content",
        get(handler).layer(
            x402.with_price_tag(Eip155Exact::price_tag(
                address!("0x1111111111111111111111111111111111111111"),
                USDC::base().amount(1_000_000u64),
                None,
            ))?
            .with_settlement_mode(SettlementMode::Sequential),
        ),
    );
    Ok(app)
}
```

```text
X402Middleware::try_new(url)?
    .with_base_url(origin)             // required; omitted → with_price_tag Err
    .with_scheme(pattern, Eip155Exact)
    .with_price_tag(tag)?              // Result<X402Layer, BuildError>
        .with_settlement_mode(mode)
        .with_resource(url)            // pins ResourceInfo.url; does not waive base_url
        .with_description(s)
        .with_mime_type(s)
        .with_hooks(h)
```

## Send a payment (TARGET)

HTTP buyer retry (`WithPayments`, `X402Client`) is the intended client
surface. Consumer binaries typically return `anyhow::Result`.

```rust,ignore
async fn pay() -> anyhow::Result<reqwest::Response> {
    use alloy_signer_local::PrivateKeySigner;
    use r402::evm::Eip155ExactClient;
    use r402::http::{WithPayments, X402Client};
    use std::sync::Arc;

    let signer = Arc::new("0x...".parse::<PrivateKeySigner>()?);
    let client = reqwest::Client::new().with_payments(
        X402Client::new().register(Eip155ExactClient::new(signer)),
    );
    Ok(client.get("https://api.example.com/paid").send().await?)
}
```

## Settlement modes

`SettlementMode` is re-exported at `r402::http::server` (and crate root).

| Mode | After verify | Receipt | Handler failure | `upto` actual |
| --- | --- | --- | --- | --- |
| **Sequential** (default) | execute, then settle | attached | settle not started | applied |
| **Concurrent** | settle ∥ execute, then join | attached | drop settle, do not await | 402 |
| **Background** | spawn settle, execute, return | not attached | already spawned | strip; charge signed max |

`upto` actual is Sequential only. Concurrent or Background with
upfront/escrow is 500.

## `upto` (TARGET)

EVM `upto` (Permit2 witness: verify = max, settle = actual ≤ max) and
Solana `upto` (escrow) live under each chain crate's `src/upto/`. EVM
`Eip155Upto::price_tag(pay_to, asset)` is two arguments (max amount).
Handlers opt in with `UptoActualAmount` on Sequential responses.

## Wire

| Item | Contract |
| --- | --- |
| Version | `x402Version: 2` |
| 402 challenge | `Payment-Required` (base64, Title-Case) |
| Retry | `Payment-Signature` |
| Receipt | `Payment-Response` |
| JSON | camelCase, `deny_unknown_fields` |

## License

Licensed under either of MIT or Apache-2.0, at your option.
